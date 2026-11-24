#!/usr/bin/env python3
"""Autoria offline dos trilhos, pousos e cais do Vale Exterior."""
from __future__ import annotations

import argparse
import heapq
import json
import math
import re
import uuid
from pathlib import Path

import numpy as np

from expand_vale import BASINS, PADS, SEA_LEVEL, WORLD_SIZE
from make_map import load_ahgt

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "world/expansion"
MESH = "/assets/meshes/"
STEP = 22
LIMIT = 3440
SHELTER_COLLIDER = (5.15, 2.42, 4.2, 1.42)
PROP_RE = re.compile(r'key: "([^"]*)", value: "([^"]*)"')
BODY_RE = re.compile(r'tag: "(Lake|River|Road)",\s*attrs: \[(.*?)\n\s*\]', re.S)

MONUMENT_MASKS = [
    (880.0, 780.0, 26.0),   # Alto de Vael altar & agulha
    (770.0, 730.0, 14.0),   # Torre de vigia do Alto
    (-881.0, -320.0, 56.0), # Cidadela Caida
    (218.0, 66.0, 16.0),    # Obelisco do Sol
]


def _splitmix64(state: int) -> int:
    state = (state + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    z = state
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return z ^ (z >> 31)


def lake_contour_fn(ax, az, seed=None, axis_deg=None, stretch=None, lobes=None):
    def h_hash(salt):
        s = float(np.float32(np.float32(ax) * np.float32(12.9898) + np.float32(az) * np.float32(78.233)))
        return abs(math.modf(math.sin(s * salt + salt * 91.7) * 43758.55)[0])

    def g_seed(salt):
        st = (int(seed) ^ ((salt * 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF)) & 0xFFFFFFFFFFFFFFFF
        return (_splitmix64(st) >> 40) / 16777216.0

    pick = (lambda fs, us: g_seed(us)) if seed is not None else (lambda fs, us: h_hash(fs))
    harmonics = []
    for i, k in enumerate((1.0, 3.0, 5.0, 7.0)):
        lo, hi = (0.03, 0.05, 0.03, 0.02)[i], (0.06, 0.10, 0.05, 0.04)[i]
        harmonics.append([k, lo + (hi - lo) * pick(2.3 + i * 1.37, 11 + i), pick(3.7 + i * 1.73, 21 + i) * math.tau])
    ax_rad = math.radians(axis_deg) if axis_deg is not None else pick(0.77, 1) * math.tau
    st_val = max(0.0, min(0.45, stretch)) if stretch is not None else 0.10 + 0.10 * pick(1.31, 2)
    if lobes is not None:
        m = max(0.0, min(1.6, lobes))
        for h in harmonics:
            h[1] *= m
    thetas = np.linspace(0.0, math.tau, 128, endpoint=False)
    vals = 1.0 + st_val * np.cos(2.0 * (thetas - ax_rad))
    for k, a, p in harmonics:
        vals = vals + a * np.sin(k * thetas + p)
    mx, mn = float(vals.max()), float(vals.min())
    sc = 1.0
    if mx > 1.45:
        sc = min(sc, 0.45 / (mx - 1.0))
    if mn < -0.45:
        sc = min(sc, -0.45 / (mn - 1.0))
    if sc < 1.0:
        sc *= 0.995
        st_val *= sc
        for h in harmonics:
            h[1] *= sc

    def rfrac(theta):
        r = 1.0 + st_val * np.cos(2.0 * (theta - ax_rad))
        for k, a, p in harmonics:
            r = r + a * np.sin(k * theta + p)
        return r

    return rfrac


def lake_mirror_reach(depth: float, water_offset: float) -> float:
    ratio = max(0.0, min(1.0, water_offset / max(depth, 1e-3)))
    return max(0.0, min(1.45 * 1.25, math.sqrt(max(0.0, 1.0 - ratio ** (2.0 / 3.0))) * 1.25))


def _dist_seg(px: float, pz: float, ax: float, az: float, bx: float, bz: float) -> float:
    abx, abz = bx - ax, bz - az
    l2 = abx * abx + abz * abz
    t = 0.0 if l2 <= 1e-12 else max(0.0, min(1.0, ((px - ax) * abx + (pz - az) * abz) / l2))
    return math.hypot(px - (ax + abx * t), pz - (az + abz * t))

PLACES = [
    ("porto-do-poente", "Porto do Poente", "costa", 0),
    ("farol-da-lanterna", "Farol da Lanterna", "costa", 1),
    ("ilha-dos-juncos", "Ilha dos Juncos", "costa", 2),
    ("pouso-dos-freixos", "Pouso dos Freixos", "freixos", 3),
    ("refugio-do-vidro", "Refúgio do Vidro", "vidro", 4),
    ("entreposto-das-sete-margens", "Entreposto das Sete Margens", "sete_margens", 5),
    ("pouso-do-sul", "Pouso do Sul", "sete_margens", 6),
    ("passo-da-salina", "Passo da Salina", "costa", 7),
    ("atalaia-de-barro", "Atalaia de Barro", "vidro", 8),
    ("marco-do-norte", "Marco do Norte", "freixos", 9),
    ("estalagem-do-caminho-comprido", "Estalagem do Caminho Comprido", "sete_margens", 10),
]

PATHS = [
    ("Estrada das Salinas", [(-835, -280), (-1320, -280), (-2130, -230), (-2370, 335), (-2590, 335)]),
    ("Trilho dos Freixos", [(-1320, -280), (-1520, -815), (-1520, -1350), (-1325, -1780)]),
    ("Caminho do Norte", [(-1325, -1780), (-740, -2130), (125, -2735), (770, -2230), (1410, -1740)]),
    ("Subida do Vidro", [(1450, -706), (1550, -1060), (1410, -1740)]),
    ("Estrada do Barro", [(858, 756), (1330, 690), (1840, 610), (2320, 470)]),
    ("Rota das Sete Margens", [(172, 1370), (560, 1440), (1320, 1735)]),
    ("Caminho Comprido", [(560, 1440), (430, 2040), (90, 2630), (-740, 2550), (-1650, 2110)]),
]

def q(value):
    return json.dumps(str(value), ensure_ascii=False)


def xyz(v):
    return " ".join(f"{n:.3f}" for n in v)


def attrs_bsn(attrs):
    return "\n".join(
        f"        bevy_world_xml::bsn::WorldProp {{ key: {q(k)}, value: {q(v)} }},"
        for k, v in attrs.items()
    )


def node(name, tag="Entity", at=(0, 0, 0), attrs=None, children=(), *, extra="", rotate=0, roll=0, scale=1):
    key = uuid.uuid5(uuid.NAMESPACE_URL, "vael/vale-exterior/" + name).int % 2**62 + 1
    x, y, z = at
    # Yaw em torno de +Y, seguido pela inclinação do tabuleiro em torno de Z.
    cy, sy = math.cos(rotate * .5), math.sin(rotate * .5)
    cz, sz = math.cos(roll * .5), math.sin(roll * .5)
    props = {"static": "true"} if tag == "world" else {"name": name}
    props.update(attrs or {})
    shape = ""
    if tag not in ("Road", "SpawnExclusion", "Vegetation", "Include", "world"):
        shape = "true"
    else:
        shape = "false"
    text = f'''#{q(name)}
bevy_transform::components::transform::Transform {{
    translation: glam::Vec3 {{ x: {x:.3f}, y: {y:.3f}, z: {z:.3f} }},
    rotation: glam::Quat {{ x: {sy*sz:.6f}, y: {sy*cz:.6f}, z: {cy*sz:.6f}, w: {cy*cz:.6f} }},
    scale: glam::Vec3 {{ x: {scale:.4f}, y: {scale:.4f}, z: {scale:.4f} }},
}}
{extra}bevy_world_xml::bsn::WorldNode {{
    tag: {q(tag)},
    attrs: [
{attrs_bsn(props)}
    ],
    text: "",
    pose: {shape},
}}
jackdaw_scene_types::node_id::SceneNodeId({key})
'''
    if children:
        indent = lambda fragment: "\n".join("    " + line for line in fragment.rstrip().splitlines())
        text += "bevy_ecs::hierarchy::Children [\n" + "\n    --\n".join(map(indent, children)) + "\n]\n"
    return text


def write_scene(file, children):
    (OUT / f"{file}.bsn").write_text(node(file, "world", children=children))


def script(path):
    return f"bevy_ext_scene::ScriptRef {{ path: {q(path)} }}\n"


def model(name, asset, at, *, yaw=0, scale=1, collision=None, behavior=None, attrs=None, children=()):
    extra = script(behavior) if behavior else ""
    if collision:
        if isinstance(collision, tuple):
            width, height, depth, offset = collision
            shape = f"Box {{ size: glam::Vec3 {{ x: {width}, y: {height}, z: {depth} }}, offset_y: {offset} }}"
        else:
            url = MESH + collision + ".glb"
            shape = f"Mesh {{ url: {q(url)}, base: true }}"
        extra += f'''bevy_ext_scene::ColliderSpec {{
    shape: bevy_ext_scene::ColliderShape::{shape},
}}
bevy_ext_scene::BodySpec {{ kind: bevy_ext_scene::BodyKind::Fixed }}
'''
    visual = node(name + " · malha", "GltfScene", attrs={"url": MESH + asset + ".glb"})
    return node(name, at=at, rotate=yaw, scale=scale, attrs=attrs, extra=extra,
                children=[visual, *children])


class Ground:
    def __init__(self, path):
        self.h, self.w, self.d, _ = load_ahgt(path)
        self.lakes = []
        self.rivers = []
        self.existing_roads = []
        for bsn_path in sorted((ROOT / "world").rglob("*.bsn")):
            rel = str(bsn_path.relative_to(ROOT))
            if rel.startswith(("world/prefabs/", "world/scenes/", "world/expansion/routes", "world/expansion/settlements")):
                continue
            for kind, fields in BODY_RE.findall(bsn_path.read_text()):
                attrs = dict(PROP_RE.findall(fields))
                if kind == "Lake" and "at" in attrs and "radius" in attrs:
                    cx, cz = map(float, attrs["at"].split())
                    rad = float(attrs["radius"])
                    depth = float(attrs.get("depth", 1.5))
                    off = float(attrs.get("water-offset", 0.5))
                    wl = float(attrs.get("water-level", 0.0))
                    seed = int(float(attrs["seed"])) if "seed" in attrs else None
                    axis_deg = float(attrs["axis"]) if "axis" in attrs else None
                    stretch = float(attrs["stretch"]) if "stretch" in attrs else None
                    lobes = float(attrs["lobes"]) if "lobes" in attrs else None
                    rf = lake_contour_fn(cx, cz, seed, axis_deg, stretch, lobes)
                    mr = lake_mirror_reach(depth, off)
                    self.lakes.append((attrs.get("name", ""), cx, cz, rad, wl, rf, mr))
                elif kind == "River" and "path" in attrs:
                    pts = [float(v) for v in attrs["path"].split()]
                    pairs = list(zip(pts[::2], pts[1::2]))
                    hw = float(attrs.get("width", 6.0)) * 0.5
                    self.rivers.append((pairs, hw))
                elif kind == "Road" and "path" in attrs:
                    pts = [float(v) for v in attrs["path"].split()]
                    pairs = list(zip(pts[::2], pts[1::2]))
                    hw = float(attrs.get("width", 3.0)) * 0.5
                    self.existing_roads.append((pairs, hw))
        self.axis = np.arange(-LIMIT, LIMIT + 1, STEP)
        X, Z = self.axis[None, :], self.axis[:, None]
        self.route_h = self.sample(X, Z)
        self.route_ok = ~self.wet(X, Z, self.route_h, 0.85)
        self.route_ok &= np.hypot(X, Z) < LIMIT - 16
        for mx, mz, mr in MONUMENT_MASKS:
            self.route_ok &= np.hypot(X - mx, Z - mz) >= mr

    def sample(self, x, z):
        u = np.clip((np.asarray(x) / WORLD_SIZE + .5) * (self.w - 1), 0, self.w - 1)
        v = np.clip((np.asarray(z) / WORLD_SIZE + .5) * (self.d - 1), 0, self.d - 1)
        i, j = u.astype(int), v.astype(int)
        im, jm = np.minimum(i + 1, self.w - 1), np.minimum(j + 1, self.d - 1)
        a, b = u - i, v - j
        return (self.h[j, i] * (1 - a) + self.h[j, im] * a) * (1 - b) + (
            self.h[jm, i] * (1 - a) + self.h[jm, im] * a
        ) * b

    def find_lake(self, cx, cz):
        for lake in self.lakes:
            if abs(lake[1] - cx) < 2.0 and abs(lake[2] - cz) < 2.0:
                return lake
        return None

    def wet(self, x, z, y, margin=0):
        x_arr = np.asarray(x)
        z_arr = np.asarray(z)
        y_arr = np.asarray(y)
        water = (x_arr < -2075) & (y_arr < SEA_LEVEL + margin)
        for _, cx, cz, rad, wl, rf, mr in self.lakes:
            dx, dz = x_arr - cx, z_arr - cz
            dist = np.hypot(dx, dz)
            max_r = rad * 1.45 * mr + 8.0
            in_bbox = dist <= max_r
            if np.any(in_bbox):
                th = np.arctan2(dz, dx)
                contour_r = rad * rf(th) * mr + (4.0 if margin > 0 else 0.0)
                water = water | (dist <= contour_r)
        for pairs, hw in self.rivers:
            pad = hw + (2.5 if margin > 0 else 0.5)
            xs = [p[0] for p in pairs]; zs = [p[1] for p in pairs]
            bx0, bx1 = min(xs) - pad, max(xs) + pad
            bz0, bz1 = min(zs) - pad, max(zs) + pad
            in_box = (x_arr >= bx0) & (x_arr <= bx1) & (z_arr >= bz0) & (z_arr <= bz1)
            if np.any(in_box):
                for (ax, az), (bx, bz) in zip(pairs, pairs[1:]):
                    abx, abz = bx - ax, bz - az
                    l2 = abx * abx + abz * abz
                    t = 0.0 if l2 <= 1e-12 else np.clip(((x_arr - ax) * abx + (z_arr - az) * abz) / l2, 0.0, 1.0)
                    d = np.hypot(x_arr - (ax + abx * t), z_arr - (az + abz * t))
                    water = water | (d <= pad)
        return water

    def point(self, x, z):
        return x, float(self.sample(x, z)), z

    def route(self, begin, end):
        side = len(self.axis)
        def index(at):
            x, z = map(lambda n: int(np.argmin(abs(self.axis - n))), at)
            nearest = [(j, i) for j in range(max(0, z - 3), min(side, z + 4))
                       for i in range(max(0, x - 3), min(side, x + 4)) if self.route_ok[j, i]]
            if not nearest:
                raise RuntimeError(f"Sem chão perto de {at}")
            return min(nearest, key=lambda p: (self.axis[p[1]] - at[0])**2 + (self.axis[p[0]] - at[1])**2)
        start, goal = index(begin), index(end)
        queue = [(0.0, start)]
        cost = {start: 0.0}
        prev = {}
        seen = set()
        neighbours = [(dz, dx, math.hypot(dz, dx) * STEP)
                       for dz in (-1, 0, 1) for dx in (-1, 0, 1) if (dz, dx) != (0, 0)]
        while queue:
            _, at = heapq.heappop(queue)
            if at in seen:
                continue
            if at == goal:
                result = [end]
                while at != start:
                    result.append((int(self.axis[at[1]]), int(self.axis[at[0]])))
                    at = prev[at]
                return [begin, *reversed(result[1:]), end]
            seen.add(at)
            for dz, dx, run in neighbours:
                z, x = at[0] + dz, at[1] + dx
                nxt = (z, x)
                if not (0 <= z < side and 0 <= x < side) or not self.route_ok[nxt] or nxt in seen:
                    continue
                slope = abs(float(self.route_h[nxt] - self.route_h[at])) / run
                if slope > 0.66:
                    continue
                mid_x = (self.axis[x] + self.axis[at[1]]) * 0.5
                mid_z = (self.axis[z] + self.axis[at[0]]) * 0.5
                y_mid = (self.route_h[nxt] + self.route_h[at]) * 0.5
                if self.wet(mid_x, mid_z, y_mid, 0.65):
                    continue
                new = cost[at] + run * (1 + 10 * slope**2)
                if new < cost.get(nxt, math.inf):
                    cost[nxt] = new
                    prev[nxt] = at
                    heapq.heappush(queue, (new + STEP * math.dist(nxt, goal), nxt))
        raise RuntimeError(f"Sem ligação por terra entre {begin} e {end}")


def thin_path(points):
    keep = [points[0]]
    for index, point in enumerate(points[1:-1], start=1):
        a, b, c = np.array([keep[-1], point, points[index + 1]])
        determinant = abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]))
        if determinant > 10 or np.linalg.norm(c-a) > 70:
            keep.append(point)
    keep.append(points[-1])
    return keep


