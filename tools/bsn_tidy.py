#!/usr/bin/env python3
"""Organiza os filhos de topo de um mundo `.bsn` em pastas do Outliner.

As pastas são entidades sem `WorldNode` (só `Transform` + filhos): o loader
do jogo trata-as como transparentes (os filhos sobem ao nível da pasta), por
isso o mundo que corre é o mesmo — só a árvore do Jackdaw muda. Junta também
a pasta **Definições** com as instâncias de `world/settings/*.bsn`.

As pastas e as definições são nós novos: levam um `SceneNodeId` determinístico
(acima do maior id do ficheiro, na faixa esparsa do Jackdaw) para o editor as
poder endereçar nas edições ao vivo do Play.

Uso:  python3 tools/bsn_tidy.py world.bsn            # reescreve no sítio
      python3 tools/bsn_tidy.py world.bsn --dry-run  # mostra as pastas

Idempotente: um mundo já organizado (com pastas) não é tocado.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

CHILDREN = "bevy_ecs::hierarchy::Children [\n"
SEP = re.compile(r"^    --\n", re.M)

# O Jackdaw cunha `SceneNodeId` em [SPARSE_MIN, SPARSE_MAX) (aleatório por
# sessão) e o emissor grava-o numa linha própria, depois dos componentes do nó
# e antes do `Children`. As pastas são nós novos: sem id o editor não as
# consegue endereçar nas edições ao vivo do Play.
NODE_ID_RE = re.compile(r"jackdaw_scene_types::node_id::SceneNodeId\((\d+)\)")
SPARSE_MIN = 1 << 32
SPARSE_MAX = 1 << 63
# Base de cunhagem quando o ficheiro ainda não tem ids nenhuns (mundo vindo de
# outro sítio). Fixa para o mesmo mundo dar sempre os mesmos ids.
BASE_NODE_ID = 0x5000_0000_0000_0000

# Pasta por prefixo de `IsA.source` (o primeiro que casar ganha).
SOURCE_FOLDERS: list[tuple[str, str]] = [
    ("world/environment", "Mundo base"),
    ("world/atmosphere/", "Mundo base"),
    ("world/cities/", "Cidades"),
    ("world/spawn/", "Acampamento inicial"),
    ("world/paths/", "Caminhos e pontes"),
    ("world/bridges/", "Caminhos e pontes"),
    ("world/vegetation/", "Vegetação"),
    ("world/landmarks/", "Marcos"),
    ("world/frontier/", "Relevo e fronteira"),
    ("world/landforms/", "Relevo e fronteira"),
    ("world/caves/", "Relevo e fronteira"),
    ("world/fantasy/", "Fantasia"),
    ("world/scenes/", "Cenários"),
    ("world/creatures/", "Criaturas e NPCs"),
    ("world/ai/", "Criaturas e NPCs"),
    ("world/", "Jogo"),
]

# Pasta por tag para os nós que não são includes.
TAG_FOLDERS = {
    "ProjectileTemplate": "Jogo",
    "SpawnGate": "Mundo base",
    "SceneEntry": "Mundo base",
    "Scatter": "Acampamento inicial",
}

FOLDER_ORDER = [
    "Definições",
    "Mundo base",
    "Cidades",
    "Acampamento inicial",
    "Caminhos e pontes",
    "Vegetação",
    "Marcos",
    "Relevo e fronteira",
    "Fantasia",
    "Cenários",
    "Criaturas e NPCs",
    "Jogo",
    "Outros",
]

SETTINGS = [
    ("Ambiente", "world/settings/environment.bsn"),
    ("Pós-processamento", "world/settings/postfx.bsn"),
    ("Flora", "world/settings/flora.bsn"),
    ("Relva", "world/settings/grass.bsn"),
]


def split_root(text: str) -> tuple[str, list[str], str]:
    """Cabeça do nó raiz, blocos dos filhos de topo e o fecho."""
    start = text.index(CHILDREN)
    # O `[` de fecho dos filhos da raiz é o último `]` na coluna 0.
    end = text.rstrip().rindex("\n]") + 1
    head = text[: start + len(CHILDREN)]
    body = text[start + len(CHILDREN) : end]
    tail = text[end:]
    blocks = [block for block in SEP.split(body) if block.strip()]
    return head, blocks, tail


def block_name(block: str) -> str:
    match = re.match(r'\s*#(?:"((?:[^"\\]|\\.)*)"|(\S+))', block)
    if not match:
        return "?"
    return match.group(1) or match.group(2)


def folder_of(block: str) -> str:
    # O `IsA` tanto sai numa linha (`IsA { source: "…", deleted: [] }`) como
    # no formato multilinha que o Jackdaw grava (`IsA {\n    source: "…",`).
    source = re.search(r'IsA \{\s*source: "([^"]+)"', block)
    if source:
        for prefix, folder in SOURCE_FOLDERS:
            if source.group(1).startswith(prefix):
                return folder
    tag = re.search(r'^    \s*tag: "([^"]+)"', block, re.M)
    if tag:
        if tag.group(1) in TAG_FOLDERS:
            return TAG_FOLDERS[tag.group(1)]
        if tag.group(1) == "Group":
            name = block_name(block)
            if name.startswith("spawn/"):
                return "Acampamento inicial"
            return "Mundo base"
    return "Outros"


def is_folder(block: str) -> bool:
    """Já é uma pasta (sem `WorldNode` próprio e com filhos)."""
    own = block.split(CHILDREN.strip(), 1)[0]
    return "WorldNode" not in own and "IsA" not in own and CHILDREN.strip() in block


def indent(block: str, levels: int = 1) -> str:
    pad = "    " * levels
    return "".join(pad + line if line.strip() else line for line in block.splitlines(True))


def node_id_line(node_id: int) -> str:
    return f"    jackdaw_scene_types::node_id::SceneNodeId({node_id})\n"


def folder_block(name: str, children: list[str], node_id: int) -> str:
    inner = "        --\n".join(indent(child) for child in children)
    return (
        f'    #"{name}"\n'
        "    bevy_transform::components::transform::Transform\n"
        f"{node_id_line(node_id)}"
        f"    {CHILDREN}"
        f"{inner}"
        "    ]\n"
    )


def settings_block(name: str, source: str, node_id: int) -> str:
    return (
        f'    #"{name}"\n'
        "    bevy_transform::components::transform::Transform\n"
        f'    jackdaw::prefab::components::IsA {{ source: "{source}", deleted: [] }}\n'
        f"{node_id_line(node_id)}"
    )


def mint_node_ids(count: int, text: str) -> list[int]:
    """`count` ids determinísticos, acima dos que o ficheiro já usa.

    Continuar a partir do maior id existente mantém o ficheiro num só "range"
    (o mesmo que uma sessão do editor faria) e nunca colide; sem ids, a base é
    fixa. Ids mintados têm de estar na faixa esparsa (`>= 2^32`), como os do
    `SceneNodeId::next`.
    """
    taken = {int(match.group(1)) for match in NODE_ID_RE.finditer(text)}
    value = max(taken) + 1 if taken else BASE_NODE_ID
    out: list[int] = []
    while len(out) < count:
        if value >= SPARSE_MAX:
            value = SPARSE_MIN
        if value not in taken:
            taken.add(value)
            out.append(value)
        value += 1
    return out


def unique_names(children: list[str]) -> list[str]:
    """Nomes repetidos dentro da mesma pasta ganham o nome do ficheiro pai."""
    seen: dict[str, int] = {}
    for child in children:
        seen[block_name(child)] = seen.get(block_name(child), 0) + 1
    out = []
    for child in children:
        name = block_name(child)
        source = re.search(r'IsA \{\s*source: "([^"]+)"', child)
        if seen[name] > 1 and source:
            parent = Path(source.group(1)).parent.name
            child = child.replace(f'#"{name}"', f'#"{parent}/{name}"', 1)
        out.append(child)
    return out


def tidy(text: str, root: Path) -> tuple[str, dict[str, list[str]]]:
    head, blocks, tail = split_root(text)
    if any(is_folder(block) for block in blocks):
        return text, {}
    settings = [(name, source) for name, source in SETTINGS if (root / source).is_file()]
    folders: dict[str, list[str]] = {name: [] for name in FOLDER_ORDER}
    for block in blocks:
        folders[folder_of(block)].append(block if block.endswith("\n") else block + "\n")
    emitted = [
        (name, children)
        for name, children in folders.items()
        if children or (name == "Definições" and settings)
    ]
    # Um id por nó novo (as pastas e cada `IsA` de definições), na ordem em
    # que saem — o mesmo mundo dá sempre os mesmos ids.
    minted = iter(mint_node_ids(len(emitted) + len(settings), text))
    final: dict[str, list[str]] = {}
    for name, children in emitted:
        if name == "Definições":
            # As entradas de definições abrem a pasta.
            children = [
                settings_block(entry, source, next(minted))
                for entry, source in settings
            ] + children
        final[name] = children
    out_blocks = [
        folder_block(name, unique_names(children), next(minted))
        for name, children in final.items()
    ]
    return head + "    --\n".join(out_blocks) + tail, {
        name: [block_name(child) for child in children]
        for name, children in final.items()
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("world", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if not args.world.is_file():
        print(f"erro: {args.world} não existe", file=sys.stderr)
        return 1
    text = args.world.read_text(encoding="utf-8")
    try:
        new, layout = tidy(text, args.world.resolve().parent)
    except ValueError:
        # `split_root` espera a raiz `<world>` com um `Children [` de topo e o
        # `]` de fecho na coluna 0; outro `.bsn` (módulo, cena) não é mundo.
        print(f"erro: {args.world} não é um mundo (sem `Children [` de raiz)", file=sys.stderr)
        return 1
    if not layout:
        print(f"{args.world}: já organizado")
        return 0
    for folder, names in layout.items():
        print(f"{folder}: {', '.join(names)}")
    if not args.dry_run:
        args.world.write_text(new, encoding="utf-8")
        print(f"{args.world}: reescrito")
    return 0


if __name__ == "__main__":
    sys.exit(main())
