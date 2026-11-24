#!/usr/bin/env python3
"""Bake offline do relevo exterior a partir do AHGT de 4 km.

O núcleo mantém o sistema de coordenadas original. A fonte da versão
jogável é o AHGT gravado. Depois deste passo, aplica o carve dos lagos
autorados em world/expansion/waters.bsn e gera ui/vael_map.png.
"""
from __future__ import annotations

import argparse
import json
import struct
import zlib
from pathlib import Path

import numpy as np

WORLD_SIZE = 8000.0
SEA_LEVEL = 18.0
SEED = 84317

# Centro, altura, raio de plataforma e largura da transição, em metros.
PADS = [
    (-2590, 335, 24, 105, 105),  # Porto do Poente
    (-3210, 375, 34, 30, 70),   # Ilha da Lanterna
    (-3070, -680, 27, 22, 65),  # Ilha dos Juncos
    (-1325, -1780, 53, 34, 115),  # Pouso dos Freixos
    (1410, -1740, 90, 42, 95),  # Refúgio do Vidro
    (1320, 1735, 63, 40, 95),   # Entreposto das Sete Margens
    (-1650, 2110, 66, 38, 100),  # Pouso do Sul
    (-2130, -230, 42, 38, 100),  # Passo da Salina
    (2320, 470, 78, 50, 140),   # Atalaia de Barro
    (125, -2735, 81, 44, 130),  # Marco do Norte
    (90, 2630, 78, 48, 110),    # Estalagem do Caminho Comprido
]

# Lagos: cota do espelho. Este passe prepara o terreno envolvente.
# O contorno orgânico final é o do carve normal da engine.
BASINS = [
    (-1570, -2170, 46, 235),
    (1650, -1995, 78, 190),
    (1650, 2170, 55, 320),
]


def smoothstep(t):
    t = np.clip(t, 0, 1)
    return t * t * (3 - 2 * t)


def gaussian(x, z, cx, cz, rx, rz):
    return np.exp(-((x - cx) / rx) ** 2 - ((z - cz) / rz) ** 2)


def noise(x, z, band, table):
    xx, zz = x / band + 256, z / band + 256
    xi, zi = xx.astype(np.int32), zz.astype(np.int32)
    dx, dz = smoothstep(xx - xi).astype(np.float32), smoothstep(zz - zi).astype(np.float32)
    xi, zi = xi % table.shape[1], zi % table.shape[0]
    x1, z1 = (xi + 1) % table.shape[1], (zi + 1) % table.shape[0]
    n0 = table[zi, xi] * (1 - dx) + table[zi, x1] * dx
    n1 = table[z1, xi] * (1 - dx) + table[z1, x1] * dx
    return n0 * (1 - dz) + n1 * dz


def exterior(x, z, tables):
    broad = noise(x, z, 650, tables[0])
    hills = noise(x, z, 245, tables[1])
    detail = noise(x, z, 85, tables[2])
    grain = noise(x, z, 28, tables[3])
    height = 58 + 18 * broad + 14 * hills + 4.5 * detail + 1.2 * grain
    height += 70 * gaussian(x, z, 2180, -2700, 1150, 500) * (0.79 + 0.24 * hills)
    height += 61 * gaussian(x, z, -900, -3170, 2050, 530)
    height += 68 * gaussian(x, z, 3410, -180, 510, 2570) * (0.85 + 0.32 * hills)
    height += 68 * gaussian(x, z, 620, 3240, 2300, 530)
    height += 35 * gaussian(x, z, -2180, 2070, 660, 530) * (0.8 + 0.3 * broad)
    # Duas enseadas abertas a oeste. A língua de água do porto entra entre
    # duas promontórios, com uma praia rasa e uma bacia de manobra.
    shore = -2790 + 110 * np.sin(z / 310) + 90 * np.sin(z / 560 + 0.9)
    shore += 310 * np.exp(-((z - 200) / 255) ** 2)
    shore += 135 * np.exp(-((z + 860) / 235) ** 2)
    shore += 29 * noise(np.zeros((1, 1), dtype=np.float32), z, 115, tables[2])
    distance = x - shore
    shelf = SEA_LEVEL + 14 * np.tanh(distance / 113) + detail * 1.5
    height = shelf * (1 - smoothstep((distance - 45) / 355)) + height * smoothstep((distance - 45) / 355)
    # Duas ilhas dentro do limite de navegação e um recife além do limite.
    for cx, cz, rx, rz, rise in [
        (-3220, 405, 126, 100, 34),
        (-3080, -690, 101, 139, 25),
        (-3145, -905, 76, 56, 15),
        (-3790, 690, 200, 123, 24),
    ]:
        height += rise * gaussian(x, z, cx, cz, rx, rz) * (1 + detail * 0.10)
    for cx, cz, level, radius in BASINS:
        d = np.hypot(x - cx, z - cz)
        portion = 1 - smoothstep((d - 1.15 * radius) / (radius * 0.75))
        rim_floor = level + 2.4
        bank = np.maximum(level + 3.0 + np.clip((d - radius) * 0.055, -1.5, 18.0) + detail, rim_floor)
        height = portion * np.maximum(height, bank) * 0.35 + portion * bank * 0.65 + (1 - portion) * height
        rim_band = (1 - smoothstep((d - 1.45 * radius) / (radius * 0.35))) * smoothstep((d - 0.85 * radius) / (radius * 0.25))
        height = np.maximum(height, height * (1 - rim_band) + np.maximum(height, rim_floor) * rim_band)
    for cx, cz, level, radius, shoulder in PADS:
        d = np.hypot(x - cx, z - cz)
        portion = 1 - smoothstep((d - radius) / shoulder)
        height = portion * level + (1 - portion) * height
    return height.astype(np.float32)


