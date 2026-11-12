#!/usr/bin/env python3
"""Migra as árvores GLB do mundo para estações procedurais (`bevy_flora`).

Para cada elemento de visual cujo `url` é uma árvore do pool:

* **instância** (`<GltfScene … translation=…>`, o que os scatters bakeados
  escrevem), vira `<Entity … />`: mantém `name`/`translation`/`euler`/`scale`
  e os props (`destructible`, `resource-kind`, …), perde `url`/`lod*` e ganha
  `collider="shape: cylinder; …"` + `prop-flora="<espécie>"`;
* **visual filho** de `<Entity>`/`<GameObject>` (sem `translation`), é apagado
  e o pai ganha o collider cilindro + `prop-flora` (o pai fica `… />` se não
  tiver mais filhos).

As **receitas** de `<Scatter>` (`<GLTFLoader role="visual">` dentro de um
`<Scatter>`) ficam intactas: um `viber tool scatter` re-bake ainda precisa
delas. Depois de um re-scatter, voltar a correr esta migração.

A pose fica no XML (o runtime lê a estação); a árvore é plantada por
`src/flora.rs` com pools por espécie.

Uso:
    python3 tools/flora_migrate.py            # dry-run: só relata
    python3 tools/flora_migrate.py --apply    # escreve
    python3 tools/flora_migrate.py --check    # exit 1 se ainda houver árvores GLB

Idempotente: depois de migrado não há urls de árvore para encontrar.
"""

from __future__ import annotations

import argparse
import glob
import os
import re
import sys

# Espécie (stem do GLB) -> (nome, raio do tronco, meia-altura do cilindro).
TREES: dict[str, tuple[str, float, float]] = {
    "/assets/meshes/forest/pine_dark_lod0.glb": ("pine_dark", 0.45, 2.2),
    "/assets/meshes/forest/tree_pine_lod0.glb": ("tree_pine", 0.45, 2.2),
    "/assets/meshes/forest/tree_oak_lod0.glb": ("tree_oak", 0.55, 2.2),
    "/assets/meshes/farm/tree_birch_lod0.glb": ("tree_birch", 0.35, 2.2),
    "/assets/meshes/forest/dead_tree_lod0.glb": ("dead_tree", 0.40, 2.2),
    "/assets/meshes/swamp/dead_willow_lod0.glb": ("dead_willow", 0.45, 2.2),
    "/assets/meshes/props/palm_tree_lod0.glb": ("palm_tree", 0.45, 2.2),
    # As quatro da praça já foram migradas à mão; ficam aqui para o --check.
}

# Atributos que só fazem sentido no visual GLB.
DROP_ATTRS = {
    "url",
    "lod1-url",
    "lod2-url",
    "lod-threshold-near",
    "lod-threshold-mid",
    "lod-threshold-far",
}

WORLD_GLOBS = ["world.xml", "world/**/*.xml", "scenes/menu.xml"]

TAG_RE = re.compile(
    r"<(?P<close>/?)(?P<tag>[A-Za-z][\w:.-]*)"
    r"(?P<attrs>(?:[^<>\"']|\"[^\"]*\"|'[^']*')*?)"
    r"(?P<self>/?)>",
    re.S,
)
ATTR_RE = re.compile(r"([\w:.-]+)\s*=\s*(\"[^\"]*\"|'[^']*')", re.S)


def attrs_of(text: str) -> dict[str, str]:
    return {m.group(1): m.group(2)[1:-1] for m in ATTR_RE.finditer(text)}


class Node:
    __slots__ = ("tag", "start", "open_end", "end", "attrs_text", "attrs", "children", "parent", "self_closing")

    def __init__(self, tag, start, open_end, attrs_text, attrs, self_closing, parent):
        self.tag = tag
        self.start = start
        self.open_end = open_end
        self.end = open_end
        self.attrs_text = attrs_text
        self.attrs = attrs
        self.self_closing = self_closing
        self.children: list[Node] = []
        self.parent = parent

    def text(self, source: str) -> str:
        return source[self.start : self.open_end]


def parse(text: str) -> tuple[list[Node], list[Node]]:
    """Árvore de elementos (sem tocar no texto) e a lista plana."""
    roots: list[Node] = []
    flat: list[Node] = []
    stack: list[Node] = []
    for match in TAG_RE.finditer(text):
        tag = match.group("tag")
        if match.group("close"):
            if stack and stack[-1].tag == tag:
                node = stack.pop()
                node.end = match.end()
            continue
        attrs_text = match.group("attrs")
        node = Node(
            tag,
            match.start(),
            match.end(),
            attrs_text,
            attrs_of(attrs_text),
            bool(match.group("self")),
            stack[-1] if stack else None,
        )
        flat.append(node)
        if node.parent is None:
            roots.append(node)
        else:
            node.parent.children.append(node)
        if not node.self_closing:
            stack.append(node)
    return roots, flat


def url_in(attrs: dict[str, str]) -> str | None:
    for key in ("url", "model-url", "gltf", "src"):
        candidate = attrs.get(key)
        if candidate in TREES:
            return candidate
    return None


def inside_scatter(node: Node) -> bool:
    parent = node.parent
    while parent is not None:
        if parent.tag == "Scatter":
            return True
        parent = parent.parent
    return False


def format_attr(key: str, value: str) -> str:
    return f'{key}="{value}"'