def road(name, points, small=False):
    return node(name, "Road", attrs={
        "path": " ".join(f"{x:g} {z:g}" for x, z in points),
        "width": "2.4" if small else "4.0", "flatten": 0,
        "texture-scale": 6, "edge-feather": 0.9, "edge-noise": 0.43,
        "texture-url": "/assets/textures/dirt_trail/albedo.ktx2",
        "normal-map-url": "/assets/textures/dirt_trail/normal.ktx2",
    })


def box(name, at, size, color, *, yaw=0, roll=0, solid=True, landing=False):
    attrs = {"half-size": xyz(n / 2 for n in size), "base-color": color, "roughness": .88}
    if solid:
        attrs.update({"collider": "auto", "body": "fixed"})
    if landing:
        attrs["prop-boat-landing"] = "1"
    return node(name, "Box", at=at, rotate=yaw, roll=roll, attrs=attrs)


def pier(name, bank, water, level):
    bank, water = np.array(bank), np.array(water)
    forward = (water - bank) / np.linalg.norm(water - bank)
    side = np.array([-forward[1], forward[0]])
    length = float(np.linalg.norm(water - bank))
    deck_y = level + .56
    yaw = math.atan2(-float(forward[1]), float(forward[0]))
    ramp = min(4.8, length * .28)
    deck_mid = bank + forward * ((length + ramp) * .5)
    at = lambda p, y: (float(p[0]), y, float(p[1]))
    pieces = [
        box(name + " · tabuleiro", at(deck_mid, deck_y - .13), (length - ramp, .26, 3.15),
            "#896846", yaw=yaw, landing=True),
        box(name + " · rampa", at(bank + forward * ramp * .50, deck_y + .13),
            (ramp + .3, .26, 3.15), "#95734e", yaw=yaw, roll=-.105, landing=True),
    ]
    for i, d in enumerate(np.arange(ramp + 1, length, 4.3)):
        for sign in [-1, 1]:
            xy = bank + forward * d + sign * 1.26 * side
            pieces.append(box(f"{name} · estaca {i} {sign}", at(xy, level - 1.1),
                              (.34, 4.15, .34), "#564131", solid=False))
    for i, d in enumerate(np.arange(ramp + .36, length - .15, .58)):
        xy = bank + forward * d
        pieces.append(box(f"{name} · tábua {i}", at(xy, deck_y - .019), (.53, .065, 3.18),
                          ["#ab8150", "#987049", "#a3764b"][i % 3], yaw=yaw, solid=False))
    return node(name, "Group", children=pieces)


