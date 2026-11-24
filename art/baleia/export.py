"""Model and animate the tide bell in metres, glTF Y up. Run with numpy and trimesh."""

import io
import json
from collections import defaultdict
from math import cos, pi, sin
from pathlib import Path
import struct

import numpy as np
import trimesh
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "assets/public/assets/meshes/infra/sino_da_mare_lod0.glb"


class Scene:
    def __init__(self):
        self.doc = dict(
            asset=dict(version="2.0", generator="Vael / Sino da maré"),
            scene=0,
            scenes=[dict(nodes=[0])],
            nodes=[],
            meshes=[],
            materials=[],
            accessors=[],
            bufferViews=[],
            animations=[],
            extensionsUsed=["KHR_materials_emissive_strength"],
        )
        self.bin = bytearray()
        self.geometry = defaultdict(lambda: defaultdict(list))
        self.root = self.node("SinoDaMare")

    def node(self, name, parent=None, **props):
        i = len(self.doc["nodes"])
        self.doc["nodes"].append(dict(name=name, **props))
        if parent is not None:
            self.doc["nodes"][parent].setdefault("children", []).append(i)
        return i

    def material(self, name, rgb, metal=0, rough=0.5, light=0):
        m = dict(
            name=name,
            pbrMetallicRoughness=dict(
                baseColorFactor=[*rgb, 1], metallicFactor=metal, roughnessFactor=rough
            ),
        )
        if light:
            m.update(
                emissiveFactor=rgb,
                extensions={
                    "KHR_materials_emissive_strength": dict(emissiveStrength=light)
                },
            )
        self.doc["materials"].append(m)
        return len(self.doc["materials"]) - 1

    def array(self, data, kind):
        ix = kind == "indices"
        a = np.asarray(data, dtype="<u4" if ix else "<f4")
        self.bin.extend(bytes(-len(self.bin) % 4))
        views, acc = self.doc["bufferViews"], self.doc["accessors"]
        views.append(dict(buffer=0, byteOffset=len(self.bin), byteLength=a.nbytes))
        self.bin.extend(a.tobytes())
        desc = dict(
            bufferView=len(views) - 1,
            componentType=5125 if ix else 5126,
            count=len(a),
            type="SCALAR" if a.ndim == 1 else f"VEC{a.shape[1]}",
        )
        if kind in ("time", "position"):
            desc.update(
                min=np.atleast_1d(a.min(axis=0)).tolist(),
                max=np.atleast_1d(a.max(axis=0)).tolist(),
            )
        acc.append(desc)
        return len(acc) - 1

    def channel(self, anim, node, path, times, values):
        i = len(anim["samplers"])
        anim["samplers"].append(
            dict(input=self.array(times, "time"), output=self.array(values, "key"))
        )
        anim["channels"].append(dict(sampler=i, target=dict(node=node, path=path)))

    def clip(self, name):
        a = dict(name=name, samplers=[], channels=[])
        self.doc["animations"].append(a)
        return a

    def save(self):
        for node, mats in self.geometry.items():
            primitives = []
            for mat, shapes in mats.items():
                mesh = trimesh.util.concatenate(shapes)
                p = self.array(mesh.vertices, "position")
                n = self.array(mesh.vertex_normals, "normal")
                attributes = dict(POSITION=p, NORMAL=n)
                if mat == skin:
                    x, y, _ = mesh.vertices.T
                    cy = np.interp(x, stations[:, 0], stations[:, 1])
                    ry = np.interp(x, stations[:, 0], stations[:, 2])
                    u = (x - stations[0, 0]) / (stations[-1, 0] - stations[0, 0])
                    v = 0.5 - 0.499 * (y - cy) / ry
                    attributes["TEXCOORD_0"] = self.array(
                        np.stack([u, v], axis=1), "uv"
                    )
                primitives.append(
                    dict(
                        material=mat,
                        attributes=attributes,
                        indices=self.array(mesh.faces.flatten(), "indices"),
                    )
                )
            self.doc["nodes"][node]["mesh"] = len(self.doc["meshes"])
            self.doc["meshes"].append(dict(primitives=primitives))
        self.doc["buffers"] = [dict(byteLength=len(self.bin))]
        chunk = json.dumps(self.doc, separators=(",", ":"), ensure_ascii=True).encode()
        chunk += b" " * (-len(chunk) % 4)
        self.bin.extend(bytes(-len(self.bin) % 4))
        OUT.write_bytes(
            struct.pack("<4sII", b"glTF", 2, 28 + len(chunk) + len(self.bin))
            + struct.pack("<I4s", len(chunk), b"JSON")
            + chunk
            + struct.pack("<I4s", len(self.bin), b"BIN\0")
            + self.bin
        )