def write_ahgt(base: Path, destination: Path):
    raw = base.read_bytes()
    header = bytearray(raw[:20])
    width, depth = struct.unpack_from("<HH", header, 6)
    length = struct.unpack_from("<I", header, 16)[0]
    meta = json.loads(raw[20:20 + length])
    old_size = float(meta["worldSize"])
    if old_size != WORLD_SIZE / 2 or width != depth:
        raise SystemExit("A base deve ser a grelha quadrada de 4 km.")
    max_height = float(meta["maxHeight"])
    payload = raw[20 + length:]
    try:
        plain = zlib.decompress(payload)
    except zlib.error:
        plain = zlib.decompress(payload, -15)
    old_raw = np.frombuffer(plain, dtype="<u2").reshape(depth, width)
    side = width * 2
    axis = np.linspace(-WORLD_SIZE / 2, WORLD_SIZE / 2, side, dtype=np.float32)
    x = axis[None, :]
    # Prolonga as encostas com derivada contínua junto aos limites do AHGT antigo.
    rim = old_size * .45
    shoulder = old_size * .05
    old_axis = np.sign(axis) * (np.minimum(abs(axis), rim)
                                + shoulder * np.tanh(np.maximum(abs(axis) - rim, 0) / shoulder))
    old_coord = (old_axis / old_size + .5) * (width - 1)
    lo = np.floor(old_coord).astype(np.int32)
    hi = np.minimum(lo + 1, width - 1)
    mix = old_coord - lo
    generator = np.random.default_rng(SEED)
    tables = [generator.uniform(-1, 1, (512, 512)).astype(np.float32) for _ in range(4)]
    meta["worldSize"] = WORLD_SIZE
    meta_bytes = json.dumps(meta, separators=(",", ":"), ensure_ascii=False).encode()
    struct.pack_into("<HH", header, 6, side, side)
    struct.pack_into("<I", header, 16, len(meta_bytes))
    compress = zlib.compressobj(level=6)
    temp = destination.with_suffix(".tmp")
    with temp.open("wb") as out:
        out.write(header)
        out.write(meta_bytes)
        for start in range(0, side, 96):
            stop = min(start + 96, side)
            z = axis[start:stop, None]
            heights = exterior(x, z, tables)
            if abs(z).min() < 2490:
                extent = np.hypot(x, z)
                blend_width = 585 + 130 * noise(x, z, 330, tables[0])
                keep = 1 - smoothstep((extent - 1750) / blend_width)
                for cx, cz, _, radius in BASINS:
                    distance = np.hypot(x - cx, z - cz)
                    shore = smoothstep((distance - radius * 1.45) / 165)
                    keep *= np.maximum(shore, 1 - smoothstep((extent - 1750) / 110))
                for cx, cz, _, radius, shoulder in PADS:
                    distance = np.hypot(x - cx, z - cz)
                    keep *= smoothstep((distance - radius) / shoulder)
                u, v, f = lo[start:stop, None], hi[start:stop, None], mix[start:stop, None]
                old_a = old_raw[u, lo] * (1 - mix) + old_raw[u, hi] * mix
                old_b = old_raw[v, lo] * (1 - mix) + old_raw[v, hi] * mix
                old = (old_a * (1 - f) + old_b * f) * (max_height / 65535)
                heights = heights * (1 - keep) + old * keep
            samples = np.rint(np.clip(heights / max_height, 0, 1) * 65535).astype("<u2")
            out.write(compress.compress(samples.tobytes()))
        out.write(compress.flush())
    temp.replace(destination)
    print(f"{destination}: {side}² · {WORLD_SIZE:g} m")


def expand_paint(base: Path, destination: Path):
    """Enquadra a pintura antiga, mantendo a dimensão de cada célula."""
    raw = base.read_bytes()
    size, side = struct.unpack_from("<fI", raw, 8)
    if abs(size * 2 - WORLD_SIZE) > 0.01 or len(raw) != 16 + side**2 * 2:
        raise SystemExit("A pintura deve acompanhar a grelha de 4 km.")
    paint = np.zeros((side * 2, side * 2, 2), dtype=np.uint8)
    offset = side // 2
    paint[offset:offset + side, offset:offset + side] = np.frombuffer(
        raw[16:], dtype=np.uint8
    ).reshape(side, side, 2)
    temporary = destination.with_suffix(".tmp")
    with temporary.open("wb") as out:
        out.write(raw[:8] + struct.pack("<fI", WORLD_SIZE, side * 2))
        paint.tofile(out)
    temporary.replace(destination)
    print(f"{destination}: {side * 2}²")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path, help="AHGT original de 4 km")
    parser.add_argument("destination", type=Path, help="AHGT expandido")
    args = parser.parse_args()
    write_ahgt(args.base, args.destination)
    paint_source = args.base.with_suffix(".ahgt.paint")
    if paint_source.is_file():
        expand_paint(paint_source, args.destination.with_suffix(".ahgt.paint"))


if __name__ == "__main__":
    main()
