#!/usr/bin/env python3
"""Gera LOD1/LOD2 a partir dos modelos base `*_lod0.glb`.

O repositório versiona (Git LFS) apenas o modelo BASE finalizado:
`*_lod0.glb`, as colisões `*_collision.glb` e os GLB sem ladder
(`vegetation/*.glb`). LOD1/LOD2 são DERIVADOS e não entram no git: este
script reconstrói-os com `text3d lod` + `text3d finish`, de forma
determinística, a partir do base commitado.

Os parâmetros espelham o pipeline do GameAssets (`assets/game.yaml` e
`pipeline.py`, stage 9): ratios 0.40/0.22, textura 2048, rig preservado até
ao LOD2. O LOD0 que o `text3d lod` também escreve é descartado — o base é
a fonte de verdade e nunca é reescrito.

Uso:
  python3 tools/make_lods.py                    # gera só os que faltam
  python3 tools/make_lods.py --force            # regenera todos
  python3 tools/make_lods.py --dry-run          # lista o que faria
  python3 tools/make_lods.py --only 'characters/*' --jobs 8
  python3 tools/make_lods.py --no-finish        # LODs crus (debug)
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROOT = ROOT / "assets" / "public" / "assets" / "meshes"

# Mesmos valores do stage 9 do GameAssets. Mudar aqui muda a ladder de todos
# os assets; não são "gostos" por modelo.
LOD1_RATIO = 0.40
LOD2_RATIO = 0.22
MIN_FACES_LOD1 = 500
MIN_FACES_LOD2 = 150
TEXTURE_SIZE = 2048
RIG_MAX_LEVEL = 2

PRINT_LOCK = threading.Lock()


def log(message: str) -> None:
    with PRINT_LOCK:
        print(message, flush=True)


def find_text3d(explicit: str | None) -> str:
    if explicit:
        return explicit
    env = os.environ.get("TEXT3D")
    if env:
        return env
    found = shutil.which("text3d")
    if found:
        return found
    fallback = Path.home() / "GitClones" / "AiGameKit" / "Text3D" / ".venv" / "bin" / "text3d"
    if fallback.is_file():
        return str(fallback)
    raise SystemExit(
        "text3d não encontrado. Instala o AiGameKit ou passa --text3d / TEXT3D=…"
    )


@dataclass(frozen=True)
class GlbInfo:
    faces: int
    skins: int
    animations: int
    images: int

    @property
    def rigged(self) -> bool:
        return self.skins > 0

    @property
    def animated(self) -> bool:
        return self.animations > 0


def glb_info(path: Path) -> GlbInfo:
    """Lê o chunk JSON do GLB (sem decodificar binários nem meshopt)."""
    with path.open("rb") as handle:
        header = handle.read(12)
        if len(header) != 12:
            raise ValueError("GLB truncado")
        magic, version, _length = struct.unpack("<III", header)
        if magic != 0x46546C67 or version != 2:
            raise ValueError("não é um GLB 2.0")
        chunk_len, chunk_type = struct.unpack("<II", handle.read(8))
        if chunk_type != 0x4E4F534A:
            raise ValueError("primeiro chunk não é JSON")
        doc = json.loads(handle.read(chunk_len))

    accessors = doc.get("accessors") or []
    faces = 0
    for mesh in doc.get("meshes") or []:
        for primitive in mesh.get("primitives") or []:
            indices = primitive.get("indices")
            if indices is not None:
                faces += int(accessors[indices].get("count", 0)) // 3
                continue
            position = (primitive.get("attributes") or {}).get("POSITION")
            if position is not None:
                faces += int(accessors[position].get("count", 0)) // 3

    return GlbInfo(
        faces=faces,
        skins=len(doc.get("skins") or []),
        animations=len(doc.get("animations") or []),
        images=len(doc.get("images") or []),
    )


@dataclass(frozen=True)
class Job:
    lod0: Path
    base: str
    lod1: Path
    lod2: Path


def discover(root: Path, only: list[str]) -> list[Job]:
    jobs: list[Job] = []
    for lod0 in sorted(root.rglob("*_lod0.glb")):
        if "_intermediate" in lod0.parts:
            continue
        rel = lod0.relative_to(root).as_posix()
        if only and not any(fnmatch.fnmatch(rel, pattern) for pattern in only):
            continue
        base = lod0.name[: -len("_lod0.glb")]
        jobs.append(
            Job(
                lod0=lod0,
                base=base,
                lod1=lod0.with_name(f"{base}_lod1.glb"),
                lod2=lod0.with_name(f"{base}_lod2.glb"),
            )
        )
    return jobs


def is_current(job: Job) -> bool:
    if not (job.lod1.is_file() and job.lod2.is_file()):
        return False
    source = job.lod0.stat().st_mtime
    return all(
        path.stat().st_size > 64 and path.stat().st_mtime >= source
        for path in (job.lod1, job.lod2)
    )


def run(cmd: list[str], log_path: Path) -> None:
    with log_path.open("wb") as output:
        result = subprocess.run(cmd, stdout=output, stderr=subprocess.STDOUT)
    if result.returncode != 0:
        tail = log_path.read_text(errors="replace").strip().splitlines()[-25:]
        raise RuntimeError(
            f"`{' '.join(cmd)}` falhou (exit {result.returncode})\n"
            + "\n".join(tail)
        )


def install(source: Path, destination: Path) -> None:
    """Copia e troca atomicamente; nunca deixa um LOD meio escrito."""
    temporary = destination.with_name(destination.name + f".tmp{os.getpid()}")
    shutil.copy2(source, temporary)
    os.replace(temporary, destination)


def generate(
    job: Job,
    *,
    text3d: str,
    tmp_root: Path,
    finish: bool,
    rig_max_level: int,
    texture_size: int,
    lod1_ratio: float,
    lod2_ratio: float,
    min_faces_lod1: int,
    min_faces_lod2: int,
) -> str:
    tmp = Path(tempfile.mkdtemp(prefix=f"{job.base}-", dir=tmp_root))
    try:
        log_path = tmp / "text3d.log"
        run(
            [
                text3d,
                "lod",
                str(job.lod0),
                "-o",
                str(tmp),
                "--basename",
                job.base,
                "--lod1-ratio",
                str(lod1_ratio),
                "--lod2-ratio",
                str(lod2_ratio),
                "--min-faces-lod1",
                str(min_faces_lod1),
                "--min-faces-lod2",
                str(min_faces_lod2),
                "--no-meshopt",
                "--texture-size",
                str(texture_size),
                "--rig-max-level",
                str(rig_max_level),
            ],
            log_path,
        )

        generated = {
            1: tmp / f"{job.base}_lod1.glb",
            2: tmp / f"{job.base}_lod2.glb",
        }
        for level, path in generated.items():
            if not path.is_file() or path.stat().st_size <= 64:
                raise RuntimeError(f"text3d lod não escreveu o LOD{level} ({path.name})")

        if finish:
            for level in (1, 2):
                run([text3d, "finish", str(generated[level])], log_path)

        base_info = glb_info(job.lod0)
        levels = {level: glb_info(path) for level, path in generated.items()}

        if levels[1].faces <= 0 or levels[2].faces <= 0:
            raise RuntimeError("LOD gerado sem geometria")
        if levels[1].faces >= base_info.faces:
            raise RuntimeError(
                f"LOD1 ({levels[1].faces}) não é mais leve que o base ({base_info.faces})"
            )
        if levels[2].faces > levels[1].faces:
            raise RuntimeError(
                f"LOD2 ({levels[2].faces}) tem mais faces que o LOD1 ({levels[1].faces})"
            )
        for level, info in levels.items():
            if base_info.rigged and level <= rig_max_level and not info.rigged:
                raise RuntimeError(f"LOD{level} perdeu o skin do base")
            if base_info.animated and level <= rig_max_level and not info.animated:
                raise RuntimeError(f"LOD{level} perdeu as animações do base")

        install(generated[1], job.lod1)
        install(generated[2], job.lod2)

        return (
            f"{job.lod0.relative_to(ROOT)}: "
            f"{base_info.faces} → {levels[1].faces} → {levels[2].faces} faces"
        )
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--text3d", default=None, help="binário text3d (default: PATH/TEXT3D)")
    parser.add_argument("--jobs", type=int, default=min(8, os.cpu_count() or 1))
    parser.add_argument("--force", action="store_true", help="regenera mesmo os LODs atuais")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-finish", action="store_true", help="LODs crus, sem KTX2/meshopt")
    parser.add_argument(
        "--only",
        action="append",
        default=[],
        metavar="GLOB",
        help="filtra caminhos relativos ao root (repetível)",
    )
    parser.add_argument("--lod1-ratio", type=float, default=LOD1_RATIO)
    parser.add_argument("--lod2-ratio", type=float, default=LOD2_RATIO)
    parser.add_argument("--min-faces-lod1", type=int, default=MIN_FACES_LOD1)
    parser.add_argument("--min-faces-lod2", type=int, default=MIN_FACES_LOD2)
    parser.add_argument("--texture-size", type=int, default=TEXTURE_SIZE)
    parser.add_argument("--rig-max-level", type=int, default=RIG_MAX_LEVEL, choices=(0, 1, 2))
    args = parser.parse_args()

    if not args.root.is_dir():
        parser.error(f"root inexistente: {args.root}")
    if not 0 < args.lod2_ratio < args.lod1_ratio <= 1:
        parser.error("esperado 0 < --lod2-ratio < --lod1-ratio <= 1")

    jobs = discover(args.root, args.only)
    missing = [job for job in jobs if args.force or not is_current(job)]
    if not jobs:
        print("nenhum *_lod0.glb encontrado.", file=sys.stderr)
        return 1

    print(
        f"{len(jobs)} modelos base; {len(missing)} sem LOD atual "
        f"(jobs={args.jobs}, finish={'não' if args.no_finish else 'sim'})"
    )
    if args.dry_run:
        for job in missing:
            print(f"  gerar {job.lod0.relative_to(args.root)}")
        return 0
    if not missing:
        return 0

    text3d = find_text3d(args.text3d)
    tmp_root = args.root / "_intermediate" / "lods"
    tmp_root.mkdir(parents=True, exist_ok=True)

    failures: list[str] = []
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = {
            pool.submit(
                generate,
                job,
                text3d=text3d,
                tmp_root=tmp_root,
                finish=not args.no_finish,
                rig_max_level=args.rig_max_level,
                texture_size=args.texture_size,
                lod1_ratio=args.lod1_ratio,
                lod2_ratio=args.lod2_ratio,
                min_faces_lod1=args.min_faces_lod1,
                min_faces_lod2=args.min_faces_lod2,
            ): job
            for job in missing
        }
        for done, future in enumerate(as_completed(futures), start=1):
            job = futures[future]
            try:
                log(f"[{done}/{len(missing)}] {future.result()}")
            except Exception as error:  # noqa: BLE001 - relatório de lote
                failures.append(f"{job.lod0.relative_to(args.root)}: {error}")
                log(f"[{done}/{len(missing)}] FALHOU {job.lod0.relative_to(args.root)}")

    if failures:
        print(f"\n{len(failures)} falha(s):", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(f"\n{len(missing)} ladders geradas.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