s = Scene()
wood = s.material("Carvalho molhado", [0.13, 0.065, 0.038], 0, 0.76)
slate = s.material("Xisto do cais", [0.23, 0.26, 0.28], 0.05, 0.82)
metal = s.material("Bronze de sino", [0.57, 0.28, 0.084], 0.7, 0.32)
gold = s.material("Latão gasto", [0.84, 0.52, 0.19], 0.52, 0.31)
patina = s.material("Azinhavre", [0.055, 0.31, 0.26], 0.47, 0.54)
rope = s.material("Corda encerada", [0.37, 0.22, 0.10], 0, 0.9)
blue = s.material("Lombo de maré", [0.032, 0.21, 0.38], 0.22, 0.34, 0.62)
belly = s.material("Madrepérola lunar", [0.19, 0.61, 0.67], 0.2, 0.4, 0.64)
ink = s.material("Índigo profundo", [0.021, 0.061, 0.16], 0.12, 0.45, 0.26)
aqua = s.material("Fio de luar", [0.18, 0.87, 0.88], 0.1, 0.3, 2.0)
pearl = s.material("Estrela de âmbar", [1, 0.65, 0.23], 0.15, 0.28, 3)
eye = s.material("Olho de obsidiana", [0.008, 0.019, 0.031], 0.15, 0.24)


def orb(node, at, scale, mat, subdivisions=2):
    mesh = trimesh.creation.icosphere(subdivisions=subdivisions)
    mesh.apply_scale(scale)
    mesh.apply_translation(at)
    s.geometry[node][mat].append(mesh)


def rod(node, a, b, radius, mat, sides=9):
    s.geometry[node][mat].append(
        trimesh.creation.cylinder(
            radius=radius, sections=sides, segment=np.asarray([a, b])
        )
    )


def thread(node, points, radius, mat):
    for a, b in zip(points, points[1:]):
        rod(node, a, b, radius, mat)


def revolve(node, rings, material, count=32):
    # Cross sections progress from bottom to top. Returning inside makes the bell hollow.
    verts = [
        (r * cos(2 * pi * i / count), y, r * sin(2 * pi * i / count))
        for y, r in rings
        for i in range(count)
    ]
    for j in range(len(rings) - 1):
        faces = []
        for i in range(count):
            a = j * count + i
            b = j * count + (i + 1) % count
            faces.extend([(a, a + count, b), (b, a + count, b + count)])
        s.geometry[node][material[j]].append(
            trimesh.Trimesh(verts, faces, process=True)
        )


def quat(axis, radians):
    return [*(np.asarray(axis) * sin(radians / 2)), cos(radians / 2)]


frame = s.node("Portico", 0)
for x in [-0.79, 0.79]:
    orb(frame, (x, 0.04, 0), (0.41, 0.22, 0.44), slate, 1)
    rod(frame, (x, 0.07, 0), (x, 2.54, 0), 0.137, wood)
    for y in [0.34, 0.42, 2.26, 2.36]:
        rod(frame, (x, y, 0), (x, y + 0.055, 0), 0.15, patina if y < 1 else gold)
    rod(frame, (x, 1.79, 0), (x * 0.25, 2.46, 0), 0.072, wood)
    orb(frame, (x, 2.55, 0), (0.123, 0.165, 0.123), gold)
rod(frame, (-1.02, 2.48, 0), (1.02, 2.48, 0), 0.138, wood)
for x in [-0.85, -0.77, -0.69, 0.69, 0.77, 0.85]:
    rod(frame, (x, 2.48, 0), (x + 0.043, 2.48, 0), 0.151, rope)
