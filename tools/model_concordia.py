"""Exporta os GLBs de Concórdia. Usa -- --pranchas para renderizar o primeiro uso."""
from math import cos, sin, tau
from pathlib import Path
from random import Random
import sys

import bpy
from mathutils import Vector

OUTPUT = Path(__file__).resolve().parents[1] / "assets/public/assets/meshes/concordia"
RANDOM = Random(712)
bpy.ops.wm.read_factory_settings(use_empty=True)


def material(name, rgb):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*rgb, 1)
    mat.use_nodes = True
    shader = mat.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (*rgb, 1)
    shader.inputs["Roughness"].default_value = 0.87
    return mat


WOOD = [material(f"Pinho {i}", color) for i, color in enumerate([
    (0.23, 0.115, 0.052), (0.30, 0.165, 0.080), (0.37, 0.207, 0.095),
    (0.25, 0.142, 0.074),
])]
STONE = [material(f"Cantaria {i}", color) for i, color in enumerate([
    (0.32, 0.29, 0.23), (0.38, 0.34, 0.28), (0.42, 0.38, 0.30),
    (0.27, 0.26, 0.20),
])]
ROOF = [material(f"Telha {i}", color) for i, color in enumerate([
    (0.38, 0.135, 0.078), (0.29, 0.106, 0.065), (0.34, 0.114, 0.067),
])]
ROPE = material("Cânhamo", (0.49, 0.35, 0.18))
IRON = material("Ferro batido", (0.084, 0.079, 0.070))


def clear():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)


def bevel(obj, amount):
    if amount:
        mod = obj.modifiers.new("Chanfro de enxó", "BEVEL")
        mod.width, mod.segments = amount, 1


def box(name, loc, dims, mat, chamfer=0.025, angle=0):
    bpy.ops.mesh.primitive_cube_add(size=1, location=loc)
    obj = bpy.context.object
    obj.name = name
    obj.dimensions = dims
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.rotation_euler.z = angle
    obj.data.materials.append(mat)
    bevel(obj, chamfer)
    return obj


def beam(name, start, end, width, depth, mat, chamfer=0.025):
    a, b = Vector(start), Vector(end)
    obj = box(name, (a + b) / 2, (width, depth, (b - a).length), mat, chamfer)
    obj.rotation_euler = (b - a).to_track_quat("Z", "Y").to_euler()
    return obj


def segment(name, a, b, top_a, top_b, bottom_a, bottom_b, width, mat, chamfer=0):
    """Prisma longitudinal no eixo Y do Blender."""
    x = width / 2
    vertices = [(-x, a, bottom_a), (x, a, bottom_a), (x, b, bottom_b),
                (-x, b, bottom_b), (-x, a, top_a), (x, a, top_a),
                (x, b, top_b), (-x, b, top_b)]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], [(3,2,1,0), (4,5,6,7), (0,1,5,4),
                                     (1,2,6,5), (2,3,7,6), (3,0,4,7)])
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    mesh.materials.append(mat)
    bevel(obj, chamfer)
    return obj


def export(name):
    OUTPUT.mkdir(parents=True, exist_ok=True)
    groups = {}
    for obj in list(bpy.context.scene.objects):
        # Aplica o chanfro antes de consolidar as peças do mesmo material.
        bpy.context.view_layer.objects.active = obj
        for mod in list(obj.modifiers):
            bpy.ops.object.modifier_apply(modifier=mod.name)
        if name.endswith("_collision"):
            obj.data.materials.clear()
            obj.data.materials.append(STONE[0])
        groups.setdefault(obj.data.materials[0].name, []).append(obj)
    for material_name, objects in groups.items():
        bpy.ops.object.select_all(action="DESELECT")
        for obj in objects:
            obj.select_set(True)
        bpy.context.view_layer.objects.active = objects[0]
        if len(objects) > 1:
            bpy.ops.object.join()
        bpy.context.object.name = material_name
    bpy.ops.export_scene.gltf(filepath=str(OUTPUT / f"{name}.glb"),
                              export_format="GLB", export_apply=True)


