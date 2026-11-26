#!/usr/bin/env python3
"""Share byte-identical GLB textures through Bevy's image asset paths.

Run without --apply to see what would change. --apply archives the original
models outside the checkout before rewriting them. Mesh data, material slots
and glTF extensions are preserved.
"""
from __future__ import annotations

import argparse
from bisect import bisect_right
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import tempfile

from glb_audit import BIN_CHUNK, JSON_CHUNK, MESHES, parts, read_glb, used_textures, write_glb

MIME_SUFFIX = {"image/jpeg": "jpg", "image/png": "png", "image/ktx2": "ktx2"}
LINEAR_SLOTS = {
    "normalTexture", "occlusionTexture", "metallicRoughnessTexture",
    "anisotropyTexture", "thicknessTexture", "clearcoatTexture",
    "clearcoatRoughnessTexture", "clearcoatNormalTexture",
}
MESHOPT_EXTENSIONS = ("EXT_meshopt_compression", "KHR_meshopt_compression")


def shared_path(pool: Path, data: bytes, mime: str, linear: bool, sampler=None) -> Path:
    sampler = sampler or {}
    settings = (
        linear, sampler.get("wrapS", 10497), sampler.get("wrapT", 10497),
        sampler.get("magFilter"), sampler.get("minFilter"),
    )
    digest = hashlib.sha256(json.dumps(settings).encode() + data).hexdigest()
    return pool / "_textures" / f"{digest}.{MIME_SUFFIX[mime]}"


def image_uri(model: Path, texture: Path) -> str:
    return Path(os.path.relpath(texture, model.parent)).as_posix()


def write_shared(texture: Path, data: bytes) -> None:
    if texture.is_file():
        if texture.read_bytes() != data:
            raise ValueError(f"Shared texture content mismatch: {texture}")
        return
    texture.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=texture.parent, delete=False) as pending:
        temporary = Path(pending.name)
        try:
            pending.write(data)
            pending.close()
            temporary.chmod(0o644)
            temporary.replace(texture)
        finally:
            temporary.unlink(missing_ok=True)


def fields(node, keys):
    if isinstance(node, dict):
        for key, value in node.items():
            if key in keys:
                yield node, key
            if key != "extras":
                yield from fields(value, keys)
    elif isinstance(node, list):
        for value in node:
            yield from fields(value, keys)


def view_span(view, blob):
    if view.get("buffer") != 0:
        return None
    start = view.get("byteOffset", 0)
    end = start + view["byteLength"]
    if not 0 <= start <= end <= len(blob):
        raise ValueError("Invalid byte range in GLB buffer")
    return start, end


def externalizable_ktx2(data):
    if len(data) < 80 or data[:12] != b"\xabKTX 20\xbb\r\n\x1a\n":
        return False
    scheme, dfd = struct.unpack_from("<II", data, 44)
    # The Vael GLB reader handles these embedded UASTC RRR/ETC1S layouts.
    # They cannot be passed unchanged to Bevy's standalone KTX2 loader.
    return dfd + 31 < len(data) and scheme != 1 and data[dfd + 8] != 163 and data[dfd + 31] != 4


def candidates(pool, doc, blob):
    used = used_textures(doc)
    linear = {parent[key]["index"] for parent, key in fields(doc.get("materials", []), LINEAR_SLOTS)}
    configs = defaultdict(set)
    relevant = set()
    samplers = doc.get("samplers", [])
    images = doc.get("images", [])
    views = doc.get("bufferViews", [])
    data_by_image = {}
    for index, texture in enumerate(doc.get("textures", [])):
        # The fallback image also needs to preserve its settings for other glTF consumers.
        sources = {texture.get("source")}
        for extension in texture.get("extensions", {}).values():
            if "source" in extension:
                sources.add(extension["source"])
        sources.discard(None)
        sampler_index = texture.get("sampler")
        sampler = samplers[sampler_index] if sampler_index is not None else None
        for source in sources:
            image = images[source]
            mime = image.get("mimeType")
            if "bufferView" not in image or mime not in MIME_SUFFIX:
                continue
            if source not in data_by_image:
                span = view_span(views[image["bufferView"]], blob)
                data_by_image[source] = blob[span[0]:span[1]] if span else b""
            data = data_by_image[source]
            if not data or (mime == "image/ktx2" and not externalizable_ktx2(data)):
                continue
            configs[source].add(shared_path(pool, data, mime, index in linear, sampler))
            if index in used:
                relevant.add(source)
    for source, paths in configs.items():
        if source in relevant and len(paths) == 1:
            yield source, next(iter(paths)), data_by_image[source]