hinge = s.node("Balanco", 0, translation=[0, 2.45, 0])
rod(hinge, (0, -0.22, 0), (0, 0.01, 0), 0.069, metal)
profile = [
    (-1.06, 0.56),
    (-1.02, 0.56),
    (-0.98, 0.51),
    (-0.89, 0.435),
    (-0.66, 0.348),
    (-0.39, 0.309),
    (-0.36, 0.315),
    (-0.33, 0.312),
    (-0.31, 0.298),
    (-0.25, 0.258),
    (-0.21, 0.10),
    (-0.21, 0.02),
]
revolve(
    hinge,
    profile,
    [gold, metal, gold, metal, metal, patina, gold, patina, metal, metal, gold],
)
revolve(
    hinge,
    list(
        reversed(
            [
                (-1.06, 0.56),
                (-1.055, 0.50),
                (-0.92, 0.397),
                (-0.45, 0.26),
                (-0.28, 0.09),
            ]
        )
    ),
    [metal] * 4,
)
rod(hinge, (0, -0.34, 0), (0, -1.18, 0), 0.035, rope)
orb(hinge, (0, -1.16, 0), (0.119, 0.139, 0.119), metal)
rod(hinge, (0, -1.23, 0), (0, -1.52, 0), 0.025, rope)
orb(hinge, (0, -1.52, 0), (0.05, 0.075, 0.05), gold)
for side in [-1, 1]:
    points = [
        (-0.24, -0.66, 0.235 * side),
        (-0.13, -0.60, 0.32 * side),
        (0.02, -0.62, 0.35 * side),
        (0.16, -0.67, 0.304 * side),
        (0.23, -0.63, 0.26 * side),
    ]
    thread(hinge, points, 0.018, gold)
    for x, y, z in points[1:4]:
        orb(hinge, (x, y, z), (0.032, 0.033, 0.03), aqua, 1)

# Cetacean body along X. Each station records centre height, vertical and lateral radii.
stations = np.array(
    [
        [-4.12, 0.06, 0.055, 0.06],
        [-3.72, 0.07, 0.17, 0.23],
        [-3.24, 0.13, 0.26, 0.35],
        [-2.61, 0.17, 0.43, 0.49],
        [-1.85, 0.14, 0.68, 0.70],
        [-0.96, 0.12, 0.94, 0.91],
        [0, 0.11, 1.03, 1.05],
        [0.86, 0.09, 1.01, 1.09],
        [1.71, 0.08, 0.90, 1.01],
        [2.43, 0.12, 0.72, 0.87],
        [2.99, 0.21, 0.51, 0.65],
        [3.35, 0.28, 0.32, 0.41],
        [3.52, 0.31, 0.09, 0.12],
        [3.55, 0.31, 0.017, 0.025],
    ]
)
gone = [0.0001] * 3
flight = s.node("Mare", 0, translation=[-8, 5.8, 17])
body = s.node("Baleia", flight, scale=gone)


def surface(x, t):
    cy, ry, rz = [np.interp(x, stations[:, 0], stations[:, i]) for i in [1, 2, 3]]
    return np.array([x, cy + ry * cos(t), rz * sin(t)])


count = 40
verts = [surface(x, 2 * pi * i / count) for x in stations[:, 0] for i in range(count)]


# Pearlescent tide lines follow the animal along its flank. A shared projected
# texture keeps the markings continuous across both sides of the backbone.
def paint_flank():
    width, height = 640, 256
    v, u = (
        np.mgrid[0:height, 0:width] / np.array([height - 1, width - 1])[:, None, None]
    )
    swash = 0.043 * np.sin(7 * u + 0.8) + 0.022 * np.sin(13 * u)
    depth = v + swash * np.sin(pi * v)
    palette = np.array(
        [
            [0.21, 0.64, 0.74],
            [0.14, 0.46, 0.60],
            [0.10, 0.25, 0.43],
            [0.12, 0.22, 0.38],
            [0.21, 0.43, 0.55],
            [0.47, 0.74, 0.77],
            [0.75, 0.88, 0.81],
            [0.31, 0.54, 0.64],
        ]
    )
    levels = [0, 0.11, 0.27, 0.48, 0.58, 0.65, 0.83, 1.0]
    rgb = np.stack([np.interp(depth, levels, c) for c in palette.T], axis=-1)
    spiral = 27 * (depth - 0.085 * np.sin(7 * u + v * 2.3)) + 0.27 * np.sin(
        22 * u + v * 4
    )
    ripples = np.exp(-(((np.sin(spiral)) / 0.14) ** 2))
    rgb += (
        ripples * (0.5 + 0.5 * np.cos(31 * u + v * 5)) ** 2 * np.sin(pi * v) * 0.20
    )[:, :, None] * [0.35, 0.92, 1]
    rng = np.random.default_rng(19)
    for _ in range(270):
        x, y = rng.uniform([0.045, 0.075], [0.96, 0.63])
        radius = rng.uniform(0.65, 2.05)
        spark = np.clip(
            1 - (((u - x) * width / radius) ** 2 + ((v - y) * height / radius) ** 2),
            0,
            1,
        )
        rgb += spark[:, :, None] * [0.53, 0.62, 0.61]
    out = io.BytesIO()
    Image.fromarray((np.clip(rgb, 0, 1) * 255).astype("uint8")).save(out, format="PNG")
    return out.getvalue()