def bridge(collision):
    # Y += norte. Cotas relativas a 30 m. O canal fica sem pilares.
    profile = [(-25, 0.15), (-13, 3.55), (-7, 5.1), (0, 5.65),
               (7, 5.5), (13, 4.65), (25, 1.35)]
    for (a, ya), (b, yb) in zip(profile, profile[1:]):
        segment("Tabuleiro", a, b, ya, yb, ya - 0.47, yb - 0.47, 4.8,
                STONE[1], 0 if collision else 0.035)
        for side in [-1, 1]:
            rail = segment("Parapeito", a, b, ya + 0.88, yb + 0.88, ya - 0.03,
                           yb - 0.03, 0.39, STONE[0], 0 if collision else 0.045)
            rail.location.x = side * 2.14
            if not collision:
                cap = segment("Capeamento", a, b, ya + 0.96, yb + 0.96,
                              ya + 0.83, yb + 0.83, 0.53, STONE[2], 0.02)
                cap.location.x = side * 2.14
                n = round((b - a) / 1.3)
                for i in range(n):
                    f = (i + 0.5) / n
                    box("Junta", (side * 2.34, a + (b - a) * f,
                                 ya + (yb - ya) * f + 0.45),
                        (0.012, 0.025, 0.65), STONE[3], 0)
    arch = [(-16, 2.7, -5.0), (-13, 3.55, -1.1), (-10, 4.33, 1.0),
            (-7, 5.1, 3.27), (-4, 5.34, 4.17), (0, 5.65, 4.52),
            (4, 5.56, 4.17), (7, 5.5, 3.27), (10, 5.08, 1.0),
            (13, 4.65, -1.1), (16, 3.82, -5.0)]
    for (a, ya, la), (b, yb, lb) in zip(arch, arch[1:]):
        segment("Arco", a, b, ya - 0.15, yb - 0.15, la, lb, 4.7,
                RANDOM.choice(STONE) if not collision else STONE[0], 0.028 if not collision else 0)


def shipyard(stage, collision=False):
    # A rampa pousa na margem a leste. O cais aponta para o lago a oeste.
    for xa, xb, za, zb in [(-3.6, 3.2, 0, 0), (3.2, 6.5, 0, 2.15)]:
        for i in range(round((xb - xa) / 0.29)) if not collision else range(1):
            n = round((xb - xa) / 0.29) if not collision else 1
            a, b = i / n, (i + 1) / n
            segment_obj = segment(
                "Estrado", xa + (xb - xa) * a, xa + (xb - xa) * b,
                za + (zb - za) * a, za + (zb - za) * b,
                za + (zb - za) * a - 0.18, za + (zb - za) * b - 0.18,
                5.3, RANDOM.choice(WOOD), 0.012 if not collision else 0)
            segment_obj.rotation_euler.z = -1.57079632679
    if stage >= 3:
        for a, b, za, zb in [(-7.2, -3.6, -0.94, 0), (-12.4, -7.2, -0.94, -0.94)]:
            plank = segment("Cais", a, b, za, zb, za - 0.21, zb - 0.21, 2.8,
                            WOOD[1], 0.022 if not collision else 0)
            plank.rotation_euler.z = -1.57079632679
    for x in [-3.35, 2.85]:
        for y in [-2.34, 2.34]:
            box("Pé de encaixe", (x, y, -1.3), (0.48, 0.46, 2.7), STONE[1])
            box("Pilar", (x, y, 1.23), (0.25, 0.27, 3.05), WOOD[0])
            for dx in [0.8 if x < 0 else -0.8]:
                beam("Mão francesa", (x, y, 1.82), (x + dx, y, 2.73),
                     0.16, 0.17, WOOD[2])
    if stage < 2 or collision:
        return
    for x in [-3.7, 0.0, 3.2]:
        for side in [-1, 1]:
            beam("Asna", (x, 2.75 * side, 2.67), (x, 0, 4.15), 0.19, 0.21, WOOD[0])
        beam("Tirante", (x, -2.45, 2.70), (x, 2.45, 2.70), 0.22, 0.20, WOOD[1])
    beam("Cumeeira", (-3.95, 0, 4.13), (3.5, 0, 4.13), 0.20, 0.23, WOOD[0])
    for sign in ([-1] if stage == 2 else [-1, 1]):
        for i in range(6):
            y0, y1 = sign * i * 0.49, sign * (i + 1) * 0.49
            for j in range(10):
                x = -3.72 + j * 0.76 + (i % 2) * 0.13
                inner = 4.17 - i * 0.255
                outer = inner - 0.25
                tile = segment("Telha", min(y0, y1), max(y0, y1),
                               inner if sign > 0 else outer, outer if sign > 0 else inner,
                               (inner if sign > 0 else outer) - 0.075,
                               (outer if sign > 0 else inner) - 0.075,
                               0.755, ROOF[(i + j) % 3], 0.012)
                tile.location.x = x
    for x in [-1.8, 0.4, 2.4]:
        box("Cavalete", (x, 0, 0.30), (0.21, 1.60, 0.60), WOOD[0])
    beam("Quilha de pinho", (-2.7, 0, 0.69), (2.8, 0, 0.78), 0.25, 0.28, WOOD[2])
    for x, width in [(-2.25, 0.46), (-1.30, 0.86), (0, 0.98), (1.3, 0.83), (2.35, 0.45)]:
        for side in [-1, 1]:
            beam("Caverna da quilha", (x, 0, 0.73), (x, side * width, 1.15),
                 0.13, 0.13, WOOD[1])
    if stage == 3:
        for x in [-5, -8.5, -11.9]:
            for y in [-1.17, 1.17]:
                beam("Estaca do cais", (x, y, -3.1), (x, y, -0.42), 0.28, 0.31, WOOD[0])
        box("Bancada", (1.9, -1.66, 0.9), (2.15, 0.83, 0.16), WOOD[2])
        for x in [1.0, 2.8]:
            box("Pé da bancada", (x, -1.66, 0.45), (0.19, 0.62, 0.90), WOOD[0])
        box("Enxó", (2.24, -1.74, 1.05), (0.15, 0.21, 0.08), IRON)
        box("Cabo", (2.50, -1.73, 1.03), (0.45, 0.055, 0.06), WOOD[1])