def externalize(doc, blob, model, selected):
    views = doc["bufferViews"]
    dropped = set()
    for index, target, _ in selected:
        image = doc["images"][index]
        dropped.add(image.pop("bufferView"))
        image["uri"] = image_uri(model, target)

    references = list(fields(doc, {"bufferView"}))
    dropped.difference_update(parent[key] for parent, key in references)
    remap = {}
    kept = []
    for index, view in enumerate(views):
        if index not in dropped:
            remap[index] = len(kept)
            kept.append(view)
    doc["bufferViews"] = kept
    for parent, key in references:
        parent[key] = remap[parent[key]]

    live_blocks = kept + [parent[key] for parent, key in fields(doc, MESHOPT_EXTENSIONS)]
    spans = [(block, span) for block in live_blocks if (span := view_span(block, blob)) is not None]
    removed = []
    for index in sorted(dropped, key=lambda index: views[index].get("byteOffset", 0)):
        start, end = view_span(views[index], blob)
        if any(start < right and left < end for _, (left, right) in spans):
            continue
        end -= (end - start) % 4  # Preserve alignment of all following accessors.
        if end > start and (not removed or start >= removed[-1][1]):
            removed.append((start, end))

    limit = doc["buffers"][0]["byteLength"]
    chunks = []
    ends = []
    offsets = [0]
    previous = 0
    for start, end in removed:
        chunks.append(blob[previous:start])
        ends.append(end)
        offsets.append(offsets[-1] + end - start)
        previous = end
    chunks.append(blob[previous:limit])
    compacted = b"".join(chunks)
    for block, (start, end) in spans:
        offset = start - offsets[bisect_right(ends, start)]
        if blob[start:end] != compacted[offset:offset + end - start]:
            raise ValueError(f"Live buffer bytes changed in {model}")
        if offset != start:
            block["byteOffset"] = offset
    doc["buffers"][0]["byteLength"] = len(compacted)
    return compacted


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pool", type=Path, default=MESHES)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--backup", type=Path)
    args = parser.parse_args()
    pool = args.pool.resolve()
    models = sorted(path for path in pool.rglob("*.glb") if "_intermediate" not in path.relative_to(pool).parts)
    paths_by_model = {}
    counts = defaultdict(int)
    for model in models:
        _, chunks = read_glb(model)
        doc, blob = parts(chunks)
        if not doc.get("buffers") or "uri" in doc["buffers"][0]:
            continue
        items = [(index, texture, len(data)) for index, texture, data in candidates(pool, doc, blob)]
        if items:
            paths_by_model[model] = items
        for _, texture, _ in items:
            counts[texture] += 1

    targets = {path for path, count in counts.items() if count > 1 or path.is_file()}
    models = [model for model, items in paths_by_model.items() if any(path in targets for _, path, _ in items)]
    count = sum(counts[path] for path in targets)
    embedded_size = sum(length for model in models for _, path, length in paths_by_model[model] if path in targets)
    print(f"{len(models)} GLBs, {count} embedded images, {len(targets)} shared paths, {embedded_size:,} embedded bytes", flush=True)
    if not args.apply or not models:
        return

    backup = args.backup
    if backup is None:
        archive = Path.home() / ".local/share/vael-asset-archive"
        archive.mkdir(parents=True, exist_ok=True)
        backup = Path(tempfile.mkdtemp(prefix="shared-glb-textures-", dir=archive))
    print(f"GLB originals: {backup}", flush=True)
    model_savings = 0
    new_texture_size = 0
    for model in models:
        version, chunks = read_glb(model)
        doc, blob = parts(chunks)
        selected = [item for item in candidates(pool, doc, blob) if item[1] in targets]
        compacted = externalize(doc, blob, model, selected)
        original = backup / model.relative_to(pool)
        original.parent.mkdir(parents=True, exist_ok=True)
        if not original.exists():
            shutil.copy2(model, original)
        for _, path, data in selected:
            if not path.exists():
                new_texture_size += len(data)
            write_shared(path, data)
        encoded = json.dumps(doc, separators=(",", ":"), allow_nan=False).encode()
        old_size = model.stat().st_size
        write_glb(model, version, [(kind, {JSON_CHUNK: encoded, BIN_CHUNK: compacted}.get(kind, data)) for kind, data in chunks])
        model_savings += old_size - model.stat().st_size
    print(f"Net reduction: {model_savings - new_texture_size:,} bytes")


if __name__ == "__main__":
    main()
