#!/usr/bin/env python3
"""Audita as texturas dos GLB do pool e poda imagens orfas com bufferView vazio.

O `patch-pbr` zera o bufferView das imagens que substitui em vez de as remover;
se nenhum `finish`/prune correr a seguir, a imagem fica no JSON com 0 bytes. O
Bevy carrega **todas** as imagens do GLB, usadas ou não, e desiste do asset
inteiro (`Failed to parse ktx2 file: UnexpectedEnd`).

    python3 tools/glb_audit.py                       # só lista
    python3 tools/glb_audit.py --prune-orphans       # repõe, guardando cópia em --backup

`--prune-orphans` só remove imagens que **nenhum** material referencia; o BIN
sai byte a byte igual e cada ligação material→textura→bytes e acessor→bytes é
comparada antes e depois (ver `snapshot`). Um asset com uma imagem vazia em uso
é recusado, nunca arranjado às cegas.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
MESHES = ROOT / "assets/public/assets/meshes"
JSON_CHUNK = 0x4E4F534A
BIN_CHUNK = 0x004E4942
# Extensões que apontam uma textura para uma imagem (a última ganha).
IMAGE_SOURCES = ("KHR_texture_basisu", "EXT_texture_webp", "EXT_texture_avif", "MSFT_texture_dds")


def read_glb(path: Path) -> tuple[int, list[tuple[int, bytes]]]:
    data = path.read_bytes()
    if data[:4] != b"glTF":
        raise ValueError("não é um GLB")
    version, _length = struct.unpack("<II", data[4:12])
    chunks: list[tuple[int, bytes]] = []
    off = 12
    while off + 8 <= len(data):
        clen, ctype = struct.unpack("<II", data[off : off + 8])
        chunks.append((ctype, data[off + 8 : off + 8 + clen]))
        off += 8 + clen
    return version, chunks


def write_glb(path: Path, version: int, chunks: list[tuple[int, bytes]]) -> None:
    out = []
    for ctype, payload in chunks:
        pad = (-len(payload)) % 4
        out.append(struct.pack("<II", len(payload) + pad, ctype) + payload + (b" " if ctype == JSON_CHUNK else b"\0") * pad)
    body = b"".join(out)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_bytes(struct.pack("<III", 0x46546C67, version, 12 + len(body)) + body)
    os.replace(tmp, path)


def parts(chunks: list[tuple[int, bytes]]) -> tuple[dict, bytes]:
    js = next(json.loads(c) for t, c in chunks if t == JSON_CHUNK)
    bin_ = next((c for t, c in chunks if t == BIN_CHUNK), b"")
    return js, bin_


def texture_image(tex: dict) -> int | None:
    for name in IMAGE_SOURCES:
        source = (tex.get("extensions") or {}).get(name, {}).get("source")
        if source is not None:
            return source
    return tex.get("source")


def used_textures(js: dict) -> set[int]:
    """Índices de textura alcançáveis de algum slot `*Texture` de algum material."""
    used: set[int] = set()

    def walk(node, parent_key: str) -> None:
        if isinstance(node, dict):
            for key, value in node.items():
                if key == "index" and isinstance(value, int) and parent_key.endswith("Texture"):
                    used.add(value)
                else:
                    walk(value, key)
        elif isinstance(node, list):
            for value in node:
                walk(value, parent_key)

    for material in js.get("materials") or []:
        walk(material, "")
    return used


def empty_images(js: dict) -> set[int]:
    views = js.get("bufferViews") or []
    return {
        i
        for i, image in enumerate(js.get("images") or [])
        if "bufferView" in image and views[image["bufferView"]]["byteLength"] == 0
    }


def snapshot(version: int, chunks: list[tuple[int, bytes]]) -> dict:
    """O que o GLB *resolve*: cada ligação vira o hash dos bytes que lê."""
    js, bin_ = parts(chunks)
    views = js.get("bufferViews") or []
    images = js.get("images") or []
    textures = js.get("textures") or []

    def view_bytes(index: int) -> tuple:
        view = views[index]
        ext = (view.get("extensions") or {}).get("EXT_meshopt_compression")
        if ext is not None:
            if ext.get("buffer", 0) != 0:
                return ("fallback", ext.get("buffer"), ext.get("byteOffset", 0), ext["byteLength"])
            off, length = ext.get("byteOffset", 0), ext["byteLength"]
        else:
            off, length = view.get("byteOffset", 0), view["byteLength"]
        return (view.get("buffer", 0), off, length, hashlib.sha1(bin_[off : off + length]).hexdigest(), view.get("byteStride"))

    def image_bytes(index: int) -> tuple:
        image = images[index]
        if "bufferView" not in image:
            return ("uri", image.get("uri"))
        view = views[image["bufferView"]]
        off = view.get("byteOffset", 0)
        return ("emb", image.get("mimeType"), image.get("name"), view["byteLength"], hashlib.sha1(bin_[off : off + view["byteLength"]]).hexdigest())

    def texture_bytes(index: int) -> tuple:
        tex = textures[index]
        source = texture_image(tex)
        others = {k: v for k, v in (tex.get("extensions") or {}).items() if k not in IMAGE_SOURCES}
        return (image_bytes(source) if source is not None else None, json.dumps(tex.get("sampler"), sort_keys=True), json.dumps(others, sort_keys=True))

    def resolve(node, parent_key: str):
        if isinstance(node, dict):
            out = {}
            for key, value in node.items():
                if key == "index" and isinstance(value, int) and parent_key.endswith("Texture"):
                    out[key] = "tex:" + json.dumps(texture_bytes(value))
                else:
                    out[key] = resolve(value, key)
            return out
        if isinstance(node, list):
            return [resolve(v, parent_key) for v in node]
        return node

    accessors = []
    for acc in js.get("accessors") or []:
        rest = {k: v for k, v in acc.items() if k != "bufferView" and k != "sparse"}
        accessors.append(
            (
                json.dumps(rest, sort_keys=True),
                view_bytes(acc["bufferView"]) if "bufferView" in acc else None,
                json.dumps(
                    {
                        sub: view_bytes(acc["sparse"][sub]["bufferView"])
                        for sub in ("indices", "values")
                        if sub in acc.get("sparse", {}) and "bufferView" in acc["sparse"][sub]
                    },
                    sort_keys=True,
                ),
            )
        )
    return {
        "version": version,
        "asset": js.get("asset"),
        "ext_used": js.get("extensionsUsed"),
        "ext_required": js.get("extensionsRequired"),
        "bin": (len(bin_), hashlib.sha1(bin_).hexdigest()),
        "buffers": js.get("buffers"),
        "samplers": js.get("samplers"),
        "materials": [json.dumps(resolve(m, ""), sort_keys=True) for m in js.get("materials") or []],
        "accessors": accessors,
        "meshes": js.get("meshes"),
        "nodes": js.get("nodes"),
        "skins": js.get("skins"),
        "animations": js.get("animations"),
        "cameras": js.get("cameras"),
        "scenes": js.get("scenes"),
        "scene": js.get("scene"),
        "extensions": js.get("extensions"),
        "extras": js.get("extras"),
    }


def prune(js: dict) -> tuple[dict, dict]:
    """Devolve o JSON novo e a conta do que saiu."""
    images = js.get("images") or []
    textures = js.get("textures") or []
    views = js.get("bufferViews") or []
    drop_img = empty_images(js)
    if not drop_img:
        return js, {"images": 0, "textures": 0, "bufferViews": 0}
    in_use = {texture_image(textures[t]) for t in used_textures(js)}
    clash = sorted(drop_img & in_use)
    if clash:
        raise ValueError("imagem vazia em uso por um material: %s" % clash)
    drop_tex = {t for t in range(len(textures)) if texture_image(textures[t]) in drop_img}

    referenced_views: set[int] = set()
    for acc in js.get("accessors") or []:
        if "bufferView" in acc:
            referenced_views.add(acc["bufferView"])
        for sub in ("indices", "values"):
            if sub in acc.get("sparse", {}) and "bufferView" in acc["sparse"][sub]:
                referenced_views.add(acc["sparse"][sub]["bufferView"])
    for i, image in enumerate(images):
        if i not in drop_img and "bufferView" in image:
            referenced_views.add(image["bufferView"])
    drop_view = {images[i]["bufferView"] for i in drop_img} - referenced_views

    img_map = {old: new for new, old in enumerate(i for i in range(len(images)) if i not in drop_img)}
    tex_map = {old: new for new, old in enumerate(t for t in range(len(textures)) if t not in drop_tex)}
    view_map = {old: new for new, old in enumerate(v for v in range(len(views)) if v not in drop_view)}

    def take(seq, drop, mapper):
        return [mapper(item) for i, item in enumerate(seq) if i not in drop]

    def remap(node, where: str, parent_key: str):
        if isinstance(node, dict):
            for key, value in node.items():
                if key == "bufferView" and isinstance(value, int):
                    node[key] = view_map[value]
                elif where == "texture" and key == "source" and isinstance(value, int):
                    node[key] = img_map[value]
                elif key == "index" and isinstance(value, int) and parent_key.endswith("Texture"):
                    node[key] = tex_map[value]
                else:
                    remap(value, where, key)
        elif isinstance(node, list):
            for value in node:
                remap(value, where, parent_key)

    new = json.loads(json.dumps(js))
    remap({k: v for k, v in new.items() if k not in ("images", "textures")}, "", "")
    new["images"] = take(images, drop_img, lambda im: im)
    new["textures"] = take(textures, drop_tex, lambda tex: (remap(tex, "texture", ""), tex)[1])
    new["bufferViews"] = take(views, drop_view, lambda view: view)
    for i, im in enumerate(images):
        if i not in drop_img and "bufferView" in im:
            new["images"][img_map[i]]["bufferView"] = view_map[im["bufferView"]]
    return new, {"images": len(drop_img), "textures": len(drop_tex), "bufferViews": len(drop_view)}


def validate_images(path: Path, *, validate_ktx: bool = True) -> list[str]:
    """Verifica URIs locais e, se pedido, os bytes KTX2."""
    _version, chunks = read_glb(path)
    js, bin_ = parts(chunks)
    problems = []
    for i, image in enumerate(js.get("images") or []):
        if "bufferView" not in image:
            uri = urlsplit(image.get("uri", ""))
            if uri.scheme or uri.netloc or not uri.path:
                continue
            external = path.parent / unquote(uri.path)
            if not external.is_file():
                problems.append(f"img{i} ficheiro em falta: {uri.path}")
                continue
            if not validate_ktx or external.suffix.lower() != ".ktx2":
                continue
            blob = external.read_bytes()
        else:
            if not validate_ktx:
                continue
            view = js["bufferViews"][image["bufferView"]]
            off = view.get("byteOffset", 0)
            blob = bin_[off : off + view["byteLength"]]
        if blob[:4] != b"\xabKTX":
            if image.get("mimeType") == "image/ktx2" or len(blob) == 0:
                problems.append("img%d %dB sem cabeçalho KTX2" % (i, len(blob)))
            continue
        with tempfile.NamedTemporaryFile(suffix=".ktx2", delete=False) as tmp:
            tmp.write(blob)
        try:
            done = subprocess.run(["ktx", "validate", tmp.name], capture_output=True, text=True)
        finally:
            os.unlink(tmp.name)
        if done.returncode != 0:
            lines = (done.stdout or done.stderr).strip().splitlines()
            problems.append("img%d inválida: %s" % (i, lines[-1][:90] if lines else done.returncode))
    return problems


def pool_files(root: Path, only: str | None) -> list[Path]:
    files = sorted(p for p in root.rglob("*.glb") if "_intermediate" not in p.parts and ".jackdaw-cache" not in p.parts)
    return [p for p in files if only is None or only in str(p.relative_to(root))]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--prune-orphans", action="store_true", help="poda as imagens orfas vazias (por omissão só lista)")
    parser.add_argument("--backup", type=Path, help="pasta onde guardar os originais (obrigatória com --prune-orphans)")
    parser.add_argument("--only", help="restringe a um pedaço do caminho (ex.: interiors/rug_woven)")
    parser.add_argument("--no-ktx", action="store_true", help="salta o `ktx validate`")
    parser.add_argument("--root", type=Path, default=MESHES, help="pasta do pool (por omissão assets/public/assets/meshes)")
    args = parser.parse_args()
    if args.prune_orphans and not args.backup:
        parser.error("--prune-orphans precisa de --backup <pasta>")

    broken = 0
    files = pool_files(args.root, args.only)
    for path in files:
        rel = path.relative_to(args.root)
        try:
            version, chunks = read_glb(path)
            js, _ = parts(chunks)
        except (OSError, ValueError) as exc:
            print("✗ %-46s %s" % (rel, exc))
            broken += 1
            continue
        empties = empty_images(js)
        problems = [] if empties else validate_images(path, validate_ktx=not args.no_ktx)
        if not empties and not problems:
            continue
        if not args.prune_orphans or problems:
            broken += 1
            print("✗ %-46s %s" % (rel, ", ".join(problems) or "%d imagem(ns) vazia(s)" % len(empties)))
            continue
        try:
            new, removed = prune(js)
            before = snapshot(version, chunks)
            new_chunks = [(t, json.dumps(new, separators=(",", ":"), ensure_ascii=False).encode() if t == JSON_CHUNK else c) for t, c in chunks]
            if snapshot(version, new_chunks) != before:
                raise ValueError("a poda alterou ligações; recusado")
        except (ValueError, KeyError, IndexError) as exc:
            print("✗ %-46s %s" % (rel, exc))
            continue
        out = args.backup / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, out)
        write_glb(path, version, new_chunks)
        left = validate_images(path)
        if left:
            broken += 1
        state = "ok" if not left else "KTX INVÁLIDA: " + ", ".join(left)
        print("✓ %-46s −%d img, −%d tex, −%d bv  json %d→%dB  %s" % (
            rel, removed["images"], removed["textures"], removed["bufferViews"],
            len(chunks[0][1]), len(read_glb(path)[1][0][1]), state))
    print("%s: %d de %d GLB com problemas" % ("podados" if args.prune_orphans else "auditoria", broken, len(files)))
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