def stakes():
    for x, y in [(-1.55, -1.05), (-1.55, 1.05), (1.55, 1.05), (1.55, -1.05)]:
        beam("Estaca", (x, y, -0.17), (x + 0.04, y - 0.03, 0.72), 0.11, 0.11, WOOD[2])
        bpy.ops.mesh.primitive_cylinder_add(vertices=11, radius=0.25, depth=0.025,
                                            location=(x, y, 0.012))
        bpy.context.object.name = "Buraco de estaca"
        bpy.context.object.data.materials.append(STONE[3])
    for a, b in [((-1.55, -1.05), (-1.55, 1.05)), ((-1.55, 1.05), (1.55, 1.05)),
                 ((1.55, 1.05), (1.55, -1.05))]:
        center = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, 0.47)
        beam("Cordel", (*a, 0.60), center, 0.026, 0.026, ROPE, 0)
        beam("Cordel", center, (*b, 0.60), 0.026, 0.026, ROPE, 0)


def sign():
    box("Pé da placa", (0, 0, 0.69), (0.14, 0.12, 1.60), WOOD[0])
    for i in range(3):
        box("Tábua da placa", (0, 0.075, 1.14 + i * 0.155), (0.88, 0.095, 0.15),
            WOOD[i], 0.012)
    for x in [-0.31, 0.31]:
        box("Prego", (x, 0.132, 1.29), (0.031, 0.022, 0.028), IRON, 0.004)


def frame():
    for x in [-1.55, 1.55]:
        for y in [-1.05, 1.05]:
            box("Soco", (x, y, 0.18), (0.40, 0.40, 0.39), STONE[1])
            beam("Prumo", (x, y, 0.35), (x, y, 2.25), 0.17, 0.16, WOOD[0])
    for y in [-1.05, 1.05]:
        beam("Frechal", (-1.72, y, 2.19), (1.72, y, 2.19), 0.18, 0.22, WOOD[1])
        for z in [0.59, 0.82, 1.05, 1.28]:
            beam("Parede de tábua", (-1.55, y, z), (1.55, y, z), 0.10, 0.22, WOOD[2])
    for x in [-1.55, 1.55]:
        beam("Frechal", (x, -1.13, 2.19), (x, 1.13, 2.19), 0.18, 0.22, WOOD[0])
    beam("Escora", (-1.50, 1.08, 0.43), (1.50, 1.08, 2.1), 0.12, 0.14, WOOD[0])