def dock_pair(terrain, name, center, direction, level, initial=(0, 650)):
    direction = np.array(direction, dtype=float)
    direction /= np.linalg.norm(direction)
    origin = np.array(center, dtype=float)
    lake = terrain.find_lake(origin[0], origin[1])
    if lake is not None:
        _, cx, cz, rad, wl, rf, mr = lake
        # Check radial ray going out from lake center toward bank: dir_out = -direction when initial is negative!
        th_out = math.atan2(-direction[1], -direction[0]) if initial[0] < 0 else math.atan2(direction[1], direction[0])
        r_water = float(rad * rf(th_out) * mr)
        # Bank starts outside the lobed lake water contour on dry ground (r >= r_water + 1.5m)
        s_steps = np.arange(initial[0], initial[1], 0.5)
        pts = origin + s_steps[:, None] * direction
        dists = np.hypot(pts[:, 0] - cx, pts[:, 1] - cz)
        hs = terrain.sample(pts[:, 0], pts[:, 1])
        valid = np.flatnonzero((dists >= r_water + 1.5) & (hs >= level + 0.35))
        if len(valid) == 0:
            raise ValueError(f"Sem margem seca fora do contorno de água para {name}")
        # Closest valid point to the water contour
        bank_index = valid[int(np.argmin(dists[valid]))]
        bank = pts[bank_index]
        r_bank = float(np.hypot(bank[0] - cx, bank[1] - cz))
        length = max(r_bank - r_water + 11.0, 14.0)
        side = np.array([-direction[1], direction[0]])
        for _ in range(120):
            end = bank + length * direction
            hull = end - direction * 2.6 + side * 3.1
            corners = np.array([hull + a * direction + b * side for a in (-4.4, 4.4) for b in (-2.1, 2.1)])
            cdx, cdz = corners[:, 0] - cx, corners[:, 1] - cz
            c_dist = np.hypot(cdx, cdz)
            c_rw = rad * rf(np.arctan2(cdz, cdx)) * mr
            in_water = np.all(c_dist < c_rw - 2.0)
            deep_enough = np.max(terrain.sample(corners[:, 0], corners[:, 1])) < level - 1.05
            if in_water and deep_enough:
                break
            length += 0.75
        else:
            raise ValueError(f"Sem calado livre dentro do lago no cais de {name}")
    else:
        s = np.arange(initial[0], initial[1], .5)
        line = origin + s[:, None] * direction
        h = terrain.sample(line[:, 0], line[:, 1])
        candidates = np.flatnonzero((h >= level + .62) & (np.r_[h[1:], h[-1]] < level + .62))
        if len(candidates) == 0:
            raise ValueError(f"Sem entrada para o cais de {name}")
        bank_index = candidates[-1]
        depth = np.flatnonzero(h[bank_index:] < level - 1.35)
        if not len(depth):
            raise ValueError(f"Sem profundidade no cais de {name}")
        bank = line[bank_index]
        # Barcos ficam paralelos ao cais, com espaço entre costado e tabuleiro.
        length = max(float(depth[0]) * .5 + 4, 14)
        side = np.array([-direction[1], direction[0]])
        for _ in range(120):
            end = bank + length * direction
            hull = end - direction * 2.6 + side * 3.1
            corners = np.array([hull + a * direction + b * side for a in (-4.4, 4.4) for b in (-2.1, 2.1)])
            if np.max(terrain.sample(corners[:, 0], corners[:, 1])) < level - 1.05:
                break
            length += .65
        else:
            raise ValueError(f"Sem calado livre no cais de {name}")
    stake = end - direction * 2.6 + side * 1.04
    return pier(name, bank, end, level), {
        "x": round(float(hull[0]), 2), "z": round(float(hull[1]), 2),
        "yaw": round(math.atan2(direction[0], direction[1]), 4),
        "mooring": [round(float(stake[0]), 2), round(level + .56, 2), round(float(stake[1]), 2)],
    }, tuple(map(float, bank))


