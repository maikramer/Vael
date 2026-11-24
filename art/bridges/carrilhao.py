"""Blender source for the three moon chimes on Vessa's footbridge (units in metres)."""

from math import cos, pi, sin
from pathlib import Path

import bmesh
import bpy
from mathutils import Vector

bpy.ops.wm.read_factory_settings(use_empty=True)
materials = []
vertices, faces, slots = [], [], []


def finish(name, rgb, metal, rough, glow=0):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    shader = mat.node_tree.nodes.get("Principled BSDF")
    for key, value in {"Base Color": (*rgb, 1), "Metallic": metal, "Roughness": rough,
                       "Emission Color": (*rgb, 1), "Emission Strength": glow}.items():
        shader.inputs[key].default_value = value
    materials.append(mat)
    return len(materials) - 1


wood = finish("Espinheiro", (.115, .049, .025), 0, .79)
copper = finish("Cobre antigo", (.28, .103, .037), .63, .42)
gold = finish("Borda polida", (.79, .43, .12), .61, .28)
patina = finish("Azinhavre", (.035, .27, .23), .42, .56)
thread = finish("Linho encerado", (.47, .32, .14), 0, .87)
glass = finish("Vidro de rio", (.055, .70, .63), .18, .2, .65)


def skin(rings, material):
    start, count = len(vertices), len(rings[0])
    for ring in rings:
        vertices.extend(ring)
    for j in range(len(rings) - 1):
        a = start + j * count
        for k in range(count):
            k1 = (k + 1) % count
            faces.append((a + k, a + k1, a + k1 + count, a + k + count))
            slots.append(material)
    faces.extend([tuple(start + i for i in reversed(range(count))),
                  tuple(len(vertices) - count + i for i in range(count))])
    slots.extend([material, material])


def rod(a, b, r, end, material, n=9):
    a, b = Vector(a), Vector(b)
    axis = (b - a).normalized()
    u = axis.cross(Vector((0, 1, 0)) if abs(axis.y) < .9 else Vector((1, 0, 0))).normalized()
    v = axis.cross(u)
    skin([[p + radius * (u*cos(i*2*pi/n) + v*sin(i*2*pi/n)) for i in range(n)]
          for p, radius in [(a, r), (b, end)]], material)


def station(s, side, size, tubes):
    # The mounting follows the cambered deck and sits outside its rope guard.
    deck = .18 * (s+16)/32 + 2.75*sin(pi*(s+16)/32)
    x = side*1.25 + .35*sin(2*pi*(s/32+.5))
    def point(u, h, depth=0):
        return Vector((x + size*(side*.51*u + .86*depth), s + size*(.86*u-side*.51*depth), deck+size*h))
    def branch(a, b, r, end, m):
        rod(point(*a), point(*b), size*r, size*end, m)
    branch((0,-.27),(-.08,1.14),.126,.093,wood)
    branch((-.08,1.14),(-.4,1.92),.10,.062,wood)
    branch((-.4,1.92),(-.82,2.40),.072,.024,wood)
    branch((-.08,.97),(.04,1.45),.081,.060,wood)
    branch((.04,1.45),(.53,1.91),.060,.026,wood)
    for h in [.16,.26,.36]:
        branch((-.03*h,h),(-.03*h,h+.065),.132,.132,thread)
    # Crescent tilted across the grain of the bridge. A fine gilt inner rim
    # catches sunlight while the patinated broad face reads at a distance.
    rings = []
    for i in range(37):
        a = pi*(.82 + 1.36*i/36)
        w = .014 + .14*sin(pi*i/36)**.8
        u, h = .70*cos(a), 2.29+.70*sin(a)
        rings.append([point(u+dr*cos(a), h+dr*sin(a), d) for dr,d in
                      [(-w,-.058),(w,-.058),(w,.058),(-w,.058)]])
    skin(rings, patina)
    for a, b in zip(rings, rings[1:]):
        rod(a[0],b[0],.018*size,.018*size,gold,6)
    branch((-.69,2.32),(.68,2.32),.027,.027,copper)
    for i in range(tubes):
        u = -.44 + .88*i/(tubes-1)
        h = 2.12 + .045*cos(i*1.8)
        foot = 1.62 + .12*i - .17*sin(i*1.6)
        branch((u,2.32),(u,h),.014,.014,thread)
        branch((u,h),(u,foot),.046,.062,copper)
        branch((u,foot),(u,foot+.067),.069,.067,gold)
    # Faceted spindle over the bells, in the open part of the crescent.
    branch((0,2.32),(0,2.64),.012,.012,thread)
    branch((0,2.58),(0,2.76),.016,.12,glass)
    branch((0,2.76),(0,2.95),.12,.008,glass)


station(9, 1, 1.02, 3)
station(0, -1, 1.25, 5)
station(-9, 1, .90, 4)
mesh = bpy.data.meshes.new("Lua, foles e badalos")
mesh.from_pydata(vertices, [], faces)
for material in materials:
    mesh.materials.append(material)
for poly, slot in zip(mesh.polygons, slots):
    poly.material_index = slot
    poly.use_smooth = slot != glass
# The mounts on opposite rails are mirrored. Orient each closed shell outwards.
topology = bmesh.new()
topology.from_mesh(mesh)
bmesh.ops.recalc_face_normals(topology, faces=list(topology.faces))
topology.to_mesh(mesh)
topology.free()
obj = bpy.data.objects.new("Carrilhão de Vessa", mesh)
bpy.context.collection.objects.link(obj)
target = Path(__file__).resolve().parents[2] / "assets/public/assets/meshes/infra/carrilhao_de_vessa_lod0.glb"
bpy.ops.export_scene.gltf(filepath=str(target), export_format="GLB")