def gate():
    for x in [-4.4, 4.4]:
        box("Soco do portão", (x, 0, 0.23), (0.76, 0.73, 0.56), STONE[1])
        beam("Ombreira", (x, 0, 0.26), (x, 0, 3.75), 0.45, 0.54, WOOD[0])
        beam("Escora", (x, 0, 2.20), (x * 0.72, 0, 3.57), 0.23, 0.24, WOOD[1])
    beam("Verga", (-4.75, 0, 3.64), (4.75, 0, 3.64), 0.46, 0.36, WOOD[1])
    beam("Areia e cal", (-4.65, 0, 3.95), (4.65, 0, 3.95), 0.28, 0.24, WOOD[2])


def awning():
    canvas = material("Linho cru", (0.48, 0.33, 0.17))
    for x in [-2.1, 2.1]:
        for y in [-1.65, 1.65]:
            beam("Pau da lona", (x, y, 0), (x, y, 2.77 if y > 0 else 2.2),
                 0.12, 0.14, WOOD[0])
            beam("Espia", (x, y, 2.2), (x * 1.35, y * 1.15, 0.12),
                 0.025, 0.025, ROPE, 0)
    for i in range(7):
        obj = segment("Lona", -1.95, 1.95, 2.18, 2.85, 2.14, 2.81,
                      0.68, canvas, 0.008)
        obj.location.x = (i - 3) * 0.67


def ridge():
    beam("Trave de fecho", (-1.64, 0, 2.70), (1.64, 0, 2.70), 0.18, 0.16, WOOD[2])
    for x in [-1.55, 0, 1.55]:
        for y in [-1.05, 1.05]:
            beam("Asna", (x, y, 2.18), (x, 0, 2.7), 0.12, 0.14, WOOD[1])
    for i in range(5):
        beam("Ripa", (-1.63, i * 0.26, 2.7 - i * 0.13),
             (1.63, i * 0.26, 2.7 - i * 0.13), 0.14, 0.09, WOOD[2])


def bucket():
    for i in range(12):
        a = i * tau / 12
        beam("Aduela", (0.16 * cos(a), 0.16 * sin(a), 0.03),
             (0.23 * cos(a), 0.23 * sin(a), 0.43), 0.12, 0.05, WOOD[i % 4], 0.009)
    for z, r in [(0.10, 0.183), (0.34, 0.222)]:
        bpy.ops.mesh.primitive_torus_add(major_radius=r, minor_radius=0.013,
                                         major_segments=16, minor_segments=6,
                                         location=(0, 0, z))
        bpy.context.object.data.materials.append(IRON)
    beam("Corda do balde", (0, 0, 0.48), (0, 0, 1.2), 0.027, 0.027, ROPE, 0)


def axle():
    beam("Eixo de freixo", (-1.2, 0, 0.5), (1.2, 0, 0.5), 0.18, 0.19, WOOD[1])
    for x in [-1.08, 1.08]:
        beam("Manga do eixo", (x - 0.13, 0, 0.5), (x + 0.13, 0, 0.5),
             0.21, 0.22, IRON)


def window():
    glass = material("Vidraça azul", (0.23, 0.48, 0.57))
    box("Vidraça", (0, 0, 0.77), (0.75, 0.075, 1.13), glass, 0.04)
    for x in [-0.4, 0, 0.4]:
        box("Montante da vidraça", (x, -0.01, 0.77), (0.075, 0.145, 1.24), WOOD[0])
    for z in [0.17, 0.76, 1.37]:
        box("Travessa da vidraça", (0, -0.01, z), (0.87, 0.145, 0.08), WOOD[1])


def bell():
    bronze = material("Bronze fosco", (0.38, 0.245, 0.106))
    bpy.ops.mesh.primitive_cone_add(vertices=14, radius1=0.27, radius2=0.14,
                                    depth=0.42, location=(0, 0, 1.66))
    bpy.context.object.data.materials.append(bronze)
    beam("Badalo", (0, 0, 1.34), (0, 0, 1.53), 0.064, 0.066, IRON)
    beam("Suporte", (0, 0, 1.85), (0, 0, 2.15), 0.085, 0.083, WOOD[1])


def ballast():
    for i in range(7):
        x, y = RANDOM.uniform(-0.55, 0.55), RANDOM.uniform(-0.34, 0.34)
        box("Pedra de lastro", (x, y, 0.16), (0.49, 0.34, 0.34),
            STONE[i % 4], 0.085, angle=i * 0.8)