def lighthouse(terrain, name, x, z, large=False):
    y = float(terrain.sample(x, z))
    height, radius = (14, 2.8) if large else (8.6, 1.8)
    def round_part(label, tag, yy, rad, half, color, solid=False):
        attrs = {"radius": rad, "half-height": half, "base-color": color, "roughness": .88}
        if solid:
            attrs.update({"collider": "auto", "body": "fixed"})
        return node(name + " · " + label, tag, (x, y + yy, z), attrs=attrs)
    return node(name + " · edifício", "Group", children=[
        round_part("soco", "Cylinder", .45, radius * 1.18, .65, "#8c8170", True),
        round_part("torre", "Cylinder", height / 2, radius, height * .52, "#b6a688", True),
        round_part("anel de cantaria", "Cylinder", height * .6, radius * 1.014, .34, "#756850"),
        round_part("varanda", "Cylinder", height, radius * 1.18, .35, "#7c6b52"),
        round_part("lanterna", "Cylinder", height + 1.14, radius * .70, .90, "#b09158"),
        round_part("telhado", "Cone", height + 2.19, radius * .95, 1.11, "#793e31"),
        node(name + " · lume", "PointLight", (x, y + height + 1.35, z), attrs={
            "color": "#ffb46f", "intensity": 11000, "range": 27, "shadows": "false",
        }),
    ])


