#!/usr/bin/env python3
"""Troca as cercas de GLB do mundo por cercas procedurais (`bevy_ext_fence`).

Cada cerca era o `farm/fence_segment` repetido a cada ~3 m (ou um
`<Use prototype="cerca">`, ou o `fence_gate`). Aqui cada **linha** de segmentos
colineares vira um `<Entity>` com `FenceSpec`: `fence-points` é a polilinha no
espaço do pai e `fence-style` a predefinição. As linhas que se encontram nas
pontas juntam-se numa só polilinha — e fecham num perímetro (`fence-closed`)
quando dão a volta: um poste de canto em vez de dois postes sobrepostos. O
portão vira uma cerca `gate` com o mesmo nome.

O `.bsn` é RON do Jackdaw: os nós são elementos de uma lista `Children [ … ]`
separados por `--`. A cirurgia respeita a profundidade de parênteses (não a
indentação, que o emissor não normaliza) e só mexe nas listas que tinham cercas.

Uso:
    python3 tools/fence_migrate.py            # relata o que encontraria
    python3 tools/fence_migrate.py --apply    # escreve
    python3 tools/fence_migrate.py --check    # exit 1 se ainda houver cercas de GLB

Não é idempotente: os nós antigos deixam de existir. Para repetir ou corrigir,
`git checkout -- <ficheiro>.bsn` e voltar a correr.
"""

from __future__ import annotations

import argparse
import math
import os
import random
import re
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Onde há cercas de GLB (2026-11-23): (ficheiro, predefinição).
SITES: list[tuple[str, str]] = [
    ("world/cities/discordia/walls.bsn", "pasture_rail"),
    ("world/cities/discordia/approach.bsn", "pasture_rail"),
    ("world/quest-stations.bsn", "pasture_rail"),
    # A paliçada dos contrabandistas era um segmento de cerca a escala 2,2
    # (a geração do GLB tinha falhado): agora é uma paliçada de verdade.
    ("world/scenes/smugglers.bsn", "palisade"),
    ("scenes/interiors/barn.bsn", "pasture_rail"),
]

# Os modelos antigos; o comprimento é o do eixo longo (`_precompute.json`).
PREFAB = "world/prefabs/farm/fence-segment.bsn"
SEGMENT_URL = "/assets/meshes/farm/fence_segment_lod0.glb"
GATE_URL = "/assets/meshes/farm/fence_gate_lod0.glb"
SEGMENT_LENGTH = 2.9932  # x de -1,4967 a 1,4965
GATE_LENGTH = 2.6027  # x de -1,3015 a 1,3012

# Duas cercas estão na mesma reta se os pés ficam a menos distância dela do que
# isto; um vão maior do que `GAP × passo` parte a linha (ali está um portão).
COLLINEAR_M = 0.3
PARALLEL_COS = math.cos(math.radians(6.0))
GAP = 1.6
LINK = 1.4  # vizinhos até `LINK × comprimento` de um segmento
# Uma linha que acaba a menos distância do que isto de um poste de portão
# encosta nele.
GATE_SNAP_M = 1.2
# Pontas a menos distância do que isto são o mesmo canto: até um segmento em
# falta (o layout antigo deixava ~3 m de buraco em três cantos do curral).
CORNER_M = 3.3

NAME_RE = re.compile(r'^#"([^"]*)"$')
TAG_RE = re.compile(r"^#([A-Za-z][\w.]*)$")
TRANSFORM = "bevy_transform::components::transform::Transform"
VEC3_RE = re.compile(r"(translation|scale): glam::Vec3 \{ x: ([-\d.e]+), y: ([-\d.e]+), z: ([-\d.e]+) \}")
QUAT_RE = re.compile(r"rotation: glam::Quat \{ x: ([-\d.e]+), y: ([-\d.e]+), z: ([-\d.e]+), w: ([-\d.e]+) \}")
ISA_RE = re.compile(r'jackdaw::prefab::components::IsA \{\s*source: "([^"]+)"', re.S)
NODE_RE = re.compile(r'bevy_world_xml::bsn::WorldNode \{\s*tag: "([^"]+)",\s*attrs: \[(.*?)\],\s*text:', re.S)
PROP_RE = re.compile(r'WorldProp \{ key: "([^"]*)", value: "([^"]*)" \}')
SCENE_ID_RE = re.compile(r"jackdaw_scene_types::node_id::SceneNodeId\((\d+)\)")
CHILDREN = "bevy_ecs::hierarchy::Children ["


# --------------------------------------------------------------------- matemática