def render_panels():
    """Pranchas de primeira utilização (Blender Workbench, luz e contorno mate)."""
    import json

    root = Path(__file__).resolve().parents[1]
    output = root / "assets/public/cutscenes/concordia"
    output.mkdir(parents=True, exist_ok=True)
    catalog = json.loads((root / "data/concordia.json").read_text())["sites"]
    clay = material("Barro cru", (0.37, 0.25, 0.13))
    flint = material("Cal quente", (0.63, 0.52, 0.38))
    glow = material("Brasa", (0.91, 0.26, 0.045))
    cloth = material("Lona ferrugem", (0.45, 0.16, 0.09))
    water = material("Água de sombra", (0.16, 0.33, 0.37))

    def cylinder(name, at, radius, depth, mat):
        bpy.ops.mesh.primitive_cylinder_add(vertices=14, radius=radius, depth=depth, location=at)
        obj = bpy.context.object
        obj.name = name
        obj.data.materials.append(mat)
        bevel(obj, 0.025)
        return obj

    def fire(at):
        x, y, z = at
        for i in range(12):
            a = i * tau / 12
            box("Pedra da lareira", (x + cos(a) * 0.8, y + sin(a) * 0.8, z + 0.16),
                (0.49, 0.43, 0.33), STONE[i % 4], 0.08, a)
        for i in range(6):
            angle = i * 1.1
            box("Lenha", (x, y, z + 0.15 + i * 0.05), (1.13, 0.15, 0.14),
                WOOD[i % 4], 0.025, angle)
        for dx, dy, scale in [(-0.2, 0.04, .3), (0.2, 0, .38), (0.04, .24, .2)]:
            bpy.ops.mesh.primitive_cone_add(vertices=5, radius1=scale / 2,
                radius2=0, depth=scale * 2.1, location=(x + dx, y + dy, z + .38 + scale * .55))
            bpy.context.object.data.materials.append(glow)

    def bench(at, dims):
        x, y, z = at
        w, d, h = dims
        box("Superfície de trabalho", (x, y, z + h), (w, d, 0.14), WOOD[2])
        for dx in [-0.43*w, 0.43*w]:
            for dy in [-0.37*d, 0.37*d]:
                box("Perna", (x + dx, y + dy, z + h/2), (0.16, 0.16, h), WOOD[0])

    def wagon():
        axle()
        for x in [-1.08, 1.08]:
            obj = cylinder("Roda de carroça", (x, 0, 0.57), 0.57, 0.13, WOOD[0])
            obj.rotation_euler.y = tau / 4
        box("Caixa da carroça", (0, 0.18, 0.90), (1.83, 2.2, 0.32), WOOD[2])
        for y in [-0.9, 1.23]:
            box("Tábua da carga", (0, y, 1.22), (1.80, 0.13, 0.54), WOOD[1])
        for x in [-0.77, 0.77]:
            beam("Vara", (x, -0.64, 0.93), (x / 2, -2.38, 0.71), 0.14, 0.16, WOOD[1])
        for x, y, z in [(-.44,.5,1.38), (.41,.33,1.44), (.40,-.51,1.25)]:
            box("Carga", (x, y, z), (.67,.64,.64), WOOD[3], .071)

    framing = {
        "house": ((5.7,-6.7,4.0), (0,0,.65), 6.0),
        "well": ((4.9,-6.6,4.25), (0,0,.65), 5.8),
        "gate": ((6.8,-9.0,4.9), (0,0,1.65), 12.6),
        "barn": ((6,-8,6.4), (0,0,.9), 7.5),
        "forge": ((4.6,-6.1,4.2), (0,0,.62), 5.6),
        "market": ((6.5,-9,5.9), (0,0,1.1), 8.5),
        "watch": ((4.3,-6.1,3.0), (0,0,1.25), 5.1),
        "hall": ((7.7,-8,5.7), (0,0,1.02), 8.1),
        "chapel": ((4.2,-6.5,3.6), (0,0,1.3), 5.2),
        "shipyard": ((-10,-14,9), (-4,0,0.6), 22),
        "approach": ((4.6,-7,4.4), (0,-.18,1.0), 7.2),
    }
    for site in catalog:
        clear()
        kind = site["id"]
        floor_z = -1.6 if kind == "shipyard" else -0.23
        box("Solo", (0, 0, floor_z), (60, 60, .38), clay, 0)
        if kind == "house":
            fire((-0.78, 0, 0))
            bench((1.1, 0.7, 0), (1.5, 1.0, 0.53))
            box("Lã dobrada", (1.0, 0.72, 0.65), (1.04, .64, .21), cloth, .09)
            box("Caixa de mapas", (0.83, -1.02, 0.24), (.9,.62,.49), WOOD[2], .085)
        elif kind == "well":
            for k in range(3):
                for i in range(13):
                    a = (i + k * 0.5) * tau / 13
                    box("Cantaria do poço", (cos(a),sin(a), .17+k*.30), (.33,.54,.33),
                        STONE[(i+k)%4], .037, a)
            bpy.ops.mesh.primitive_cylinder_add(vertices=48, radius=.86, depth=.025,
                                               location=(0,0,.34))
            bpy.context.object.data.materials.append(water)
            before = set(bpy.context.scene.objects)
            bucket()
            for obj in set(bpy.context.scene.objects) - before:
                obj.location += Vector((0.73, -0.77, 0.78))
        elif kind == "gate":
            gate()
            obj = box("Madeiro da pista", (-2.9, -.55, 1.24), (1.3,.16,.32), WOOD[2])
            obj.rotation_euler.y = -.18
            for x in [-5.4, 5.4]:
                box("Cerca", (x,0,1.0), (2.2,.24,.96), WOOD[1])
        elif kind == "barn":
            bench((-1.2, 0.55, 0), (2.5, 1.25, 1.0))
            for x,y,z in [(-1.3,.6,1.25),(-.4,.45,1.26),(1.1,.4,.38),(1.1,.4,1.10),(.26,-1,.35)]:
                box("Caixote de provisões", (x,y,z), (.81,.76,.7), WOOD[1], .058)
            for x in [-1.55,-.65,.1]:
                beam("Reserva de lenha", (x,-1.38,.55), (x,-.38,.47), .26,.28, WOOD[0])
        elif kind == "forge":
            box("Soco da bigorna", (.4,0,.37), (.67,.54,.75), WOOD[0], .065)
            box("Bigorna", (.4,0,.93), (1.34,.51,.32), IRON, .10)
            beam("Prego em têmpera", (.16,-.2,1.15), (.81,.16,1.15), .074,.078, glow, .01)
            beam("Cabo de martelo", (.88,-.45,.91), (1.56,-.71,.75), .075,.071, WOOD[2])
            box("Cabeça do martelo", (.96,-.41,.98), (.35,.18,.25), IRON)
            fire((-1.14,.5,0))
        elif kind == "market":
            awning()
            bench((.22, .4, 0), (3.35, .96, .93))
            for x in [-1.05,-.22,.65,1.45]:
                box("Mercadoria", (x,.37,1.15), (.61,.65,.35), WOOD[2], .076)
            for i in range(12):
                x,y = RANDOM.uniform(-1.37,1.65), RANDOM.uniform(.11,.63)
                cylinder("Maçã de banca", (x,y,1.41), .106, .15, cloth)
        elif kind == "watch":
            for x in [-.85,.85]:
                box("Ombreira de cantaria", (x,0,1.18), (.73,.66,2.39), STONE[1], .10)
            box("Peitoril", (0,0,.37), (2.17,.86,.51), STONE[2], .045)
            for x in [-.64,.64]:
                box("Travessa alta", (x,0,2.20), (1.4,.63,.47), WOOD[0])
            for obj in list(bpy.context.scene.objects):
                if obj.name != "Solo": obj.location.z += .12
            before = set(bpy.context.scene.objects)
            window()
            for obj in set(bpy.context.scene.objects) - before: obj.location.z += .59
        elif kind == "hall":
            bench((0,0,0), (4.6, 1.64, .94))
            bench((0,-1.22,0), (4.2,.30,.49))
            bench((0,1.22,0), (4.2,.30,.49))
            box("Mapa aberto", (0,0,1.031), (2.1,1.12,.055), flint, .02, .064)
            for x in [-.7,-.23,.32,.7]:
                box("Nota do Coro", (x,.18,1.09), (.23,.42,.055), water, .043, x)
        elif kind == "chapel":
            for x in [-.54,.54]:
                box("Pilar do sino", (x,.25,1.035), (.35,.36,2.10), STONE[0], .055)
            box("Verga", (0,.25,2.13), (1.6,.42,.31), STONE[2], .06)
            bell()
            for x in [-.9, .91]:
                cylinder("Vela de cera", (x,-.7,.58), .09, .31, flint)
                cylinder("Pavio aceso", (x,-.7,.77), .040, .08, glow)
        elif kind == "shipyard":
            shipyard(3)
            box("Margem", (9,0,-.7), (12,22,1.25), clay, .2)
            box("Espelho do lago", (-10,0,-.98), (26,32,.08), water, 0)
            for i in range(8):
                width = .45 + 1.1 * sin((i+.5)*tau/16)
                segment_obj = segment("Casco lançado", i*.56-2.24,(i+1)*.56-2.24,
                                      -.10,-.10,-.63,-.63,width,WOOD[i%4],.075)
                segment_obj.location.x = -13.6
        else:
            for i in range(16):
                y = (i//4)*1.05-1.75
                x = (i%4)*.88-1.45
                box("Pedra calçada", (x,y,0), (.86,1.04,.16), STONE[i%4], .055)
            wagon()

        at, focus, size = framing[kind]
        bpy.ops.object.camera_add(location=at)
        camera = bpy.context.object
        camera.rotation_euler = (Vector(focus) - camera.location).to_track_quat("-Z", "Y").to_euler()
        camera.data.type = "ORTHO"
        camera.data.ortho_scale = size
        scene = bpy.context.scene
        scene.camera = camera
        scene.render.engine = "BLENDER_WORKBENCH"
        scene.render.resolution_x, scene.render.resolution_y = 800, 440
        scene.render.resolution_percentage = 100
        scene.render.image_settings.file_format = "PNG"
        scene.render.filepath = str(output / f"{kind}.png")
        shading = scene.display.shading
        shading.light = "STUDIO"
        shading.color_type = "MATERIAL"
        shading.show_shadows = shading.show_cavity = shading.show_object_outline = True
        shading.cavity_type = "BOTH"
        shading.background_type = "WORLD"
        scene.view_settings.view_transform = "Standard"
        scene.view_settings.exposure = 0.45
        bpy.ops.render.render(write_still=True)

        page = {
            "id": kind, "paper": "parchment",
            "title_card": {"text": site["name"], "sub": site["opens"], "pos": [0.07, 0.08]},
            "panels": [{"id": kind, "image": f"cutscenes/concordia/{kind}.png",
                        "quad": [[0.07,0.25], [0.93,0.21], [0.93,0.91], [0.07,0.94]],
                        "duration": 3.9, "reveal": "slash_right", "border_px": 4.5}],
        }
        cutscene = {"id": f"canteiro-{kind}", "title": site["name"],
                    "skippable": True, "auto_advance": True, "pages": [page]}
        dest = root / "data/cutscenes" / f"canteiro-{kind}.json"
        dest.write_text(json.dumps(cutscene, ensure_ascii=False, indent=2) + "\n")


if "--pranchas" in sys.argv:
    render_panels()
else:
    for name, model in [
        ("ponte-do-estaleiro", lambda: bridge(False)),
        ("ponte-do-estaleiro_collision", lambda: bridge(True)),
        ("estaleiro", lambda: shipyard(3)),
        ("estaleiro_collision", lambda: shipyard(3, True)),
        ("estaleiro-estrutura", lambda: shipyard(1)),
        ("estaleiro-estrutura_collision", lambda: shipyard(1, True)),
        ("estaleiro-quilha", lambda: shipyard(2)),
        ("canteiro-estacas", stakes), ("canteiro-placa", sign), ("canteiro-esqueleto", frame),
        ("canteiro-esqueleto_collision", frame),
        ("balde-do-poco", bucket), ("eixo-da-carroca", axle), ("vidraca", window),
        ("campainha", bell), ("lastro", ballast),
        ("canteiro-fecho", ridge), ("portao-da-cerca", gate), ("portao-da-cerca_collision", gate), ("lona-de-edmar", awning),
    ]:
        clear()
        model()
        export(name)