def campfire(terrain, key, x, z):
    return model("campfire-" + key, "cenario/fogueira-de-cantaria_lod0", terrain.point(x, z),
                 behavior="campfire.lua", children=[
        node(f"Fogo · {key}", "ParticleSystem", (0, .38, 0), attrs={"preset": "fire"}),
        node(f"Luz da fogueira · {key}", "PointLight", (0, .75, 0), attrs={
            "color": "#ffad67", "intensity": 4200, "range": 10,
        }),
    ])


def npc(terrain, name, x, z, model_id, lines):
    body = node(name + " · corpo", "NpcBody")
    mesh = node(name + " · rig", "GltfScene", attrs={"url": MESH + f"characters/{model_id}_lod0.glb"})
    return node(name, at=terrain.point(x, z), attrs={"prop-prompt": f"Falar com {name}", "prop-banter": lines},
                children=[mesh, body], extra=script("townsfolk.lua"))


def encounter(name, kind, at, seed, number):
    x, z = at
    creature = node(name + " · criatura", "Creature", attrs={"role": "enemy"},
                    extra=script(f"enemies/{kind}.lua"), children=[
                        node(name + " · modelo", "GLTFLoader",
                             attrs={"url": MESH + f"characters/{kind}_lod0.glb"}),
                    ])
    return node(name, "DynamicSpawner", attrs={
        "count": number, "seed": seed, "region-min": f"{x-60} 0 {z-60}",
        "region-max": f"{x+60} 0 {z+60}", "avoid-water": 1, "avoid-road": 1,
        "max-slope-deg": 29, "max-distance": 290, "random-yaw": "true",
    }, children=[creature])