def identity() -> list[list[float]]:
    return [[1.0 if i == j else 0.0 for j in range(4)] for i in range(4)]


def compose(a: list[list[float]], b: list[list[float]]) -> list[list[float]]:
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def apply(m: list[list[float]], p: tuple[float, float, float]) -> tuple[float, float, float]:
    x, y, z = p
    return tuple(m[i][0] * x + m[i][1] * y + m[i][2] * z + m[i][3] for i in range(3))  # type: ignore[return-value]


def trs(translation, rotation, scale) -> list[list[float]]:
    tx, ty, tz = translation
    qx, qy, qz, qw = rotation
    rot = [
        [1 - 2 * (qy * qy + qz * qz), 2 * (qx * qy - qz * qw), 2 * (qx * qz + qy * qw)],
        [2 * (qx * qy + qz * qw), 1 - 2 * (qx * qx + qz * qz), 2 * (qy * qz - qx * qw)],
        [2 * (qx * qz - qy * qw), 2 * (qy * qz + qx * qw), 1 - 2 * (qx * qx + qy * qy)],
    ]
    m = identity()
    for i in range(3):
        for j in range(3):
            m[i][j] = rot[i][j] * scale[j]
        m[i][3] = (tx, ty, tz)[i]
    return m


def invert(m: list[list[float]]) -> list[list[float]]:
    a = [row[:] + [1.0 if i == j else 0.0 for j in range(4)] for i, row in enumerate(m)]
    for col in range(4):
        pivot = max(range(col, 4), key=lambda r: abs(a[r][col]))
        if abs(a[pivot][col]) < 1e-12:
            raise ValueError("transform sem inversa (escala nula)")
        a[col], a[pivot] = a[pivot], a[col]
        a[col] = [value / a[col][col] for value in a[col]]
        for row in range(4):
            if row != col:
                factor = a[row][col]
                a[row] = [v - factor * w for v, w in zip(a[row], a[col])]
    return [row[4:] for row in a]


def num(value: float) -> str:
    text = f"{value:.4f}".rstrip("0").rstrip(".")
    if "." not in text:
        text += ".0"
    return "0.0" if text == "-0.0" else text


def heading(dx: float, dz: float) -> str:
    """O ponto cardeal de uma direcção no chão (em Discórdia +z é norte)."""
    if abs(dz) >= abs(dx):
        return "n" if dz > 0 else "s"
    return "e" if dx > 0 else "w"


# ------------------------------------------------------------------------ árvore


class Node:
    """Um elemento de uma lista `Children [ … ]`."""

    def __init__(self, start: int, end: int, depth: int):
        self.start = start
        self.end = end
        self.depth = depth
        self.label: str | None = None
        self.kind: str | None = None
        self.tag: str | None = None  # a tag do `WorldNode` (o rótulo `#"…"` pode ser um nome)
        self.attrs: dict[str, str] = {}
        self.isa: str | None = None
        self.transform = ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0, 1.0), (1.0, 1.0, 1.0))
        self.world = identity()
        self.children: list[Node] = []
        self.parent: Node | None = None
        self.open = -1  # linha do `Children [`
        self.close = -1  # linha do `]` que o fecha

    def __repr__(self) -> str:  # só para o relatório
        return f"{self.label or self.kind or '?'}@{self.start + 1}"


def bracket_delta(line: str) -> int:
    delta = 0
    quoted = escaped = False
    for char in line:
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
            continue
        if char == '"':
            quoted = True
        elif char in "[{(":
            delta += 1
        elif char in "]})":
            delta -= 1
    return delta


def parse(text: str) -> list[Node]:
    """A árvore de elementos do documento (raízes = elementos de topo)."""
    lines = text.split("\n")
    count = len(lines)
    deltas = [bracket_delta(line) for line in lines]
    depth_at = [0] * count
    is_list = [False] * count  # a profundidade da linha é uma lista de filhos
    depth = 0
    list_depth = [True]  # por profundidade: é uma lista `Children [`?
    for index in range(count):
        depth_at[index] = depth
        is_list[index] = depth < len(list_depth) and list_depth[depth]
        delta = deltas[index]
        if delta > 0:
            while len(list_depth) <= depth + delta:
                list_depth.append(False)
            # Cada nível que a linha abre é lista de filhos só se ela abre `Children [`.
            opens_list = CHILDREN in lines[index]
            for level in range(depth + 1, depth + delta + 1):
                list_depth[level] = opens_list and level == depth + delta
        depth += delta

    elements: list[Node] = []
    current: dict[int, int] = {}
    for index in range(count):
        here = depth_at[index]
        if deltas[index] < 0:
            # Um fecho termina o elemento da lista que fecha.
            for level in [level for level in current if level >= here]:
                elements.append(Node(current.pop(level), index - 1, level))
        # Um fecho nunca começa um elemento (o `]` pertence à lista que fecha).
        if is_list[index] and deltas[index] >= 0:
            if lines[index].strip() == "--":
                if here in current:
                    elements.append(Node(current.pop(here), index - 1, here))
            else:
                current.setdefault(here, index)
    for level, start in current.items():
        elements.append(Node(start, count - 1, level))
    elements.sort(key=lambda node: node.start)

    stack: list[Node] = []
    roots: list[Node] = []
    for node in elements:
        while stack and stack[-1].end < node.start:
            stack.pop()
        if stack:
            node.parent = stack[-1]
            stack[-1].children.append(node)
        else:
            roots.append(node)
        stack.append(node)

    for node in elements:
        _read(node, lines, depth_at, deltas)
    return roots


