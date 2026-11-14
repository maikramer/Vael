#!/usr/bin/env python3
"""Soma um deslocamento Y à translação do pelvis de um clip (bake glTF).

Os clips de natação (UAL1 `Swim_Fwd_Loop`/`Swim_Idle_Loop`, retargetados para o
herói) vêm com o **root na linha de água**: o corpo boia em torno de y≈0 do
rig. No jogo a física do controller (`src/player.rs`, `SWIM_IDLE_DEPTH`) mantém
a raiz do herói ~1 m ABAIXO da lâmina, por isso o clip precisa de +1,0 m no
pelvis para o corpo aparecer à superfície. O `AnimState::Swim/SwimIdle` da
engine depende desta convenção (ver `SWIM_SUBMERSION` em
`bevy_ext_viber_engine/src/animation.rs`).

Uso:
  python3 tools/offset_anim.py <clip.glb> <nome-do-clip> <dy> [-o saida.glb]
  python3 tools/offset_anim.py q_swim.glb swim 1.0 -o q_swim_offset.glb

Reaplicar depois de qualquer regen do herói que reponha os clips sem offset
(o pipeline não conhece esta convenção; o merge do clip é manual).
"""
import json
import struct
import sys
from pathlib import Path


def load_glb(path):
    data = Path(path).read_bytes()
    magic, version, _ = struct.unpack_from("<III", data, 0)
    assert magic == 0x46546C67 and version == 2, path
    off = 12
    chunks = []
    while off < len(data):
        clen, ctype = struct.unpack_from("<II", data, off)
        off += 8
        chunks.append((ctype, data[off:off + clen]))
        off += clen
    js = json.loads(chunks[0][1].decode("utf-8"))
    bin_ = next((c for t, c in chunks[1:] if t == 0x004E4942), b"")
    return js, bytearray(bin_)


def save_glb(path, js, bin_):
    payload = json.dumps(js, separators=(",", ":")).encode("utf-8")
    payload += b" " * (-len(payload) % 4)
    bin_ = bytes(bin_) + b"\x00" * (-len(bin_) % 4)
    total = 12 + 8 + len(payload) + (8 + len(bin_) if bin_ else 0)
    out = bytearray()
    out += struct.pack("<III", 0x46546C67, 2, total)
    out += struct.pack("<II", len(payload), 0x4E4F534A) + payload
    if bin_:
        out += struct.pack("<II", len(bin_), 0x004E4942) + bin_
    Path(path).write_bytes(bytes(out))


def accessor_offsets(gltf, index):
    acc = gltf["accessors"][index]
    bv = gltf["bufferViews"][acc["bufferView"]]
    ncomp = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}[acc["type"]]
    assert acc["componentType"] == 5126, "só float32 (GLB do animator3d)"
    start = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
    stride = bv.get("byteStride") or ncomp * 4
    return acc, start, stride, ncomp


def offset_clip(path, clip, dy, output):
    gltf, bin_ = load_glb(path)
    anim = next((a for a in gltf.get("animations", []) if a.get("name") == clip), None)
    if anim is None:
        raise SystemExit(f"{path}: clip {clip!r} ausente; tem {[a.get('name') for a in gltf.get('animations', [])]}")
    names = {i: n.get("name") for i, n in enumerate(gltf["nodes"])}
    touched = 0
    for ch in anim["channels"]:
        tgt = ch["target"]
        if tgt.get("path") != "translation" or names.get(tgt["node"]) != "pelvis":
            continue
        samp = anim["samplers"][ch["sampler"]]
        acc, start, stride, ncomp = accessor_offsets(gltf, samp["output"])
        assert ncomp == 3
        for i in range(acc["count"]):
            base = start + i * stride
            y = struct.unpack_from("<f", bin_, base + 4)[0]
            struct.pack_into("<f", bin_, base + 4, y + dy)
        if "min" in acc:
            acc["min"][1] += dy
        if "max" in acc:
            acc["max"][1] += dy
        touched += 1
    if not touched:
        raise SystemExit(f"{path}: sem canal translation do pelvis no clip {clip!r}")
    save_glb(output, gltf, bin_)
    return output


def main():
    args = [a for a in sys.argv[1:]]
    out = None
    if "-o" in args:
        i = args.index("-o")
        out = args[i + 1]
        args = args[:i] + args[i + 2:]
    if len(args) != 3:
        raise SystemExit(__doc__)
    path, clip, dy = args[0], args[1], float(args[2])
    output = out or path
    offset_clip(path, clip, dy, output)
    print(f"ok {output}: {clip!r} pelvis += {dy:+.2f} m em Y")


if __name__ == "__main__":
    main()