skin = s.material("Mapa das correntes", [0.93, 0.97, 1], 0.12, 0.35, 1.4)
image = paint_flank()
s.bin.extend(bytes(-len(s.bin) % 4))
s.doc["images"] = [dict(bufferView=len(s.doc["bufferViews"]), mimeType="image/png")]
s.doc["bufferViews"].append(
    dict(buffer=0, byteOffset=len(s.bin), byteLength=len(image))
)
s.bin.extend(image)
s.doc["textures"] = [dict(source=0)]
s.doc["materials"][skin]["pbrMetallicRoughness"]["baseColorTexture"] = dict(index=0)
s.doc["materials"][skin]["emissiveTexture"] = dict(index=0)
triangles = []
for j in range(len(stations) - 1):
    for i in range(count):
        a = j * count + i
        b = j * count + (i + 1) % count
        triangles.extend([(a, b, a + count), (b, b + count, a + count)])
hull = trimesh.Trimesh(verts, triangles, process=True)
if hull.volume < 0:
    hull.invert()
s.geometry[body][skin].append(hull)


def fin(node, outline, thickness, top=blue, under=belly):
    ring = np.asarray(outline)
    mid = ring.mean(axis=0)
    vertices = [*ring, mid + [0, thickness, 0], mid - [0, thickness, 0]]
    for cap, mat in [(0, top), (1, under)]:
        idx = len(ring) + cap
        triangles = [(i, (i + 1) % len(ring), idx) for i in range(len(ring))]
        m = trimesh.Trimesh(vertices, triangles, process=True)
        if (m.face_normals[:, 1].mean() > 0) == bool(cap):
            m.invert()
        s.geometry[node][mat].append(m)


tail = s.node("NadadeiraCaudal", body, translation=[-3.88, 0.07, 0])
for side in [-1, 1]:

    def sided(outline):
        return [(x, y, z * side) for x, y, z in outline]

    fin(
        tail,
        sided(
            [
                (0.32, 0, 0.03),
                (0.07, 0.05, 0.51),
                (-0.40, 0.13, 1.12),
                (-1.17, 0.32, 1.75),
                (-1.01, 0.15, 1.03),
                (-0.79, 0.03, 0.54),
                (-0.84, -0.03, 0.19),
                (-0.64, 0.00, 0),
            ]
        ),
        0.095,
    )
    fin(
        body,
        sided(
            [
                (0.79, -0.16, 0.85),
                (0.7, -0.28, 1.17),
                (0.13, -0.5, 1.76),
                (-0.78, -0.79, 2.44),
                (-1.39, -0.87, 2.66),
                (-1.16, -0.64, 2.13),
                (-0.69, -0.39, 1.54),
                (-0.25, -0.21, 0.9),
            ]
        ),
        0.12,
    )
    thread(
        tail,
        sided([(-0.06, 0.087, 0.46), (-0.43, 0.18, 1.04), (-0.90, 0.271, 1.51)]),
        0.015,
        aqua,
    )
    thread(
        body,
        sided(
            [
                (0.46, -0.22, 1.12),
                (-0.02, -0.39, 1.63),
                (-0.76, -0.71, 2.35),
                (-1.18, -0.81, 2.54),
            ]
        ),
        0.02,
        aqua,
    )
    for angle in [1.94, 2.10, 2.27, 2.44, 2.61, 2.78]:
        points = [
            surface(x, side * (angle + 0.13 * cos((x - 0.2) * 0.76)))
            * [1, 1.018, 1.018]
            for x in np.linspace(-0.32, 3.18, 23)
        ]
        thread(body, points, 0.014, ink)
    lip = [
        surface(x, side * (1.68 + 0.17 * sin((x - 0.8) * 1.05))) * [1, 1.024, 1.024]
        for x in np.linspace(0.82, 3.46, 29)
    ]
    thread(body, lip, 0.026, ink)
    point = surface(1.05, side * 1.47)
    orb(body, point, (0.137, 0.129, 0.065), gold)
    orb(body, point + [0.012, 0, 0.047 * side], (0.088, 0.087, 0.048), eye)
    orb(body, point + [0.047, 0.031, 0.080 * side], (0.023, 0.024, 0.012), pearl, 1)