def _read(node: Node, lines: list[str], depth_at: list[int], deltas: list[int]) -> None:
    chunk = "\n".join(lines[node.start : node.end + 1])
    first = lines[node.start].strip()
    if match := NAME_RE.match(first):
        node.label = match.group(1)
    elif match := TAG_RE.match(first):
        node.kind = match.group(1)
    # O corpo do nó é o que vem antes do seu `Children [`; os filhos são outros.
    body, separator, _ = chunk.partition(CHILDREN)
    if TRANSFORM in body and "{" in body.split(TRANSFORM, 1)[1].split("\n", 1)[0]:
        translation, rotation, scale = node.transform
        for key, x, y, z in VEC3_RE.findall(body):
            values = (float(x), float(y), float(z))
            if key == "translation":
                translation = values
            else:
                scale = values
        if quat := QUAT_RE.search(body):
            rotation = tuple(float(value) for value in quat.groups())
        node.transform = (translation, rotation, scale)
    if isa := ISA_RE.search(body):
        node.isa = isa.group(1)
    if world := NODE_RE.search(body):
        node.tag = world.group(1)
        node.attrs = {key: value for key, value in PROP_RE.findall(world.group(2))}
        # Alguns nós têm o `Transform` desactualizado e a rotação só nos
        # atributos (`euler="0 90 0"`): é a intenção autorada que conta.
        rotation = _authored_rotation(node.attrs)
        if rotation and _angle(rotation) > _angle(node.transform[1]):
            node.transform = (node.transform[0], rotation, node.transform[2])
    node.world = compose(node.parent.world, trs(*node.transform)) if node.parent else trs(*node.transform)
    if separator:
        node.open = node.start + body.count("\n")
        node.close = next(
            index
            for index in range(node.open + 1, node.end + 2)
            if depth_at[index] == node.depth + 1 and deltas[index] < 0
        )


def tag_of(node: Node) -> str | None:
    """A tag do nó: a do `WorldNode`, ou, sem ela, a do rótulo `#Tag`."""
    return node.tag or node.kind


def _authored_rotation(attrs: dict[str, str]) -> tuple[float, float, float, float] | None:
    """A rotação em Y dos atributos `euler`/`transform: rotation` (graus)."""
    text = attrs.get("euler", "")
    if not text:
        for part in attrs.get("transform", "").split(";"):
            key, _, value = part.partition(":")
            if key.strip() == "rotation":
                text = value
    values = text.split()
    if len(values) != 3:
        return None
    try:
        yaw = math.radians(float(values[1]))
    except ValueError:
        return None
    return (0.0, math.sin(yaw / 2), 0.0, math.cos(yaw / 2))


def _angle(quaternion) -> float:
    return 2.0 * math.acos(max(-1.0, min(1.0, abs(quaternion[3]))))


def walk(roots: list[Node]):
    stack = list(roots)
    while stack:
        node = stack.pop()
        yield node
        stack.extend(reversed(node.children))


def ancestors(node: Node):
    while node.parent:
        node = node.parent
        yield node


# -------------------------------------------------------------------- as cercas


class Unit:
    """Uma cerca antiga: o nó que sai inteiro (o `Entity` com o GLB, ou o `Use`)."""

    def __init__(self, node: Node, center: tuple[float, float, float], direction: tuple[float, float], length: float, gate: bool):
        self.node = node
        self.center = center
        self.direction = direction
        self.length = length
        self.gate = gate


class Run:
    """Uma polilinha (ou perímetro) de cercas antigas, com os pontos no mundo."""

    def __init__(self, units: list[Unit]):
        self.units = units
        self.points: list[tuple[float, float]] = []
        self.closed = False
        self.name = ""

    @property
    def gate(self) -> bool:
        return self.units[0].gate


