#!/usr/bin/env python3
"""Relevo do Vale Exterior a partir do recorte sudoeste da Terrain3D.

A origem é assets/public/assets/terrain/terrain.ahgt. O AHGT de referência
já tem o relevo, o litoral, as bacias e as plataformas feitos em metros.
Este passe muda a faixa exterior seca e conserva os locais e canais autorados.
"""
from __future__ import annotations

import argparse
import json
import struct
import zlib
from pathlib import Path

import numpy as np
from PIL import Image

from expand_vale import BASINS, PADS, smoothstep
from make_map import load_ahgt

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets/public/assets/terrain/terrain.ahgt"
# Linhas/colunas abaixo da emenda principal. Vales a poente e contrafortes
# a nascente. A encosta tem mais continuidade que nos recortes centrais.
CROP = (2280, 56, 1440)
PRESERVED_LANDMARKS = [
    (720, -545, 230),  # Vigílias
    (864, 744, 245),   # Alto de Vael
    (170, 1450, 365),  # Bosque Encantado
    (-881, -320, 238), # Cidadela Caída
]


def refine(reference: Path, destination: Path, source: Path, crop: tuple[int, int, int]) -> None:
    old, width, depth, meta = load_ahgt(reference)
    atlas, aw, ad, _ = load_ahgt(source)
    row, col, side = crop
    if min(row, col) < 0 or side < 32 or row + side > ad or col + side > aw:
        raise ValueError("Recorte fora do heightmap de origem")
    cut = atlas[row:row+side, col:col+side]
    lo, hi = np.quantile(cut, [.025, .975])
    relief = np.clip((cut-lo) / max(hi-lo, .01), -.12, 1.13)
    relief = np.asarray(Image.fromarray((31 + relief*119).astype(np.float32))
                        .resize((width, depth), Image.Resampling.BICUBIC))
    size = float(meta["worldSize"])
    x = np.linspace(-size/2, size/2, width, dtype=np.float32).reshape(1, -1)
    z_axis = np.linspace(-size/2, size/2, depth, dtype=np.float32)
    with reference.open("rb") as existing:
        header = bytearray(existing.read(20))
    meta["reliefSource"] = {"name": source.name, "cropRowColumnSide": list(crop)}
    encoded = json.dumps(meta, separators=(",", ":")).encode()
    struct.pack_into("<I", header, 16, len(encoded))
    compress = zlib.compressobj(6)
    temporary = destination.with_name(destination.name + ".tmp")
    with temporary.open("wb") as out:
        out.write(header)
        out.write(encoded)
        for begin in range(0, depth, 96):
            stop = min(begin+96, depth)
            z = z_axis[begin:stop, None]
            radius = np.hypot(x, z)
            # Entrada ondulada no relevo antigo para evitar um anel visível.
            transition = 780 + 125*np.sin(np.arctan2(z, x)*3 + .7) + 70*np.cos(z/420)
            weight = .76*smoothstep((radius - transition) / 1050)
            # Mantém a batimetria e as praias orientadas; o novo relevo nasce
            # em terra firme, atrás das salinas e do porto.
            weight *= smoothstep((x + 2410) / 550)
            for cx, cz, _, r in BASINS:
                weight *= smoothstep((np.hypot(x-cx, z-cz) - r*1.50) / 455)
            for cx, cz, _, r, shoulder in PADS:
                weight *= smoothstep((np.hypot(x-cx, z-cz) - (r + shoulder*0.85)) / 180)
            for cx, cz, r in PRESERVED_LANDMARKS:
                weight *= smoothstep((np.hypot(x-cx, z-cz) - r) / 250)
            surface = old[begin:stop] + (relief[begin:stop] - old[begin:stop])*weight
            raw = np.rint(np.clip(surface / float(meta["maxHeight"]),0,1)*65535).astype("<u2")
            out.write(compress.compress(raw.tobytes()))
        out.write(compress.flush())
    temporary.replace(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--crop", type=int, nargs=3, default=CROP, metavar=("LINHA", "COLUNA", "TAMANHO"))
    args = parser.parse_args()
    if args.reference.resolve() == args.destination.resolve():
        parser.error("use ficheiros distintos para a referência e o resultado")
    refine(args.reference, args.destination, args.source, tuple(args.crop))