def _road_clearance_ok(terrain, x: float, z: float, radius: float, road_segs, placed_spots) -> bool:
    h0 = float(terrain.sample(x, z))
    if terrain.wet(x, z, h0, 1.0):
        return False
    # Check local flatness within radius
    hs = [float(terrain.sample(x + dx, z + dz)) for dx, dz in ((-2, 0), (2, 0), (0, -2), (0, 2))]
    if max(abs(max(hs) - min(hs)), 0.0) / 4.0 > 0.35:
        return False
    for (ax, az), (bx, bz), hw in road_segs:
        if _dist_seg(x, z, ax, az, bx, bz) < hw + radius + 1.5:
            return False
    for ox, oz, orad in placed_spots:
        if math.hypot(x - ox, z - oz) < radius + orad + 1.2:
            return False
    return True


def free_spot(terrain, x0: float, z0: float, radius: float, road_segs, placed_spots):
    if _road_clearance_ok(terrain, x0, z0, radius, road_segs, placed_spots):
        placed_spots.append((x0, z0, radius))
        return x0, z0
    for r in range(2, 34, 2):
        steps = max(12, int(math.tau * r / 3.0))
        for k in range(steps):
            ang = k * math.tau / steps
            cx, cz = x0 + r * math.cos(ang), z0 + r * math.sin(ang)
            if _road_clearance_ok(terrain, cx, cz, radius, road_segs, placed_spots):
                placed_spots.append((cx, cz, radius))
                return cx, cz
    placed_spots.append((x0, z0, radius))
    return x0, z0