def _legacy(node: Node) -> str | None:
    """`segment`, `gate` ou None."""
    url = node.attrs.get("url", "")
    if node.isa == PREFAB or url == SEGMENT_URL:
        return "segment"
    if url == GATE_URL or "fence_gate" in (node.isa or ""):
        return "gate"
    return None


def find_units(roots: list[Node]) -> tuple[list[Unit], list[Node]]:
    """As cercas antigas do mundo e os protótipos que só as definem."""
    prototypes = {
        node.attrs.get("id", "")
        for node in walk(roots)
        if tag_of(node) == "Prototype" and any(_legacy(inner) for inner in walk(node.children))
    }
    units: list[Unit] = []
    for node in walk(roots):
        if any(tag_of(ancestor) == "Prototype" for ancestor in ancestors(node)):
            continue  # o conteúdo de um protótipo não é uma cerca no mundo
        kind = _legacy(node)
        if not kind and tag_of(node) == "Use" and node.attrs.get("prototype") in prototypes:
            kind = "segment"
        if not kind:
            continue
        # O GLB vive num filho de um `Entity` que não tem mais nada: sai o
        # Entity inteiro (o filho de uma instância de prefab nem tag tem).
        owner = node
        if node.parent and len(node.parent.children) == 1 and not _legacy(node.parent):
            owner = node.parent
        scale_x = math.sqrt(sum(owner.world[i][0] ** 2 for i in range(3)))
        direction = (owner.world[0][0] / scale_x, owner.world[2][0] / scale_x)
        length = (GATE_LENGTH if kind == "gate" else SEGMENT_LENGTH) * scale_x
        units.append(Unit(owner, apply(owner.world, (0.0, 0.0, 0.0)), direction, length, kind == "gate"))
    stale = [node for node in walk(roots) if tag_of(node) == "Prototype" and node.attrs.get("id") in prototypes]
    return units, stale


def gap(a: Unit, b: Unit) -> float:
    return math.hypot(a.center[0] - b.center[0], a.center[2] - b.center[2])


def chain_neighbours(units: list[Unit], perpendicular: bool = False) -> list[list[Unit]]:
    """Encadeia vizinhos de uma mesma linha (ou, com `perpendicular`, de uma
    mesma faixa transversal — os «pentes»).

    Duas cercas seguidas estão à distância de um segmento (no máximo
    `LINK × comprimento`), paralelas e encostadas: a distância ao eixo da
    outra é no máximo `COLLINEAR_M`. Medir contra uma recta de referência não
    serve — cada segmento tem ~1° de guinada ao acaso e, a 50 m da origem, isso
    muda a distância ao eixo em ~1 m —; entre vizinhos (3 m) são 5 cm.
    """
    count = len(units)
    links: list[list[int]] = [[] for _ in units]

    def axis(unit: Unit) -> tuple[float, float]:
        return (-unit.direction[1], unit.direction[0]) if perpendicular else unit.direction

    for i in range(count):
        a = units[i]
        da = axis(a)
        for j in range(i + 1, count):
            b = units[j]
            db = axis(b)
            dx, dz = b.center[0] - a.center[0], b.center[2] - a.center[2]
            if math.hypot(dx, dz) > LINK * max(a.length, b.length):
                continue
            if abs(da[0] * db[0] + da[1] * db[1]) < PARALLEL_COS:
                continue
            along = abs(dx * da[0] + dz * da[1])
            lateral = abs(da[0] * dz - da[1] * dx)
            reach = 0.4 * min(a.length, b.length) if not perpendicular else 0.5
            if lateral <= COLLINEAR_M and along >= reach:
                links[i].append(j)
                links[j].append(i)
    seen = [False] * count
    chains: list[list[Unit]] = []
    for seed in range(count):
        if seen[seed]:
            continue
        seen[seed] = True
        chain = [units[seed]]
        queue = [seed]
        while queue:
            for other in links[queue.pop()]:
                if not seen[other]:
                    seen[other] = True
                    chain.append(units[other])
                    queue.append(other)
        chains.append(chain)
    return chains


