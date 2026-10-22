#!/usr/bin/env python3
"""Gera o heightmap da cena de menu (acampamento).

O mundo de menu é pequeno e estático, por isso o terreno é autorado uma vez e
guardado como `scenes/menu.data/terrain.ahgt` (o mesmo formato do
`world.data/terrain.ahgt`): magic AHGT + meta JSON + grelha u16 deflate.

A clareira é plana (8.2 m) até aos 16 m de raio; fora dela o terreno sobe em
colinas suaves e fecha-se numa crista a norte (z negativo), atrás das tendas.
O herói entra a sul (z positivo) e olha para a fogueira na origem.

Uso: python3 tools/make_menu_terrain.py
"""

import json
import math
import struct
import zlib
from pathlib import Path

WORLD = 220.0
MAX_HEIGHT = 40.0
SIDE = 257
CLEARING_Y = 8.2
CLEARING_R = 14.0
BLEND_R = 28.0

OUT = Path(__file__).resolve().parent.parent / "scenes" / "menu.data" / "terrain.ahgt"


def hash01(ix: int, iz: int, seed: int) -> float:
    """splitmix-ish hash determinístico em [0, 1)."""
    v = (ix * 0x9E3779B1) ^ (iz * 0x85EBCA77) ^ (seed * 0xC2B2AE3D)
    v &= 0xFFFFFFFF
    v ^= v >> 16
    v = (v * 0x7FEB352D) & 0xFFFFFFFF
    v ^= v >> 15
    v = (v * 0x846CA68B) & 0xFFFFFFFF
    v ^= v >> 16
    return v / 0x100000000


def fade(t: float) -> float:
    return t * t * (3.0 - 2.0 * t)


def value_noise(x: float, z: float, seed: int) -> float:
    x0, z0 = math.floor(x), math.floor(z)
    fx, fz = fade(x - x0), fade(z - z0)
    a = hash01(x0, z0, seed)
    b = hash01(x0 + 1, z0, seed)
    c = hash01(x0, z0 + 1, seed)
    d = hash01(x0 + 1, z0 + 1, seed)
    top = a + (b - a) * fx
    bottom = c + (d - c) * fx
    return top + (bottom - top) * fz


def fbm(x: float, z: float, seed: int, octaves: int = 4) -> float:
    total, amp, norm, freq = 0.0, 0.5, 0.0, 1.0
    for i in range(octaves):
        total += amp * value_noise(x * freq, z * freq, seed + i * 101)
        norm += amp
        amp *= 0.5
        freq *= 2.0
    return total / norm


def smoothstep(edge0: float, edge1: float, value: float) -> float:
    if edge0 == edge1:
        return 1.0 if value >= edge1 else 0.0
    t = max(0.0, min(1.0, (value - edge0) / (edge1 - edge0)))
    return fade(t)


def terrain_height(x: float, z: float) -> float:
    # Base ondulada.
    h = 9.0
    h += (fbm(x / 62.0, z / 62.0, 17) - 0.5) * 12.0
    h += (fbm(x / 23.0, z / 23.0, 41) - 0.5) * 3.5
    h += (fbm(x / 8.5, z / 8.5, 73) - 0.5) * 1.0

    # Crista a norte, atrás das tendas.
    north = smoothstep(36.0, 96.0, -z)
    h += north * (14.0 + 12.0 * fbm(x / 34.0, z / 34.0, 91))

    # Ombros laterais fecham a clareira sem tapar a vista.
    side = smoothstep(48.0, 102.0, abs(x))
    h += side * (8.0 + 8.0 * fbm(x / 27.0, z / 27.0, 113))

    # Colina baixa a sul (atrás da câmara).
    south = smoothstep(74.0, 108.0, z)
    h += south * (4.0 + 5.0 * fbm(x / 31.0, z / 31.0, 131))

    # Clareira plana com transição macia.
    r = math.hypot(x, z)
    flat = CLEARING_Y + (fbm(x / 26.0, z / 26.0, 151) - 0.5) * 0.3
    clear_w = 1.0 - smoothstep(CLEARING_R, BLEND_R, r)
    h = h * (1.0 - clear_w) + flat * clear_w

    # Caminho de entrada a sul (levemente mais baixo, legível como passagem).
    path_w = 1.0 - smoothstep(3.4, 7.5, abs(x))
    path_len = smoothstep(10.0, 18.0, z) * (1.0 - smoothstep(44.0, 58.0, z))
    path = CLEARING_Y + max(0.0, z - 14.0) * 0.035
    h = h * (1.0 - path_w * path_len * 0.85) + path * (path_w * path_len * 0.85)

    return max(2.0, min(MAX_HEIGHT - 0.5, h))


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    data = []
    for iz in range(SIDE):
        wz = (iz / (SIDE - 1) - 0.5) * WORLD
        for ix in range(SIDE):
            wx = (ix / (SIDE - 1) - 0.5) * WORLD
            h = terrain_height(wx, wz)
            data.append(round(h / MAX_HEIGHT * 65535))

    raw = bytearray()
    for v in data:
        raw += struct.pack("<H", v)
    payload = zlib.compress(bytes(raw), 9)
    meta = json.dumps(
        {"worldSize": WORLD, "maxHeight": MAX_HEIGHT},
        separators=(",", ":"),
    ).encode()
    header = b"AHGT" + struct.pack("<HHH", 1, SIDE, SIDE) + b"\0" * 6
    header += struct.pack("<I", len(meta))
    OUT.write_bytes(header + meta + payload)

    print(f"{OUT} — {SIDE}x{SIDE}, {OUT.stat().st_size / 1024:.0f} KiB")
    for name, x, z in [
        ("entrada", 0.0, 18.0),
        ("fogueira", 0.0, 0.0),
        ("tenda-oeste", -7.0, -5.0),
        ("tenda-leste", 7.0, -5.0),
        ("pinheiro-oeste", -31.0, -17.0),
        ("pinheiro-leste", 33.0, -12.0),
        ("carvalho-sul", -24.0, 27.0),
    ]:
        print(f"  {name:16s} ({x:6.1f}, {z:6.1f}) → y={terrain_height(x, z):.2f}")


if __name__ == "__main__":
    main()
