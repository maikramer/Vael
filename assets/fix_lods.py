#!/usr/bin/env python3
"""Refaz os LOD1/LOD2 do pool que não reduzem o suficiente.

A geração original (`text3d lod` → `gltf-transform simplify`) trata as
costuras UV como fronteiras bloqueadas e pára num "piso de costuras": 106 dos
154 assets ficaram com LOD1 = 50 % e LOD2 = 33 % do LOD0, e alguns (moss_rock,
ore_vein, boss_ogre) com os três tiers iguais. O LOD2 vive a 70–110 m+, onde
um adereço não precisa de mais de ~2 000 triângulos.

Este script usa o `gltfpack` (meshoptimizer) sobre o PRÓPRIO ficheiro de cada
tier — as texturas já reduzidas desse tier ficam como estão; só a geometria
muda. Nomes de nós e materiais (`-kn -km`), extras (`-ke`), skins e animações
são preservados.

NUNCA `-sa` (nem `simplify_sloppy`): a simplificação agressiva solda vértices
de ilhas UV diferentes e os triângulos passam a amostrar outra peça do atlas —
em 2026-10-22 deixou 168 tiers com a textura embaralhada (a casa do pastor,
junto ao portão oeste, era o caso mais visível). Respeitar as costuras quer
dizer que os assets gerados ficam perto do piso; reduzir mais exige refazer o
UV e re-bake da textura, não só simplificar.

Uso:
    python3 assets/fix_lods.py --dry-run          # só lista o que faria
    python3 assets/fix_lods.py                    # aplica (backup primeiro)
    python3 assets/fix_lods.py swamp/moss_rock    # só estes assets

Os originais vão para ~/.cache/vael-lod-backup/<data>/ antes de serem
substituídos.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

MESHES = Path(__file__).resolve().parent / "public" / "assets" / "meshes"

# Alvos por tier, relativos ao LOD0 (com piso/teto absolutos).
LOD1_RATIO, LOD1_MIN = 0.30, 400
LOD2_RATIO, LOD2_MIN, LOD2_MAX = 0.07, 250, 2000
# Só refaz quando o tier atual passa o alvo por mais do que isto.
SLACK = 1.3


def glb_json(path: Path) -> dict:
    data = path.read_bytes()
    length = struct.unpack_from("<I", data, 12)[0]
    return json.loads(data[20 : 20 + length])


def triangles(path: Path) -> int:
    doc = glb_json(path)
    total = 0
    for mesh in doc.get("meshes", []):
        for prim in mesh["primitives"]:
            if "indices" in prim:
                total += doc["accessors"][prim["indices"]]["count"] // 3
            else:
                total += doc["accessors"][prim["attributes"]["POSITION"]]["count"] // 3
    return total


def signature(path: Path) -> tuple:
    """O que não pode mudar: animações, juntas, nomes de nós."""
    doc = glb_json(path)
    anims = sorted(a.get("name", "") for a in doc.get("animations", []))
    joints = [len(s["joints"]) for s in doc.get("skins", [])]
    names = {n.get("name", "") for n in doc.get("nodes", [])}
    return anims, joints, names


def targets(lod0_tris: int) -> dict[int, int]:
    return {
        1: max(LOD1_MIN, int(lod0_tris * LOD1_RATIO)),
        2: max(LOD2_MIN, min(LOD2_MAX, int(lod0_tris * LOD2_RATIO))),
    }


def gltfpack(src: Path, dst: Path, ratio: float) -> str:
    cmd = [
        "npx", "--yes", "gltfpack",
        "-i", str(src), "-o", str(dst),
        "-si", f"{ratio:.5f}",
        "-kn", "-km", "-ke", "-c",
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if r.returncode != 0 or not dst.is_file():
        raise RuntimeError(r.stderr.strip() or r.stdout.strip() or "gltfpack falhou")
    return r.stderr


def plan(only: list[str]) -> list[tuple[Path, int, int, int]]:
    """(ficheiro do tier, tier, triângulos atuais, alvo)."""
    out = []
    for lod0 in sorted(MESHES.rglob("*_lod0.glb")):
        if "_intermediate" in lod0.parts:
            continue  # trabalho do pipeline, não é o que a engine carrega
        base = str(lod0.relative_to(MESHES))[: -len("_lod0.glb")]
        if only and base not in only:
            continue
        n0 = triangles(lod0)
        for tier, target in targets(n0).items():
            path = MESHES / f"{base}_lod{tier}.glb"
            if not path.is_file():
                continue
            n = triangles(path)
            if n > target * SLACK:
                out.append((path, tier, n, target))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("assets", nargs="*", help="bases relativas (ex.: swamp/moss_rock)")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    todo = plan(args.assets)
    print(f"{len(todo)} tiers a refazer")
    if args.dry_run:
        for path, tier, n, target in todo:
            print(f"  {path.relative_to(MESHES)}: {n} → ≤{target}")
        return 0

    backup = Path.home() / ".cache" / "vael-lod-backup" / dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    failed = 0
    with tempfile.TemporaryDirectory(prefix="fix_lods_") as tmp:
        for path, tier, n, target in todo:
            rel = path.relative_to(MESHES)
            out = Path(tmp) / path.name
            try:
                gltfpack(path, out, target / n)
                got = triangles(out)
                before, after = signature(path), signature(out)
                if before[0] != after[0] or before[1] != after[1] or not before[2] <= after[2]:
                    raise RuntimeError("animações/juntas/nós mudaram — não substituído")
                if got >= n:
                    raise RuntimeError(f"não reduziu ({got})")
            except Exception as error:  # noqa: BLE001 — relatório por asset
                failed += 1
                print(f"  FALHOU {rel}: {error}")
                continue
            dest = backup / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, dest)
            shutil.copy2(out, path)
            print(f"  {rel}: {n} → {got} tri ({path.stat().st_size // 1024} KiB)")
    print(f"backup: {backup}" if todo else "nada a fazer")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