def straighten(chain: list[Unit]) -> list[list[Unit]]:
    """Ordena a cadeia ao longo do eixo e parte-a onde há um vão (um portão)."""
    head = chain[0].direction
    axis = [0.0, 0.0]
    for unit in chain:
        sign = 1.0 if unit.direction[0] * head[0] + unit.direction[1] * head[1] >= 0 else -1.0
        axis[0] += sign * unit.direction[0]
        axis[1] += sign * unit.direction[1]
    norm = math.hypot(*axis) or 1.0
    axis = [axis[0] / norm, axis[1] / norm]
    chain.sort(key=lambda unit: axis[0] * unit.center[0] + axis[1] * unit.center[2])
    gaps = [gap(a, b) for a, b in zip(chain, chain[1:])]
    pitch = statistics.median(gaps) if gaps else 0.0
    out: list[list[Unit]] = []
    run: list[Unit] = [chain[0]]
    for unit, step in zip(chain[1:], gaps):
        if pitch and step > GAP * pitch and step - pitch > 1.0:
            out.append(run)
            run = []
        run.append(unit)
    out.append(run)
    return out


def lines_of(units: list[Unit]) -> list[Run]:
    """As linhas de uma lista de cercas: as que se seguem numa reta e, para as
    que ficam sozinhas, as faixas transversais (segmentos de through encostados
    uns aos lado dos outros: uma parede vista de lado)."""
    runs: list[Run] = []
    leftover: list[Unit] = []
    for chain in chain_neighbours(units):
        if len(chain) == 1:
            leftover.append(chain[0])
            continue
        for group in straighten(chain):
            runs.append(stretch(group))
    for band in chain_neighbours(leftover, perpendicular=True):
        if len(band) == 1:
            runs.append(stretch(band))
            continue
        runs.append(stretch_band(band))
    return runs


def stretch(group: list[Unit]) -> Run:
    """A linha de segmentos seguidos: do meio segmento antes do primeiro ao
    meio segmento depois do último (meio passo, no máximo meio segmento)."""
    run = Run(group)
    half = half_length(group)
    first, last = group[0], group[-1]
    run.points = [
        (first.center[0] - first.direction[0] * half, first.center[2] - first.direction[1] * half),
        (last.center[0] + last.direction[0] * half, last.center[2] + last.direction[1] * half),
    ]
    return run


def stretch_band(band: list[Unit]) -> Run:
    """A faixa transversal: a linha corre ao lado dos segmentos (perpendicular a
    eles), pelo meio deles, e dura o que eles ocupam mais meio segmento a cada
    ponta."""
    run = Run(band)
    half = statistics.median(unit.length for unit in band) / 2
    axis = (-band[0].direction[1], band[0].direction[0])
    band.sort(key=lambda unit: axis[0] * unit.center[0] + axis[1] * unit.center[2])
    first, last = band[0], band[-1]
    # A linha passa pelo centro médio dos segmentos e segue a perpendicular.
    center = (
        statistics.mean(unit.center[0] for unit in band),
        statistics.mean(unit.center[2] for unit in band),
    )
    head = (center[0] - axis[0] * (gap(first, last) / 2 + half), center[1] - axis[1] * (gap(first, last) / 2 + half))
    tail = (center[0] + axis[0] * (gap(first, last) / 2 + half), center[1] + axis[1] * (gap(first, last) / 2 + half))
    run.points = [head, tail]
    return run


def half_length(run: list[Unit]) -> float:
    """Até onde a linha passa do último centro: meio passo, no máximo meio segmento."""
    model = statistics.median(unit.length for unit in run)
    gaps = [gap(a, b) for a, b in zip(run, run[1:])]
    return 0.5 * (min(statistics.median(gaps), model) if gaps else model)


def assemble(units: list[Unit]) -> list[Run]:
    """Linhas → polilinhas (juntas nos cantos); os portões ficam à parte."""
    runs: list[Run] = []
    segments = [unit for unit in units if not unit.gate]
    runs = merge_at_corners(lines_of(segments))
    for gate in (unit for unit in units if unit.gate):
        run = Run([gate])
        run.points = [
            (gate.center[0] - gate.direction[0] * gate.length / 2, gate.center[2] - gate.direction[1] * gate.length / 2),
            (gate.center[0] + gate.direction[0] * gate.length / 2, gate.center[2] + gate.direction[1] * gate.length / 2),
        ]
        run.name = gate.node.label or "portao"
        # A linha que chega perto dum portão encosta no poste dele: sem dois
        # postes a 30 cm nem um vão pelo qual se entra.
        for other in runs:
            for index, point in enumerate(other.points):
                for post in run.points:
                    if math.hypot(point[0] - post[0], point[1] - post[1]) <= GATE_SNAP_M:
                        other.points[index] = post
        runs.append(run)
    return runs


