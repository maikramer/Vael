#!/usr/bin/env python3
"""Gera `ui/vael_map.png` a partir de `world.data/terrain.ahgt`.

O mapa não é uma captura da engine: é um hillshade do heightmap real, com
paleta hipsométrica e um véu de bioma. Corre offline (sem GPU) e é
reproduzível — o mesmo `.ahgt` dá o mesmo PNG.

Uso: python3 tools/make_map.py [--size 1200]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import struct
import zlib
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from water_carve_input import PROP

ROOT = Path(__file__).resolve().parents[1]
AHGT = ROOT / "world.data" / "terrain.ahgt"
OUT = ROOT / "ui" / "vael_map.png"

AHGT_MAGIC = 0x54474841


def load_ahgt(path: Path) -> tuple[np.ndarray, int, int, dict]:
    raw = path.read_bytes()
    if len(raw) < 20:
        raise SystemExit("AHGT truncado")
    magic = struct.unpack_from("<I", raw, 0)[0]
    version = struct.unpack_from("<H", raw, 4)[0]
    width = struct.unpack_from("<H", raw, 6)[0]
    depth = struct.unpack_from("<H", raw, 8)[0]
    meta_len = struct.unpack_from("<I", raw, 16)[0]
    if magic != AHGT_MAGIC or version != 1:
        raise SystemExit(f"AHGT inválido: magic=0x{magic:08x} version={version}")
    if 20 + meta_len > len(raw):
        # Um `meta_len` corrompido dava um JSONDecodeError de uma fatia truncada.
        raise SystemExit(f"AHGT truncado: metadata pede {meta_len} B, só há {len(raw) - 20}")
    meta = json.loads(raw[20 : 20 + meta_len])
    max_height = float(meta["maxHeight"])
    payload = raw[20 + meta_len :]
    try:
        heights = np.frombuffer(zlib.decompress(payload), dtype="<u2")
    except zlib.error:
        heights = np.frombuffer(zlib.decompress(payload, -15), dtype="<u2")
    if heights.size < width * depth:
        raise SystemExit("payload AHGT curto")
    grid = heights[: width * depth].reshape(depth, width).astype(np.float32)
    # O AHGT guarda 0..65535; o meta diz quantos metros isso são. Sem esta
    # escala o mapa inteiro caía no topo da paleta (tudo branco).
    grid *= max_height / 65535.0
    return grid, width, depth, meta


def hypsometric(norm: np.ndarray, slope: np.ndarray) -> np.ndarray:
    """Paleta por altitude + encostas rochosas."""
    stops = np.array(
        [
            [0.00, 0.20, 0.26, 0.18],  # água baixa / várzea
            [0.18, 0.32, 0.40, 0.22],
            [0.38, 0.48, 0.42, 0.25],
            [0.58, 0.55, 0.47, 0.35],
            [0.76, 0.52, 0.50, 0.48],
            [0.90, 0.78, 0.80, 0.82],
            [1.00, 0.94, 0.96, 0.98],
        ]
    )
    out = np.zeros((*norm.shape, 3), dtype=np.float32)
    for i in range(len(stops) - 1):
        a, b = stops[i], stops[i + 1]
        t = np.clip((norm - a[0]) / max(b[0] - a[0], 1e-6), 0.0, 1.0)
        mask = (norm >= a[0]) & (norm <= b[0])
        out[mask] = a[1:] + (b[1:] - a[1:]) * t[mask, None]
    # Encostas íngremes puxam para pedra.
    rock = np.clip(slope * 2.4, 0.0, 0.65)[..., None]
    stone = np.array([0.42, 0.40, 0.38], dtype=np.float32)
    return out * (1.0 - rock) + stone * rock


def hillshade(gx: np.ndarray, gy: np.ndarray) -> np.ndarray:
    nx, ny = -gx, -gy
    nz = np.ones_like(gx)
    length = np.sqrt(nx * nx + ny * ny + nz * nz)
    nx, ny, nz = nx / length, ny / length, nz / length
    # Luz de noroeste, alta.
    lx, ly, lz = -0.55, -0.55, 0.62
    ll = np.sqrt(lx * lx + ly * ly + lz * lz)
    lx, ly, lz = lx / ll, ly / ll, lz / ll
    light = np.clip(nx * lx + ny * ly + nz * lz, 0.0, 1.0)
    return 0.45 + 0.75 * light


def draw_water(rgb: np.ndarray, height: np.ndarray, axis: np.ndarray) -> None:
    x, z = axis[None, :], axis[:, None]
    body = re.compile(r'tag: "(Ocean|Lake)",\s*attrs: \[(.*?)\n\s*\]', re.S)
    for path in sorted((ROOT / "world").rglob("*.bsn")):
        for kind, fields in body.findall(path.read_text()):
            attrs = dict(PROP.findall(fields))
            if "water-level" not in attrs:
                continue
            depth = float(attrs["water-level"]) - height
            if kind == "Ocean":
                x0, z0 = map(float, attrs["min"].split())
                x1, z1 = map(float, attrs["max"].split())
                wet = (x >= x0) & (x <= x1) & (z >= z0) & (z <= z1) & (depth > .03)
            else:
                cx, cz = map(float, attrs["at"].split())
                reach = float(attrs["radius"]) * 1.45
                wet = (np.hypot(x - cx, z - cz) <= reach) & (depth > .03)
            deep = np.clip(depth[wet] / 17, 0.0, 1.0)[:, None]
            shore = np.array([.27, .51, .53])
            open_water = np.array([.10, .25, .34])
            rgb[wet] = shore * (1 - deep) + open_water * deep


def draw_routes(image: Image.Image, size_m: float) -> None:
    path = ROOT / "world/expansion/routes.bsn"
    if not path.exists():
        return
    scale = image.width / size_m
    draw = ImageDraw.Draw(image, "RGBA")
    authored = (value for key, value in PROP.findall(path.read_text()) if key == "path")
    for value in authored:
        coords = [round((float(coord) + size_m * .5) * scale) for coord in value.split()]
        points = list(zip(coords[::2], coords[1::2]))
        draw.line(points, fill=(25, 16, 9, 95), width=max(2, image.width // 640 + 1))
        draw.line(points, fill=(202, 158, 102, 164), width=max(1, image.width // 880))


def draw_rivers(image: Image.Image, size_m: float) -> None:
    from water_carve_input import NODE

    scale = image.width / size_m
    draw = ImageDraw.Draw(image, "RGBA")
    for path in sorted((ROOT / "world/landmarks").glob("*.bsn")):
        for kind, fields in NODE.findall(path.read_text()):
            if kind != "River":
                continue
            attrs = dict(PROP.findall(fields))
            coords = [(float(coord) + size_m * .5) * scale for coord in attrs["path"].split()]
            draw.line(list(zip(coords[::2], coords[1::2])),
                      fill=(53, 114, 132, 225),
                      width=max(1, round(float(attrs.get("width", 6)) * scale)),
                      joint="curve")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--size", type=int, default=1200)
    args = parser.parse_args()

    grid, _, _, meta = load_ahgt(AHGT)
    world_size = float(meta["worldSize"])
    # Sombrear à resolução de saída evita vários arrays RGB de 8192².
    raster = Image.fromarray(grid).resize((args.size, args.size), Image.Resampling.BOX)
    del grid
    height = np.asarray(raster)
    norm = np.clip(height / max(float(meta["maxHeight"]), 1.0), 0.0, 1.0)
    meters = world_size / args.size
    gy, gx = np.gradient(height, meters)
    slope = np.sqrt(gx * gx + gy * gy)

    rgb = hypsometric(norm, slope) * hillshade(gx, gy)[..., None]
    draw_water(rgb, height, (np.arange(args.size) + .5) * meters - world_size * .5)
    rgb = np.clip(rgb, 0.0, 1.0)
    image = Image.fromarray((rgb * 255).astype(np.uint8), "RGB")
    image = image.filter(ImageFilter.GaussianBlur(radius=0.6))
    draw_rivers(image, world_size)
    draw_routes(image, world_size)

    # Vinheta suave e moldura de pergaminho.
    vignette = Image.new("L", image.size, 0)
    draw = ImageDraw.Draw(vignette)
    draw.ellipse((-args.size * 0.15, -args.size * 0.15, args.size * 1.15, args.size * 1.15), fill=255)
    vignette = vignette.filter(ImageFilter.GaussianBlur(radius=args.size * 0.08))
    black = Image.new("RGB", image.size, (12, 10, 9))
    image = Image.composite(image, black, vignette)

    draw = ImageDraw.Draw(image)
    draw.rectangle((0, 0, args.size - 1, args.size - 1), outline=(245, 158, 11), width=8)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    # O PNG é o artefacto versionado: grava-se ao lado e troca-se, para uma
    # interrupção não deixar `ui/vael_map.png` truncado (o jogo carrega-o).
    temporary = OUT.with_name(OUT.name + ".tmp.png")
    image.save(temporary, optimize=True)
    os.replace(temporary, OUT)
    print(f"{OUT} ({args.size}×{args.size}, {OUT.stat().st_size / 1024:.0f} KiB)")


if __name__ == "__main__":
    main()
