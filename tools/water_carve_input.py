#!/usr/bin/env python3
"""Gera o JSON do `carve_water` a partir dos `<Lake>`/`<River>` do mundo.

O mundo é estático: o runtime carva a água numa CÓPIA da grelha, por isso um
lago ou rio novo no `.bsn` só abre o leito depois de gravado no `.ahgt`.
Este script lê os corpos de água de `world/**/*.bsn`; os que estão nos
módulos passados em `--new` são os que se gravam, os restantes vão como
`prior` (confluência e máscara de margens).

Uso:
    python3 tools/water_carve_input.py --new world/landmarks/hidrovias.bsn -o /tmp/water.json
    cargo run --release -p vael_engine --example carve_water -- \
        world.data/terrain.ahgt /tmp/water.json --dry-run

Os níveis (`water-level` / `water-levels`) têm de estar explícitos no `.bsn`.
Correr o carve UMA vez por corpo novo: gravar de novo o mesmo corpo volta a
escavar margens (o eixo amostrado já é o leito).
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NODE = re.compile(r'tag: "(Lake|River)",\s*attrs: \[(.*?)\n\s*\]', re.S)
PROP = re.compile(r'key: "([^"]*)", value: "([^"]*)"')


def body(kind: str, a: dict[str, str], where: str) -> dict:
    if kind == "Lake":
        if "water-level" not in a:
            raise SystemExit(f"{where}: <Lake at={a.get('at')}> sem water-level explícito")
        out = {
            "kind": "lake",
            "name": a.get("name", f"lake@{a['at']}"),
            "at": [float(v) for v in a["at"].split()],
            "radius": float(a["radius"]),
            "depth": float(a.get("depth", 1.5)),
            "water_offset": float(a.get("water-offset", 0.5)),
            "water_level": float(a["water-level"]),
        }
        for key, cast in (("seed", lambda v: int(float(v))), ("axis", float), ("stretch", float), ("lobes", float)):
            if key in a:
                out[key] = cast(a[key])
        return out
    if "water-levels" not in a:
        raise SystemExit(f"{where}: <River> sem water-levels explícitos")
    xs = [float(v) for v in a["path"].split()]
    return {
        "kind": "river",
        "name": a.get("name", "river"),
        "path": [xs[i : i + 2] for i in range(0, len(xs), 2)],
        "width": float(a.get("width", 6)),
        "depth": float(a.get("depth", 1.5)),
        "water_offset": float(a.get("water-offset", 0.3)),
        "bank_width": float(a.get("bank-width", 2)),
        "bank_height": float(a.get("bank-height", 0.9)),
        "pool_spacing": float(a.get("pool-spacing", 0)),
        "water_levels": [float(v) for v in a["water-levels"].split()],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--new", action="append", required=True, help="módulo .bsn com os corpos a gravar")
    parser.add_argument("-o", "--output", required=True)
    args = parser.parse_args()
    new_files = {str(Path(p)) for p in args.new}
    prior: list[dict] = []
    new: list[dict] = []
    for path in sorted((ROOT / "world").rglob("*.bsn")):
        rel = str(path.relative_to(ROOT))
        if rel.startswith(("world/prefabs/", "world/scenes/")):
            continue
        for match in NODE.finditer(path.read_text()):
            attrs = dict(PROP.findall(match.group(2)))
            (new if rel in new_files else prior).append(body(match.group(1), attrs, rel))
    # Mesma ordem do `apply_features`: lagos antes de rios.
    order = lambda b: 0 if b["kind"] == "lake" else 1  # noqa: E731
    prior.sort(key=order)
    new.sort(key=order)
    Path(args.output).write_text(json.dumps({"height_smoothing": 1.0, "prior": prior, "new": new}))
    print(f"{len(prior)} corpos existentes, {len(new)} a gravar -> {args.output}")


if __name__ == "__main__":
    main()