def rewrite_instance(node: Node, species: tuple[str, float, float]) -> str:
    """`<GltfScene … />` de instância -> `<Entity … />` estação."""
    name, radius, half_height = species
    kept: list[str] = []
    for key, value in node.attrs.items():
        if key in DROP_ATTRS or key == "collider":
            continue
        kept.append(format_attr(key, value))
    kept.append(format_attr("collider", f"shape: cylinder; radius: {radius}; half-height: {half_height}"))
    kept.append(format_attr("prop-flora", name))
    return "<Entity " + " ".join(kept) + " />"


def patch_open_tag(node: Node, species: tuple[str, float, float], self_close: bool) -> str:
    """Reescreve o tag de abertura: collider cilindro + `prop-flora`."""
    name, radius, half_height = species
    cleaned = re.sub(r"\s+(?:collider|prop-flora)\s*=\s*(\"[^\"]*\"|'[^']*')", "", node.attrs_text)
    extra = (
        " "
        + format_attr("collider", f"shape: cylinder; radius: {radius}; half-height: {half_height}")
        + " "
        + format_attr("prop-flora", name)
    )
    closing = " />" if self_close else ">"
    return f"<{node.tag}{cleaned}{extra}{closing}"


def line_span(text: str, start: int, end: int) -> tuple[int, int]:
    """Alarga o span para a linha inteira quando o elemento está sozinho nela."""
    line_start = text.rfind("\n", 0, start) + 1
    line_end = text.find("\n", end)
    line_end = len(text) if line_end == -1 else line_end
    if text[line_start:start].strip() == "" and text[end:line_end].strip() == "":
        return line_start, line_end + 1
    return start, end


def migrate_text(text: str, path: str, report: list[str]) -> tuple[str, int]:
    _roots, flat = parse(text)
    edits: list[tuple[int, int, str]] = []
    converted = 0

    for node in flat:
        url = url_in(node.attrs)
        if not url or node.tag not in ("GltfScene", "GLTFLoader") or inside_scatter(node):
            if url and inside_scatter(node):
                report.append(
                    f"{path}: receita de <Scatter> com {url.rsplit('/', 1)[-1]} (intacta)"
                )
            continue
        species = TREES[url]
        parent = node.parent
        parent_is_station = bool(
            parent
            and parent.tag in ("Entity", "GameObject")
            and ("collider" in parent.attrs or "destructible" in parent.attrs)
        )
        if "translation" in node.attrs and not parent_is_station:
            edits.append((node.start, node.end, rewrite_instance(node, species)))
            converted += 1
            continue

        # Visual filho: o pai (Entity/GameObject) fica com a estação.
        if parent is None or parent.tag not in ("Entity", "GameObject"):
            report.append(f"{path}: AVISO — {url} sem pai Entity/GameObject; ignorado")
            continue
        remaining = [child for child in parent.children if child is not node]
        if not remaining:
            # O pai fica vazio: um só edit no span inteiro do pai.
            edits.append((parent.start, parent.end, patch_open_tag(parent, species, True)))
        else:
            start, end = line_span(text, node.start, node.end)
            edits.append((start, end, ""))
            edits.append((parent.start, parent.open_end, patch_open_tag(parent, species, False)))
        converted += 1
        report.append(f"{path}: {url.rsplit('/', 1)[-1]} -> estação ({parent.tag})")

    if not edits:
        return text, converted
    edits.sort(key=lambda item: item[0], reverse=True)
    out = text
    last_start = len(text) + 1
    for start, end, replacement in edits:
        if end > last_start:
            raise RuntimeError(f"{path}: edits sobrepostos em {start}..{end}")
        out = out[:start] + replacement + out[end:]
        last_start = start
    return out, converted


def count_unmigrated(text: str) -> int:
    """Árvores GLB que o runtime ainda desenharia (fora de receitas de Scatter)."""
    _roots, flat = parse(text)
    return sum(
        1
        for node in flat
        if url_in(node.attrs)
        and node.tag in ("GltfScene", "GLTFLoader")
        and not inside_scatter(node)
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply", action="store_true", help="escreve os ficheiros")
    parser.add_argument("--check", action="store_true", help="exit 1 se ainda houver árvores GLB")
    parser.add_argument("--root", default=".", help="raiz do projeto (default: .)")
    args = parser.parse_args()

    paths: list[str] = []
    for pattern in WORLD_GLOBS:
        paths.extend(glob.glob(os.path.join(args.root, pattern), recursive=True))
    paths = sorted({p for p in paths if not p.endswith(".bak")})

    total = 0
    remaining: list[str] = []
    for path in paths:
        text = open(path, encoding="utf-8").read()
        if not any(url in text for url in TREES):
            continue
        report: list[str] = []
        migrated, converted = migrate_text(text, path, report)
        for line in report:
            print("  " + line)
        if converted:
            total += converted
            print(f"{path}: {converted} árvore(s) -> estação")
            if args.apply:
                with open(path, "w", encoding="utf-8") as handle:
                    handle.write(migrated)
        if count_unmigrated(migrated):
            remaining.append(path)

    if args.check:
        pendentes = total + sum(count_unmigrated(open(p, encoding="utf-8").read()) for p in remaining)
        if pendentes:
            print(f"\nAINDA HÁ ÁRVORES GLB ({pendentes} pendentes)", file=sys.stderr)
            for path in remaining:
                print("  " + path, file=sys.stderr)
            return 1
        print("ok: nenhuma árvore GLB no mundo (fora das receitas de <Scatter>)")
        return 0

    mode = "aplicado" if args.apply else "dry-run (use --apply)"
    print(f"\n{total} árvore(s) convertidas — {mode}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