def merge_at_corners(runs: list[Run]) -> list[Run]:
    """As linhas cujas pontas se encostam emendam-se nesse canto.

    O canto é a interseção das duas linhas (não o ponto médio): fica um poste
    de canto em vez de dois, e fecha o buraco que o layout antigo deixava nos
    cantos (até um segmento em falta). Uma cadeia que fecha em si mesma é um
    perímetro (sem repetir o primeiro ponto). Só se juntam linhas do mesmo
    pai — o `fence-points` é local.
    """
    count = len(runs)
    ends = [[list(run.points[0]), list(run.points[-1])] for run in runs]  # [início, fim]
    axis = [(run.points[1][0] - run.points[0][0], run.points[1][1] - run.points[0][1]) for run in runs]
    axis = [(x / (math.hypot(x, z) or 1.0), z / (math.hypot(x, z) or 1.0)) for x, z in axis]
    used = [False] * count
    out: list[Run] = []
    for seed in range(count):
        if used[seed]:
            continue
        used[seed] = True
        # (linha, lado voltado para o anterior da cadeia, canto com ele)
        chain: list[list] = [[seed, 0, None]]
        grew = True
        while grew:
            grew = False
            # Cresce pelas duas pontas: uma linha reivindicada por um lado
            # tinha de poder chamar a vizinha pelo outro.
            for at_head in (False, True):
                index, near, _ = chain[0] if at_head else chain[-1]
                free = near if at_head else 1 - near
                tail = ends[index][free]
                best: tuple[float, int, int, tuple[float, float]] | None = None
                for other in range(count):
                    if used[other] or runs[other].units[0].node.parent is not runs[seed].units[0].node.parent:
                        continue
                    for side in (0, 1):
                        point = ends[other][side]
                        apart = math.hypot(tail[0] - point[0], tail[1] - point[1])
                        if apart > CORNER_M:
                            continue
                        joint = crossing(tail, axis[index], point, axis[other])
                        if joint is None:
                            continue
                        if best is None or apart < best[0]:
                            best = (apart, other, side, joint)
                if best is None:
                    continue
                _, other, side, joint = best
                used[other] = True
                ends[index][free] = list(joint)
                ends[other][side] = list(joint)
                if at_head:
                    chain.insert(0, [other, 1 - side, joint])
                else:
                    chain.append([other, side, joint])
                grew = True
        merged = Run(runs[seed].units)
        # Cada linha entra pelo canto com a anterior (já substituído) e sai
        # pela ponta livre — que é o canto com a seguinte.
        first_index, first_near, _ = chain[0]
        corners: list[tuple[float, float]] = [tuple(ends[first_index][first_near])]  # type: ignore[list-item]
        for index, near, _ in chain:
            if index != seed:
                merged.units.extend(runs[index].units)
            corners.append(tuple(ends[index][1 - near]))  # type: ignore[arg-type]
        last, near_last, _ = chain[-1]
        head, tail = ends[first_index][first_near], ends[last][1 - near_last]
        merged.closed = len(chain) >= 2 and math.hypot(head[0] - tail[0], head[1] - tail[1]) <= CORNER_M
        if merged.closed and (joint := crossing(tail, axis[last], head, axis[first_index])) is not None:
            corners.pop()
            corners[0] = joint
        else:
            merged.closed = False
        merged.points = corners
        out.append(merged)
    return out


def crossing(p, u, q, v) -> tuple[float, float] | None:
    """O ponto onde a recta `p + s·u` corta `q + t·v`, se se cortam perto."""
    determinant = u[0] * v[1] - u[1] * v[0]
    if abs(determinant) < math.sin(math.radians(20.0)):
        return None
    dx, dz = q[0] - p[0], q[1] - p[1]
    step = (dx * v[1] - dz * v[0]) / determinant
    joint = (p[0] + u[0] * step, p[1] + u[1] * step)
    if math.hypot(joint[0] - p[0], joint[1] - p[1]) > CORNER_M or math.hypot(joint[0] - q[0], joint[1] - q[1]) > CORNER_M:
        return None
    return joint


def site_prefix(units: list[Unit]) -> str:
    """O nome comum às cercas de um mesmo sítio (`cerca-s0-03` e `cerca-n1-07`
    dão `cerca`; `cerca-w-s1` e `cerca-w-w2` dão `cerca-w`). Sem nomes — ou
    quando o comum é só «cerca» — fica o do grupo: uma célula dá
    `cerca-x-1-z0`, o curral dá `cercado`."""
    labels = [unit.node.label for unit in units if unit.node.label]
    prefix = ""
    if labels:
        stripped = [re.sub(r"[-_]?\d+$", "", label) for label in labels]
        prefix = os.path.commonprefix(stripped).rstrip("-_")
    if prefix in ("", "cerca"):
        parent = units[0].node.parent
        group = parent.label if parent and parent.label else ""
        cell = re.match(r"c[ée]lula x(-?\d+) z(-?\d+)$", group)
        prefix = f"cerca-x{cell.group(1)}-z{cell.group(2)}" if cell else (group or "cerca")
    return prefix


