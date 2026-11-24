#!/usr/bin/env python3
"""Gera os materiais de madeira e corda das cercas procedurais.

Lê `assets/manifests/wood-textures.yaml` e, por conjunto, corre o pipeline do
pool (`assets/public/assets/textures/SOURCES.md`):

1. `texture2d generate` — albedo seamless (SD1.5 + circular padding). Os
   portões de qualidade são os da ferramenta e cada reprovação volta a gerar
   com a seed seguinte (até `attempts`): `--max-isolated-detail` (um nó ou
   mancha que o tiling repetiria em cada célula), `--max-banding` (tábuas com
   frestas, veio deitado), `--max-tail-ratio` (flocos sobre um fundo liso).
   `--axis v` roda o que sair com o veio deitado e `--match-color` gradua a
   cor média à da paleta — o SD1.5 sai sempre castanho a meia luminância.
   Se o decode cair em tiles (o que deixa manchas) e um portão reprovar,
   repete com `--no-vae-tiling`;
2. `materialize` — height/normal/ao/smoothness derivados do albedo (preset
   `wood`);
3. Pillow/numpy — normal atenuada, roughness = 1 − smoothness remapeada para a
   faixa mate do ART.md, e o `orm` do glTF (R = oclusão, G = roughness,
   B = metálico a zero) que o `StandardMaterial` do Bevy espera, em WebP;
4. `ktx create` — KTX2 UASTC com cadeia de mipmaps, como o resto do pool.

Cada conjunto sai em `assets/public/assets/textures/<id>/` (LFS) com
`gen.json`. As folhas de verificação (tiling 3×3 + mapas) ficam no directório
de trabalho (`/tmp/vael-wood-textures` por defeito): as bordas de cada célula
são a costura — tem de passar sem se ver.

A superfície é contínua de propósito: as tábuas são geometria (`bevy_ext_fence`).

Uso:
    python3 tools/gen_fence_textures.py                 # gera o que faltar
    python3 tools/gen_fence_textures.py --only wood_oak_hewn --force
    python3 tools/gen_fence_textures.py --dry-run
    python3 tools/gen_fence_textures.py --sheet-only    # só as folhas
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import numpy as np
import yaml
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "assets" / "manifests" / "wood-textures.yaml"
POOL = ROOT / "assets" / "public" / "assets" / "textures"
AIGAMEKIT = ROOT.parent / "AiGameKit"
DEFAULT_WORK = Path("/tmp/vael-wood-textures")

# Fechamento comum do prompt: as regras de arte (ART.md §8) sem palavras que
# peçam uma ilustração em vez de uma superfície. O veio corre de cima a baixo.
STYLE_SUFFIX = (
    "wood grain texture, broad clean color zones, earthy mineral pigments, "
    "matte tempera finish, soft even lighting, flat top-down view, "
    "texture fills the whole frame, stylized game texture"
)

MAPS = ("albedo", "normal", "orm", "ao", "roughness", "height")


def log(message: str) -> None:
    print(message, flush=True)


def run(cmd: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
    log(f"  $ {' '.join(str(part) for part in cmd[:6])}{' …' if len(cmd) > 6 else ''}")
    return subprocess.run(cmd, text=True, capture_output=True, **kwargs)


def texture2d_cmd() -> list[str]:
    """Prefixo do `texture2d`: o wrapper do PATH ou o venv do Text2D.

    O `~/.local/bin/texture2d` aponta para `Texture2D/.venv`, que pode não
    existir; o pacote corre com qualquer venv do monorepo que tenha o diffusers.
    """
    wrapper = shutil.which("texture2d")
    if wrapper:
        probe = subprocess.run([wrapper, "info"], capture_output=True, text=True)
        if probe.returncode == 0:
            return [wrapper]
    candidates = sorted(AIGAMEKIT.glob("*/.venv/bin/python"))
    for python in candidates:
        env = dict(os.environ)
        env["PYTHONPATH"] = os.pathsep.join(
            str(AIGAMEKIT / part) for part in ("Texture2D/src", "Shared/src")
        )
        probe = subprocess.run(
            [str(python), "-c", "import texture2d, diffusers"], env=env, capture_output=True, text=True
        )
        if probe.returncode == 0:
            log(f"texture2d via {python} (PYTHONPATH; o wrapper do PATH não funciona)")
            return [str(python), "-m", "texture2d"]
    raise SystemExit("texture2d indisponível: ./install.sh texture2d no AiGameKit")


def tool_env() -> dict[str, str]:
    """`PYTHONPATH` para o `-m texture2d` (idempotente com o wrapper)."""
    env = dict(os.environ)
    extra = os.pathsep.join(
        str(AIGAMEKIT / part) for part in ("Texture2D/src", "Shared/src")
    )
    env["PYTHONPATH"] = extra + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    return env


def require(*names: str) -> None:
    missing = [name for name in names if shutil.which(name) is None]
    if missing:
        raise SystemExit(f"falta no PATH: {', '.join(missing)}")


def build_prompt(spec: dict[str, Any]) -> str:
    # Um conjunto pode trazer o seu fecho (`suffix:`) quando o comum lhe fica mal.
    suffix = (spec.get("suffix") or STYLE_SUFFIX).strip()
    return f"seamless tileable texture, {spec['subject'].strip()}, {suffix}"


def build_negative(manifest: dict[str, Any], spec: dict[str, Any]) -> str:
    words = list(manifest.get("negative") or []) + list(spec.get("negative") or [])
    return ", ".join(words)


# ------------------------------------------------------------------- imagens


def load_l(path: Path) -> np.ndarray:
    return np.asarray(Image.open(path).convert("L"), dtype=np.float32) / 255.0


def load_rgb(path: Path) -> np.ndarray:
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0


def save_webp(array: np.ndarray, path: Path, quality: int) -> None:
    if array.ndim == 2:
        image = Image.fromarray(np.clip(array * 255.0 + 0.5, 0, 255).astype(np.uint8), "L")
    else:
        image = Image.fromarray(np.clip(array * 255.0 + 0.5, 0, 255).astype(np.uint8), "RGB")
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path, "WEBP", quality=quality, method=6)


def ktx2(array: np.ndarray, path: Path, srgb: bool, work: Path) -> None:
    """KTX2 UASTC com mipmaps, como o resto do pool (1 byte/texel)."""
    png = work / f"{path.stem}.png"
    if array.ndim == 2:
        image = Image.fromarray(np.clip(array * 255.0 + 0.5, 0, 255).astype(np.uint8), "L").convert("RGBA")
    else:
        image = Image.fromarray(np.clip(array * 255.0 + 0.5, 0, 255).astype(np.uint8), "RGB").convert("RGBA")
    image.save(png)
    fmt = "R8G8B8A8_SRGB" if srgb else "R8G8B8A8_UNORM"
    done = run(
        [
            "ktx", "create", "--format", fmt, "--encode", "uastc",
            "--generate-mipmap", "--assign-tf", "srgb" if srgb else "linear",
            str(png), str(path),
        ]
    )
    if done.returncode != 0:
        raise SystemExit(f"ktx create falhou em {path.name}:\n{done.stderr.strip()}")
    check = subprocess.run(["ktx2check", "-q", str(path)], capture_output=True, text=True)
    if check.returncode != 0:
        raise SystemExit(f"ktx2check reprovou {path}:\n{check.stdout}{check.stderr}")


# ------------------------------------------------------------------- conjuntos


def generate_albedo(
    spec: dict[str, Any], manifest: dict[str, Any], t2: list[str], src: Path, dry: bool
) -> dict[str, Any]:
    """Corre o `texture2d` com os portões de qualidade e devolve o sidecar dele
    (params: seed final, tokens, tileabilidade, portões, eixo e cor)."""
    prompt = build_prompt(spec)
    negative = build_negative(manifest, spec)
    size = int(manifest["size"])
    out = src / "albedo.png"
    cmd = [
        *t2, "generate", prompt,
        "-o", str(out),
        "--quality", manifest["quality"],
        "-W", str(size), "-H", str(size),
        "--seed", str(spec["seed"]),
        "-n", negative,
        "--max-isolated-detail", str(manifest["max_isolated_detail"]),
        "--attempts", str(manifest["attempts"]),
    ]
    if manifest.get("max_banding") is not None:
        cmd += ["--max-banding", str(manifest["max_banding"])]
    if manifest.get("max_tail_ratio") is not None:
        cmd += ["--max-tail-ratio", str(manifest["max_tail_ratio"])]
    axis = str(spec.get("axis", manifest.get("axis") or "none")).lower()
    if axis != "none":
        cmd += ["--axis", axis]
    if spec.get("match"):
        cmd += ["--match-color", str(spec["match"])]
    if manifest.get("refine_steps"):
        cmd += ["--refine-steps", str(int(manifest["refine_steps"]))]
    if manifest.get("preset") and manifest["preset"].lower() != "none":
        cmd += ["-p", manifest["preset"]]
    if dry:
        log("  $ " + " ".join(cmd[1:]))
        return {}

    def invoke(extra: list[str]) -> dict[str, Any]:
        done = run([*cmd, *extra], env=tool_env())
        sidecar = src / "albedo.json"
        if done.returncode != 0 or not out.exists() or not sidecar.exists():
            tail = (done.stdout or "")[-1500:] + (done.stderr or "")[-1500:]
            raise SystemExit(f"texture2d falhou em {spec['id']}:\n{tail}")
        return dict(json.loads(sidecar.read_text()).get("params") or {})

    meta = invoke([])
    gates = meta.get("quality_gates") or meta.get("isolated_detail_gate") or {}
    if meta.get("vae_tiled_decode") and gates.get("passed") is False:
        # O decode em tiles deixa manchas (2026-11-23, 0,40 contra 0,08 no
        # integral): nenhuma seed as evita. Repetir com o decode integral.
        log("  decode em tiles e portão reprovado — a repetir com decode integral")
        meta = invoke(["--no-vae-tiling"])
    return meta


def derive_maps(spec: dict[str, Any], manifest: dict[str, Any], src: Path, maps: Path, dry: bool) -> None:
    if dry:
        log(f"  $ materialize {src / 'albedo.png'} -p {manifest['materialize_preset']} -o {maps}")
        return
    done = run(
        ["materialize", str(src / "albedo.png"), "-p", manifest["materialize_preset"], "-o", str(maps), "--quiet"]
    )
    if done.returncode != 0 or not (maps / "albedo_normal.png").exists():
        raise SystemExit(f"materialize falhou em {spec['id']}:\n{done.stdout}\n{done.stderr}")


def process(spec: dict[str, Any], manifest: dict[str, Any], src: Path, maps: Path, out: Path, work: Path) -> None:
    """Normal atenuada, roughness mate e os ficheiros do pool."""
    size = int(manifest["size"])
    albedo = Image.open(src / "albedo.png").convert("RGB")
    if albedo.size != (size, size):
        albedo = albedo.resize((size, size), Image.LANCZOS)

    strength = float(spec.get("normal_strength", 1.0))
    normal = load_rgb(maps / "albedo_normal.png") * 2.0 - 1.0
    normal = (normal - np.array([0.0, 0.0, 1.0], dtype=np.float32)) * strength
    normal += np.array([0.0, 0.0, 1.0], dtype=np.float32)
    normal /= np.maximum(np.linalg.norm(normal, axis=2, keepdims=True), 1e-5)

    lo, hi = float(manifest["roughness_min"]), float(manifest["roughness_max"])
    smooth = load_l(maps / "albedo_smoothness.png")
    roughness = lo + (hi - lo) * (1.0 - smooth)
    ao = load_l(maps / "albedo_ao.png")
    height = load_l(maps / "albedo_height.png")
    # ORM do glTF: é esta imagem que o `StandardMaterial` do Bevy lê — a
    # roughness vai no verde e o azul (metálico) tem de ficar a zero, senão a
    # madeira parece metal.
    orm = np.stack([ao, roughness, np.zeros_like(ao)], axis=2)

    out.mkdir(parents=True, exist_ok=True)
    save_webp(np.asarray(albedo, dtype=np.float32) / 255.0, out / "albedo.webp", 92)
    save_webp(normal * 0.5 + 0.5, out / "normal.webp", 95)
    save_webp(orm, out / "orm.webp", 95)
    save_webp(ao, out / "ao.webp", 95)
    save_webp(roughness, out / "roughness.webp", 95)
    save_webp(height, out / "height.webp", 95)

    ktx2(np.asarray(albedo, dtype=np.float32) / 255.0, out / "albedo.ktx2", True, work)
    ktx2(normal * 0.5 + 0.5, out / "normal.ktx2", False, work)
    ktx2(orm, out / "orm.ktx2", False, work)
    ktx2(ao, out / "ao.ktx2", False, work)
    ktx2(roughness, out / "roughness.ktx2", False, work)
    ktx2(height, out / "height.ktx2", False, work)


def write_gen(spec: dict[str, Any], manifest: dict[str, Any], meta: dict[str, Any], out: Path) -> dict[str, Any]:
    detail = meta.get("isolated_detail") or {}
    banding = meta.get("banding") or {}
    tail = meta.get("tail_ratio") or {}
    record = {
        "prompt": meta.get("prompt_final") or build_prompt(spec),
        "negative_prompt": meta.get("negative_prompt") or build_negative(manifest, spec),
        "width": int(manifest["size"]),
        "height": int(manifest["size"]),
        "steps": meta.get("num_inference_steps"),
        "guidance": meta.get("guidance_scale"),
        "refine_steps": meta.get("refine_steps"),
        "seed": meta.get("seed", spec["seed"]),
        "seed_requested": spec["seed"],
        "prompt_tokens": meta.get("prompt_tokens"),
        "negative_prompt_tokens": meta.get("negative_prompt_tokens"),
        "tileability": round(float((meta.get("tileability") or {}).get("score", 0.0)), 4),
        "isolated_detail": round(float(detail.get("spot", 0.0)), 4),
        "isolated_detail_gate": meta.get("isolated_detail_gate"),
        "banding": round(max(float(banding.get("rows", 0.0)), float(banding.get("cols", 0.0))), 4),
        "tail_ratio": round(max(float(tail.get("high", 0.0)), float(tail.get("low", 0.0))), 4),
        "quality_gates": meta.get("quality_gates"),
        "axis": meta.get("axis"),
        "match_color": meta.get("match_color"),
        "vae_tiled_decode": meta.get("vae_tiled_decode"),
        "normal_strength": float(spec.get("normal_strength", 1.0)),
        "roughness_range": [float(manifest["roughness_min"]), float(manifest["roughness_max"])],
        "materialize_preset": manifest["materialize_preset"],
        "biome": spec.get("biome"),
        "palette": spec.get("palette", []),
        "regenerated_at": dt.datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
    }
    (out / "gen.json").write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n")
    return record


# ------------------------------------------------------------------- folhas


def preview(out: Path, path: Path) -> Image.Image | None:
    """Albedo em tiling 3×3 (512 px) + normal, ao e roughness (256 px cada)."""
    albedo = out / "albedo.webp"
    if not albedo.exists():
        return None
    tile = Image.open(albedo).convert("RGB")
    tiled = Image.new("RGB", (tile.width * 3, tile.height * 3))
    for row in range(3):
        for column in range(3):
            tiled.paste(tile, (column * tile.width, row * tile.height))
    tiled = tiled.resize((512, 512), Image.LANCZOS)
    sheet = Image.new("RGB", (512 + 3 * 256, 512), (24, 24, 24))
    sheet.paste(tiled, (0, 0))
    for index, name in enumerate(("normal", "ao", "roughness")):
        source = out / f"{name}.webp"
        if source.exists():
            sheet.paste(Image.open(source).convert("RGB").resize((256, 256), Image.LANCZOS), (512 + index * 256, 0))
    path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(path)
    return sheet


def sheet(sets: list[dict[str, Any]], work: Path) -> Path:
    """Os conjuntos lado a lado, cada um em tiling 3×3."""
    cells: list[tuple[str, Image.Image]] = []
    for spec in sets:
        source = POOL / spec["id"] / "albedo.webp"
        if source.exists():
            tile = Image.open(source).convert("RGB")
            tiled = Image.new("RGB", (tile.width * 3, tile.height * 3))
            for row in range(3):
                for column in range(3):
                    tiled.paste(tile, (column * tile.width, row * tile.height))
            cells.append((spec["id"], tiled.resize((512, 512), Image.LANCZOS)))
    if not cells:
        raise SystemExit("nenhum albedo no pool — gera primeiro")
    columns = 3
    rows = (len(cells) + columns - 1) // columns
    page = Image.new("RGB", (columns * 512, rows * 512), (24, 24, 24))
    for index, (_, cell) in enumerate(cells):
        page.paste(cell, ((index % columns) * 512, (index // columns) * 512))
    path = work / "sheet_albedo.png"
    path.parent.mkdir(parents=True, exist_ok=True)
    page.save(path)
    return path


# ---------------------------------------------------------------------- main


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--only", help="só estes conjuntos (ids separados por vírgulas)")
    parser.add_argument("--force", action="store_true", help="regenera conjuntos já completos")
    parser.add_argument("--dry-run", action="store_true", help="só lista os comandos")
    parser.add_argument("--sheet-only", action="store_true", help="só refaz as folhas de verificação")
    parser.add_argument("--work", type=Path, default=DEFAULT_WORK, help="directório de trabalho")
    args = parser.parse_args()

    manifest = yaml.safe_load(MANIFEST.read_text())
    sets: list[dict[str, Any]] = list(manifest["sets"])
    if args.only:
        wanted = {name.strip() for name in args.only.split(",")}
        sets = [spec for spec in sets if spec["id"] in wanted]

    if args.sheet_only:
        log(f"folha: {sheet(sets, args.work)}")
        return 0

    require("materialize", "ktx", "ktx2check")
    t2 = texture2d_cmd()
    minimum = float(manifest.get("min_tileability", 0.85))
    limits = {
        "tileabilidade": ("tileability", minimum, "below"),
        "pormenor isolado": ("isolated_detail", float(manifest.get("max_isolated_detail", 0.12)), "above"),
        "bandas": ("banding", manifest.get("max_banding"), "above"),
        "cauda": ("tail_ratio", manifest.get("max_tail_ratio"), "above"),
    }
    report: list[tuple[str, dict[str, Any], int]] = []

    for spec in sets:
        name = spec["id"]
        out = POOL / name
        complete = all((out / f"{map_name}.ktx2").exists() for map_name in MAPS) and (out / "gen.json").exists()
        if complete and not args.force:
            log(f"[{name}] já completo (--force para refazer)")
            report.append((name, json.loads((out / "gen.json").read_text()), 0))
            continue
        log(f"[{name}] a gerar")
        work = args.work / name
        src, maps = work / "src", work / "maps"
        for folder in (src, maps):
            folder.mkdir(parents=True, exist_ok=True)
        for stale in src.glob("*.json"):
            stale.unlink()
        meta = generate_albedo(spec, manifest, t2, src, args.dry_run)
        if args.dry_run:
            derive_maps(spec, manifest, src, maps, True)
            continue
        derive_maps(spec, manifest, src, maps, False)
        process(spec, manifest, src, maps, out, work)
        record = write_gen(spec, manifest, meta, out)
        preview(out, args.work / f"preview_{name}.png")
        problems = []
        for label, (key, limit, sense) in limits.items():
            if limit is None:
                continue
            value = float(record.get(key) or 0.0)
            if (sense == "below" and value < limit) or (sense == "above" and value > limit):
                problems.append(f"{label} {value:.3f}")
        if problems:
            log(f"[{name}] REPROVA ({'; '.join(problems)}) — a melhor de {meta.get('attempts', 1)} seeds; "
                f"probe outra seed no manifesto")
        else:
            log(
                f"[{name}] seed {record['seed']}, tileabilidade {record['tileability']:.3f}, "
                f"pormenor isolado {record['isolated_detail']:.3f}, bandas {record['banding']:.3f}, "
                f"cauda {record['tail_ratio']:.2f}; "
                f"folha em {args.work / f'preview_{name}.png'}"
            )
        report.append((name, record, 1 if problems else 0))

    if args.dry_run:
        return 0
    log("")
    log(f"{'conjunto':<20} {'seed':>5} {'tileab.':>8} {'destaque':>9} {'bandas':>8} {'cauda':>6}  prompt")
    for name, record, failed in report:
        log(
            f"{name:<20} {record.get('seed', '?'):>5} {record.get('tileability', 0):>8.3f} "
            f"{record.get('isolated_detail', 0):>9.3f} {record.get('banding', 0):>8.3f} "
            f"{record.get('tail_ratio', 0):>6.2f}  {record.get('prompt_tokens', '?')} tokens"
            f"{'  REPROVA' if failed else ''}"
        )
    log(f"folha: {sheet(sets, args.work)}")
    return 1 if any(failed for _, _, failed in report) else 0


if __name__ == "__main__":
    sys.exit(main())