def build_places(terrain, dock_data, road_segs):
    regions = []
    docks, boats = dock_data
    for key, title, biome, i in PLACES:
        x, z, _, _, _ = PADS[i]
        placed_spots = []
        cfx, cfz = free_spot(terrain, x + 5, z - 7, 1.4, road_segs, placed_spots)
        bits = [node(key, at=terrain.point(x, z)),
                node("Recinto · " + title, "SpawnExclusion", attrs={"at": f"{x} {z}", "radius": 65 if i != 0 else 132}),
                campfire(terrain, key, cfx, cfz)]
        if i == 0:
            for j, (dx, dz, asset, collision, prad) in enumerate([
                (-15, 20, "village/village_longhouse_lod0", "village/village_longhouse_collision", 8.5),
                (17, 15, "cenario/abrigo-do-vale_lod0", SHELTER_COLLIDER, 3.6),
                (22, -18, "cenario/abrigo-do-vale_lod0", SHELTER_COLLIDER, 3.6),
                (-22, -15, "cenario/banca-ribeirinha_lod0", None, 2.2),
                (-17, -6, "cenario/banca-ribeirinha_lod0", None, 2.2),
            ]):
                px, pz = free_spot(terrain, x + dx, z + dz, prad, road_segs, placed_spots)
                bits.append(model(f"{title} · casa {j+1}", asset,
                                  terrain.point(px, pz), yaw=(j * .91),
                                  collision=collision))
            mpx, mpz = free_spot(terrain, x - 3, z - 10, 0.9, road_segs, placed_spots)
            bits.append(npc(terrain, "Mestre do Porto", mpx, mpz, "npc_scout",
                            "O farol fica a poente. Da ponta do cais vê-se a Ilha da Lanterna.|As balizas marcam o fim da zona que conhecemos."))
            gpx, gpz = free_spot(terrain, x + 18, z - 10, 0.9, road_segs, placed_spots)
            bits.append(npc(terrain, "Guarda da Salina", gpx, gpz, "npc_guard",
                            "A estrada de terra sobe para o Passo da Salina.|Quando o vento vira, voltamos pelo mesmo canal."))
            lpx, lpz = free_spot(terrain, -2650, 390, 3.2, road_segs, placed_spots)
            bits.append(lighthouse(terrain, "Vigia do Porto", lpx, lpz))
        if i in (1, 8, 9):
            lrad = 3.5 if i == 1 else 2.5
            lpx, lpz = free_spot(terrain, x - 7, z + 8, lrad, road_segs, placed_spots)
            bits.append(lighthouse(terrain, title, lpx, lpz, large=i == 1))
        if i in (3, 4, 5, 6, 7, 10):
            for j, (dx, dz) in enumerate([(-11, 4), (13, 7)]):
                spx, spz = free_spot(terrain, x + dx, z + dz, 3.6, road_segs, placed_spots)
                bits.append(model(f"{title} · abrigo {j+1}", "cenario/abrigo-do-vale_lod0",
                                  terrain.point(spx, spz), yaw=-.78+j*1.56,
                                  collision=SHELTER_COLLIDER))
            npx, npz = free_spot(terrain, x + 5, z + 5, 0.9, road_segs, placed_spots)
            bits.append(npc(terrain, f"Vigia · {title}", npx, npz, "npc_scout",
                            "Assina este pouso para poderes voltar pela fogueira."))
        regions.append(node(title, "Group", children=bits))

    for deck in docks["decks"]:
        regions.append(deck)
    buoys = []
    for index, z in enumerate(range(-1320, 1330, 132)):
        x = -math.sqrt(3400**2 - z*z)
        y = SEA_LEVEL
        if not terrain.wet(x, z, terrain.sample(x, z)):
            continue
        buoy = f"Baliza de mar aberto {index+1}"
        buoys.extend([
            node(buoy, "Cylinder", (x, y+.30, z), attrs={
                "radius":1.2, "half-height":.50, "base-color":"#a65c3f", "roughness":.79,
            }),
            node(buoy + " · mastro", "Cone", (x, y+1.66, z), attrs={
                "radius":.62, "half-height":1.20, "base-color":"#d4b585", "roughness":.73,
            }),
        ])
    regions.append(node("Balizas da linha de navegação", "Group", children=buoys))
    habitats = [
        ("Salinas altas", "bandit", (-2100, 395)), ("Freixos ocidentais", "wolf", (-1730, -1320)),
        ("Vale do Norte", "wolf", (-550, -2190)), ("Ravina do Vidro", "goblin", (2090, -1740)),
        ("Colina do Barro", "bandit", (2040, 825)), ("Lameiros das Sete Margens", "slime", (1030, 2040)),
        ("Barranco do Sul", "wolf", (-1210, 1760)), ("Charneca do Caminho Comprido", "bandit", (-690, 2290)),
        ("Areias do Poente", "slime", (-2630, -900)), ("Pastos do Norte", "goblin", (640, -2780)),
    ]
    regions.extend(encounter(name, kind, at, i * 357 + 1234, 4 + i % 3)
                   for i, (name, kind, at) in enumerate(habitats))
    write_scene("settlements", regions)
    return boats