fin(
    body,
    [
        (-2.39, 0.52, 0),
        (-2.22, 1.11, -0.045),
        (-1.77, 0.87, 0),
        (-1.27, 0.85, 0.03),
        (-1.82, 0.56, 0.05),
    ],
    0.064,
)

# A moving constellation follows each flank. Loose stars converge on the near flank first.
anchors = []
for side in [-1, 1]:
    constellation = [
        (-3.1, 1.36),
        (-2.21, 0.82),
        (-1.54, 1.25),
        (-0.76, 0.79),
        (0.02, 1.19),
        (0.63, 0.72),
        (1.35, 1.06),
        (2.2, 0.71),
        (2.68, 1.13),
    ]
    points = [
        surface(x, angle * side) * [1, 1.053, 1.053] for x, angle in constellation
    ]
    thread(body, points, 0.016, aqua)
    for i, p in enumerate(points):
        orb(
            body, p, (0.068, 0.071, 0.063) if i % 3 else (0.086, 0.089, 0.081), pearl, 1
        )
    if side == -1:
        anchors = points

idle = s.clip("idle")
s.channel(idle, body, "scale", [0, 1], [gone, gone])
show = s.clip("canto")
times = [0, 1.7, 3.8, 5.2, 7.3, 9.1, 11.6, 13.4, 15.1, 16.0]
# The channel runs along world Z=215. The apparition comes from the water,
# passes the approach banners at its crest, and returns to the channel.
path = [
    [-10, 0.3, 50.5],
    [-10, 1.1, 48.0],
    [-9, 5.6, 35.0],
    [-7.7, 8.7, 24.0],
    [-6.3, 11.0, 21.0],
    [-5.1, 11.95, 22.0],
    [-3.2, 11.6, 25.0],
    [-1.8, 8.8, 33.0],
    [-0.5, 2.0, 47.0],
    [0, -0.4, 51.0],
]
s.channel(show, flight, "translation", times, path)
s.channel(
    show,
    flight,
    "rotation",
    times,
    [
        quat([0, 0, 1], a)
        for a in [0, 0.12, 0.25, 0.20, 0.14, 0.065, -0.15, -0.36, -0.52, -0.55]
    ],
)
s.channel(
    show,
    body,
    "scale",
    [0, 3.6, 4.2, 5.3, 6.1, 12.7, 13.9, 14.8, 16],
    [[k] * 3 for k in [0.0001, 0.0001, 0.13, 0.84, 1, 1, 0.71, 0.0001, 0.0001]],
)
wave = np.linspace(0, 16, 65)
s.channel(
    show, tail, "rotation", wave, [quat([0, 0, 1], 0.30 * sin(1.55 * t)) for t in wave]
)
s.channel(
    show,
    hinge,
    "rotation",
    np.linspace(0, 5, 81),
    [
        quat([1, 0, 0], 0.45 * sin(8 * t) * np.exp(-t * 1.03))
        for t in np.linspace(0, 5, 81)
    ],
)
rng = np.random.default_rng(318)
for i, p in enumerate(anchors):
    light = s.node(f"Faisca{i}", flight, scale=gone)
    orb(light, [0, 0, 0], [0.074, 0.14, 0.072], pearl, 1)
    start = p + rng.uniform([-4, -2, -3], [4, 0.6, 3])
    bend = (start + p) / 2 + [0.62, 0.94, -0.25]
    t = i * 0.085
    s.channel(
        show,
        light,
        "translation",
        [0, 1 + t, 2.7 + t, 4.15 + t, 6.2, 12.9, 14.5 + t, 16],
        [start, start, bend, p, p, p, p + [0.23, 1.15, 0.21], p + [0.42, 1.45, 0.32]],
    )
    s.channel(
        show,
        light,
        "scale",
        [0, 0.4 + t, 1.5 + t, 4.4 + t, 6.1, 12.9, 14.2, 15.25 + t, 16],
        [
            [k] * 3
            for k in [0.0001, 0.0001, 1.55, 1.15, 0.0001, 0.0001, 1.15, 0.0001, 0.0001]
        ],
    )
s.save()
