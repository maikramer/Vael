#!/usr/bin/env python3
"""Auditoria de terreno do Vael: entidades enterradas ou a flutuar.

Percorre a árvore de `Include`s do `world.xml`, compõe as translações dos
`<Group>` (sem rotações), e compara o Y de cada entidade com mesh/NPC contra o
heightmap `world.data/terrain.ahgt` (bilinear).

Uso:
  python3 tools/audit_terrain.py                # resumo + outliers
  python3 tools/audit_terrain.py --min 0.35     # limiar |dy|
  python3 tools/audit_terrain.py --json out.json
"""
import argparse, json, math, struct, sys, zlib
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORLD = ROOT / "world.xml"
AHGT = ROOT / "world.data" / "terrain.ahgt"
AHGT_MAGIC = 0x54474841


def load_ahgt(path=AHGT):
    raw = path.read_bytes()
    magic = struct.unpack_from("<I", raw, 0)[0]
    version, width, depth = struct.unpack_from("<HHH", raw, 4)
    meta_len = struct.unpack_from("<I", raw, 16)[0]
    if magic != AHGT_MAGIC or version != 1:
        raise SystemExit(f"AHGT inválido: magic=0x{magic:08x} version={version}")
    meta = json.loads(raw[20 : 20 + meta_len])
    ws = float(meta.get("worldSize", 4000.0))
    mh = float(meta["maxHeight"])
    payload = raw[20 + meta_len :]
    try:
        h = zlib.decompress(payload)
    except zlib.error:
        h = zlib.decompress(payload, -15)
    import numpy as np

    grid = np.frombuffer(h, dtype="<u2")[: width * depth].reshape(depth, width)
    grid = grid.astype(np.float64) * (mh / 65535.0)
    return grid, width, depth, ws, mh


GRID, W, D, WORLD_SIZE, MAX_H = load_ahgt()


def terrain(x, z):
    """Bilinear no heightmap; o documento cobre [-ws/2, +ws/2] em XZ."""
    fx = (x + WORLD_SIZE / 2) / WORLD_SIZE * W - 0.5
    fz = (z + WORLD_SIZE / 2) / WORLD_SIZE * D - 0.5
    ix, iz = math.floor(fx), math.floor(fz)
    if ix < 0 or iz < 0 or ix + 1 >= W or iz + 1 >= D:
        return None
    tx, tz = fx - ix, fz - iz
    g = GRID
    return (
        g[iz][ix] * (1 - tx) * (1 - tz)
        + g[iz][ix + 1] * tx * (1 - tz)
        + g[iz + 1][ix] * (1 - tx) * tz
        + g[iz + 1][ix + 1] * tx * tz
    )


def parse_vec(text):
    return [float(v) for v in text.split()]


def has_rotation(rot):
    """`euler="0 28 0"` / `transform="rotation: 0 90 0"` / `rotation="quat"`.

    O teste antigo procurava a SUBSTRING "rotation" no VALOR do atributo —
    `euler="0 28 0"` nunca a contém, por isso os grupos rodados do mundo
    (cave.minecart, village.cart) passavam em silêncio e a composição, que só
    soma translações, auditava os filhos no sítio errado."""
    if not rot:
        return False
    text = rot.split("rotation:")[-1].split(";")[0].strip()
    try:
        vals = [float(v) for v in text.split()]
    except ValueError:
        return True  # formato desconhecido: avisar na mesma
    if len(vals) == 4:  # quaternion: identidade é 0 0 0 1
        return any(abs(v) > 1e-6 for v in vals[:3]) or abs(vals[3] - 1.0) > 1e-6
    return any(abs(v) > 1e-6 for v in vals)


def has_descendant(el, tag):
    for child in el.iter(tag):
        return True
    return False


def walk_module(path, off, groups_out, ents_out, seen, depth=0):
    if path in seen or depth > 6:
        return
    seen.add(path)
    root = ET.parse(path).getroot()
    walk_element(root, off, path, groups_out, ents_out, seen, depth)