def name_run(run: Run, taken: set[str], prefix: str) -> str:
    """A primeira linha do sítio fica com o nome dele; as outras acrescentam
    o canto (`cercado-so`) ou um número (`cerca-w-2`)."""
    name = prefix
    if name in taken:
        if len(run.points) >= 3:
            # Cotovelo: o canto (o ponto do meio) em relação ao centro da cerca.
            middle = run.points[len(run.points) // 2]
            center = (
                statistics.mean(unit.center[0] for unit in run.units),
                statistics.mean(unit.center[2] for unit in run.units),
            )
            dx, dz = middle[0] - center[0], middle[1] - center[1]
            name = f"{prefix}-{'s' if dz < 0 else 'n'}{'o' if dx < 0 else 'e'}"
        index = 2
        while name in taken:
            name = f"{prefix}-{index}"
            index += 1
    taken.add(name)
    return name


# -------------------------------------------------------------------- a escrita


def fresh_id(taken: set[int]) -> int:
    while True:
        candidate = random.getrandbits(60)
        if candidate not in taken:
            taken.add(candidate)
            return candidate


def chunk_of(run: Run, style: str, interior: bool, taken_ids: set[int], indent: str) -> str:
    parent = run.units[0].node.parent
    to_local = invert(parent.world) if parent else identity()

    def cota(x: float, z: float) -> float:
        nearest = min(run.units, key=lambda unit: math.hypot(unit.center[0] - x, unit.center[2] - z))
        return nearest.center[1]

    local = [apply(to_local, (x, cota(x, z), z)) for x, z in run.points]
    origin = local[0]
    if interior:
        # Sem terreno: cada ponto traz a sua cota e a cerca não amostra o chão.
        points = "; ".join(f"{num(p[0] - origin[0])} {num(p[1] - origin[1])} {num(p[2] - origin[2])}" for p in local)
        follow = f'{indent}    follow: core::option::Option::Some("points"),\n'
    else:
        points = "; ".join(f"{num(p[0] - origin[0])} {num(p[2] - origin[2])}" for p in local)
        follow = ""
    closed = f"{indent}    closed: core::option::Option::Some(true),\n" if run.closed else ""
    where = " ".join(num(value) for value in origin)
    pad = indent + "    "
    return (
        f'{indent}#"{run.name}"\n'
        f"{indent}{TRANSFORM} {{\n"
        f"{pad}translation: glam::Vec3 {{ x: {num(origin[0])}, y: {num(origin[1])}, z: {num(origin[2])} }},\n"
        f"{pad}scale: glam::Vec3 {{ x: 1.0, y: 1.0, z: 1.0 }},\n"
        f"{indent}}}\n"
        f"{indent}bevy_ext_scene::FenceSpec {{\n"
        f'{pad}points: "{points}",\n'
        f'{pad}style: core::option::Option::Some("{style}"),\n'
        f"{closed}{follow}"
        f"{indent}}}\n"
        f"{indent}bevy_world_xml::bsn::WorldNode {{\n"
        f'{pad}tag: "Entity",\n'
        f"{pad}attrs: [\n"
        f'{pad}    bevy_world_xml::bsn::WorldProp {{ key: "name", value: "{run.name}" }},\n'
        f'{pad}    bevy_world_xml::bsn::WorldProp {{ key: "translation", value: "{where}" }},\n'
        f"{pad}],\n"
        f'{pad}text: "",\n'
        f"{pad}pose: true,\n"
        f"{indent}}}\n"
        f"{indent}jackdaw_scene_types::node_id::SceneNodeId({fresh_id(taken_ids)})\n"
    )


def migrate(text: str, style: str, interior: bool, report: list[str]) -> str:
    """O texto novo com as cercas trocadas (e o relatório em `report`)."""
    roots = parse(text)
    units, stale = find_units(roots)
    if not units:
        report.append("  sem cercas de GLB")
        return text
    lines = text.split("\n")
    runs = assemble(units)
    taken: set[str] = set()
    sites: dict[int, str] = {}
    for run in runs:
        if run.gate:
            continue  # o portão fica com o nome que tinha
        parent = run.units[0].node.parent
        if parent.start not in sites:
            same_site = [unit for unit in units if unit.node.parent is parent and not unit.gate]
            lines_here = sum(1 for other in runs if not other.gate and other.units[0].node.parent is parent)
            sites[parent.start] = site_prefix(same_site)
            # Um sítio com várias linhas não dá o nome dele a uma só delas: todas
            # levam o canto ou o número (e o nome do grupo fica só no grupo).
            taken.add(sites[parent.start]) if lines_here > 1 else None
        run.name = name_run(run, taken, sites[parent.start])
    taken_ids = {int(value) for value in SCENE_ID_RE.findall(text)}

    gone: dict[int, list[Node]] = {}
    added: dict[int, list[str]] = {}
    for run in runs:
        nodes = [unit.node for unit in run.units]
        parent = nodes[0].parent
        if parent is None or any(unit.node.parent is not parent for unit in run.units):
            raise SystemExit(f"a cerca `{run.name}` junta nós de pais diferentes")
        gone.setdefault(parent.start, []).extend(nodes)
        first = lines[nodes[0].start]
        indent = " " * (len(first) - len(first.lstrip()))
        # O portão é sempre um portão; o estilo do sítio é para as linhas.
        added.setdefault(parent.start, []).append(chunk_of(run, "gate" if run.gate else style, interior, taken_ids, indent))
        report.append(
            f"  {run.name}: {len(run.units)} segmento(s)"
            f"{' (portão)' if run.gate else ''}{' perímetro fechado' if run.closed else ''}"
            f" em `{parent.label or tag_of(parent)}`; {len(run.points)} ponto(s)"
        )
    for node in stale:
        if node.parent:
            gone.setdefault(node.parent.start, []).append(node)
            report.append(f"  protótipo `{node.attrs.get('id')}` sem uso: sai")

    cuts: set[int] = set()
    insert_after: dict[int, list[str]] = {}
    for parent in walk(roots):
        if parent.start not in gone:
            continue
        out = gone[parent.start]
        keep = [child for child in parent.children if child not in out]
        ranges: list[list[int]] = []
        for child in (child for child in parent.children if child in out):
            start, stop = child.start, child.end
            if lines[stop + 1].strip() == "--":
                stop += 1  # leva o separador seguinte
            elif lines[start - 1].strip() == "--":
                start -= 1  # era o último: leva o separador anterior
            ranges.append([start, stop])
        merged: list[list[int]] = []
        for start, stop in sorted(ranges):
            if merged and start <= merged[-1][1] + 1:
                merged[-1][1] = max(merged[-1][1], stop)
            else:
                merged.append([start, stop])
        for start, stop in merged:
            # Um corte que chega ao fim da lista leva também o `--` de antes.
            if stop == parent.close - 1 and lines[start - 1].strip() == "--":
                start -= 1
            cuts.update(range(start, stop + 1))
        new = added.get(parent.start, [])
        if new:
            anchor = keep[-1].end if keep else parent.open
            block: list[str] = []
            for piece in new:
                if block or keep:
                    block.append("--")
                block.extend(piece[:-1].split("\n"))
            insert_after[anchor] = block
    out_lines: list[str] = []
    for index, line in enumerate(lines):
        if index not in cuts:
            out_lines.append(line)
        if index in insert_after:
            out_lines.extend(insert_after[index])
    return "\n".join(out_lines)


def leftovers(text: str) -> list[str]:
    units, stale = find_units(parse(text))
    return [repr(unit.node) for unit in units] + [f"protótipo {node!r}" for node in stale]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply", action="store_true", help="escreve os ficheiros")
    parser.add_argument("--check", action="store_true", help="exit 1 se ainda houver cercas de GLB")
    parser.add_argument("--style", help="predefinição para todas as cercas (ganha à de `SITES`)")
    args = parser.parse_args()

    remaining = 0
    for relative, style in SITES:
        path = ROOT / relative
        if not path.exists():
            continue
        text = path.read_text()
        if args.check:
            found = leftovers(text)
            print(f"{relative}: {len(found)} cerca(s) de GLB")
            remaining += len(found)
            continue
        report: list[str] = [f"{relative}:"]
        migrated = migrate(text, args.style or style, relative.startswith("scenes/interiors/"), report)
        print("\n".join(report))
        if not args.apply:
            continue
        still = leftovers(migrated)
        if still:
            print(f"  ERRO: a cirurgia deixou {len(still)}: {still[:3]} — nada escrito")
            return 2
        path.write_text(migrated)
        print("  escrito")
    return 1 if remaining else 0


if __name__ == "__main__":
    sys.exit(main())