def compute_docks(terrain):
    dock_specs = [
        ("Cais do Poente", (-2590, 335), (-1, 0), SEA_LEVEL, (48, 290), "barco-do-poente"),
        ("Cais da Lanterna", (-3210, 375), (1, -.89), SEA_LEVEL, (20, 260), "barco-da-lanterna"),
        ("Cais dos Juncos", (-3070, -680), (1, .17), SEA_LEVEL, (20, 260), "barco-dos-juncos"),
        ("Cais dos Freixos", (-1570, -2170), (-.52, -.85), 46, (-600, -35), "barco-dos-freixos"),
        ("Cais do Vidro", (1650, -1995), (.71, -.71), 78, (-600, -30), "barco-do-vidro"),
        ("Cais das Sete Margens", (1650, 2170), (.53, .86), 55, (-680, -35), "barco-das-sete-margens"),
    ]
    boats = {}
    banks = []
    decks = []
    for name, center, direction, level, interval, key in dock_specs:
        deck, boat, bank = dock_pair(terrain, name, center, direction, level, interval)
        banks.append((name, bank))
        boats[key] = boat
        decks.append(deck)
    return {"banks": banks, "decks": decks}, boats


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("terrain", type=Path)
    parser.add_argument("--boats", type=Path, help="JSON temporário com as novas posições e amarras")
    args = parser.parse_args()
    terrain = Ground(args.terrain)
    OUT.mkdir(exist_ok=True)
    docks, boats = compute_docks(terrain)
    paths = []
    for name, stops in PATHS:
        points = [stops[0]]
        for begin, end in zip(stops, stops[1:]):
            points.extend(terrain.route(begin, end)[1:])
        paths.append((name, thin_path(points)))
    # A última ponta de cada trilho chega ao encontro real do cais e da praia.
    for (name, bank), i in zip(docks["banks"], (0, 1, 2, 3, 4, 5)):
        x, z, *_ = PADS[i]
        path = terrain.route((x, z), bank) if i in (3, 4, 5) else [(x, z), bank]
        paths.append(("Chegada · " + name, thin_path(path)))
    road_segs = []
    for pairs, hw in terrain.existing_roads:
        for a, b in zip(pairs, pairs[1:]):
            road_segs.append((a, b, hw))
    for name, pts in paths:
        hw = 1.2 if name.startswith("Chegada") else 2.0
        for a, b in zip(pts, pts[1:]):
            road_segs.append((a, b, hw))
    build_places(terrain, (docks, boats), road_segs)
    write_scene("routes", [road(name, points, small=name.startswith("Chegada")) for name, points in paths])
    if args.boats:
        args.boats.write_text(json.dumps(boats, ensure_ascii=False, indent=2))
    print(f"{len(paths)} trilhos, {len(docks['banks'])} cais, {len(PLACES)} lugares com nome")


if __name__ == "__main__":
    main()