def walk_element(el, off, path, groups_out, ents_out, seen, depth):
    gx, gy, gz = off
    for child in el:
        tag = child.tag
        if tag == "Include":
            src = (child.get("src") or "").lstrip("/")
            if src:
                walk_module(str(ROOT / src), off, groups_out, ents_out, seen, depth + 1)
            continue
        tr = child.get("translation")
        dx, dy, dz = parse_vec(tr) if tr else (0.0, 0.0, 0.0)
        rot = child.get("transform") or child.get("euler") or child.get("rotation") or ""
        if tag == "Group":
            if has_rotation(rot):
                print(f"AVISO: grupo com rotação em {path}: {child.attrib}", file=sys.stderr)
            ngo = (gx + dx, gy + dy, gz + dz)
            groups_out.append((path, child.get("name") or "", ngo))
            walk_element(child, ngo, path, groups_out, ents_out, seen, depth)
            continue
        if tag != "Entity":
            continue
        ex, ey, ez = gx + dx, gy + dy, gz + dz
        gltf = has_descendant(child, "GltfScene")
        npc = has_descendant(child, "NpcBody")
        script = child.get("script") or ""
        info = {
            "path": f"{path}",
            "name": child.get("name") or "",
            "x": ex, "y": ey, "z": ez,
            "gltf": gltf, "npc": npc, "script": script,
            "collider": child.get("collider") or "",
            "props": {k: v for k, v in child.attrib.items() if k.startswith("prop-")},
        }
        ents_out.append(info)
        walk_element(child, (ex, ey, ez), path, groups_out, ents_out, seen, depth)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--min", type=float, default=0.35, help="limiar |dy| para outlier")
    ap.add_argument("--json", default=None)
    args = ap.parse_args()

    groups, ents = [], []
    walk_module(str(WORLD), (0.0, 0.0, 0.0), groups, ents, set())

    # entidades com conteúdo visual/físico
    audit = [e for e in ents if e["gltf"] or e["npc"] or e["script"]]
    for e in audit:
        e["terrain"] = terrain(e["x"], e["z"])
        e["dy"] = None if e["terrain"] is None else e["y"] - e["terrain"]

    ok = [e for e in audit if e["dy"] is not None]
    noh = [e for e in audit if e["dy"] is None]
    buried = sorted([e for e in ok if e["dy"] < -args.min], key=lambda e: e["dy"])
    float_ = sorted([e for e in ok if e["dy"] > args.min], key=lambda e: -e["dy"])

    print(f"entidades auditadas: {len(audit)} (mesh/npc/script de {len(ents)} entities)")
    print(f"sem altura no mapa: {len(noh)}  |  dentro do limiar: {len(ok)-len(buried)-len(float_)}")
    print(f"ENTERRADAS (dy < -{args.min}): {len(buried)}   FLUTUANDO (dy > +{args.min}): {len(float_)}")

    def line(e):
        tag = "NPC " if e["npc"] else ("glb " if e["gltf"] else "scr ")
        name = e["name"] or "(sem nome)"
        scr = e["script"].split("/")[-1] if e["script"] else ""
        return (f"{tag} dy={e['dy']:+7.2f}  ({e['x']:8.1f},{e['y']:7.2f},{e['z']:8.1f})"
                f"  {name}  {scr}  [{e['path'].replace(str(ROOT) + '/', '')}]")

    for label, lst in (("ENTERRADAS", buried), ("FLUTUANDO", float_)):
        if lst:
            print(f"\n== {label} ==")
            for e in lst:
                print(line(e))
    if noh:
        print(f"\n== SEM ALTURA (fora do mapa?) ==")
        for e in noh[:20]:
            print(line(e))

    if args.json:
        with open(args.json, "w") as f:
            json.dump({"buried": buried, "floating": float_, "no_height": noh,
                       "audited": len(audit)}, f, ensure_ascii=False, indent=1)
        print(f"\njson -> {args.json}")


if __name__ == "__main__":
    main()
