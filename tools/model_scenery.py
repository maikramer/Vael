"""Exporta os adereços talhados dos cenários autorados para o pool do Vael."""
from pathlib import Path
from math import cos, pi, sin, tau
import bpy

output = Path(__file__).resolve().parents[1] / "assets/public/assets/meshes/cenario"
output.mkdir(exist_ok=True)

def color(name, rgb):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    rgba = (*rgb, 1)
    m.diffuse_color = rgba
    shader = m.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Base Color'].default_value = rgba
    shader.inputs['Roughness'].default_value = .9
    return m

oak = color('Carvalho fumado', (.19, .105, .052))
wood = color('Madeira cortada', (.31, .192, .105))
iron = color('Ferro oxidado', (.08, .075, .066))
stone = color('Cantaria', (.21, .193, .145))
glass = color('Vidro âmbar', (.61, .31, .065))
cactus = color('Verde de cacto', (.115, .241, .098))
cactus_light = color('Crescimento novo', (.157, .291, .12))

def finish(material):
    bpy.context.object.data.materials.append(material)

def box(center, size, material, bevel=.025, rotation=(0,0,0)):
    bpy.ops.mesh.primitive_cube_add(size=1, location=center, rotation=rotation)
    obj = bpy.context.object
    obj.dimensions = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    edge = obj.modifiers.new('Arestas talhadas', 'BEVEL')
    edge.width = bevel
    edge.segments = 1
    finish(material)

def stem(center, r1, r2, height, material, turn=(0, 0, 0), sides=7):
    bpy.ops.mesh.primitive_cone_add(vertices=sides, radius1=r1, radius2=r2,
                                   depth=height, location=center, rotation=turn)
    finish(material)

def export(name):
    bpy.ops.export_scene.gltf(filepath=str(output / (name + '_lod0.glb')),
                              export_format='GLB', export_apply=True)
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)

bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
for x in [-.64, .64]:
    box((x, 0, .18), (.18, .45, .36), stone, .05)
for y in [-.13, .13]:
    box((0,y,.39), (1.78,.23,.14), wood)
box((0,0,.27), (1.57,.13,.11), oak)
export('banco-ribeirinho')

for y,z,length,r in [(-.28,.16,2.5,.17),(.06,.17,2.36,.18),(.4,.15,2.44,.16),
                     (-.11,.42,2.5,.16),(.23,.43,2.63,.17),(.05,.66,2.25,.15)]:
    stem((0,y,z),r,r*.92,length,oak,(0,pi/2,0))
    stem((length/2+.007,y,z),r*.87,r*.87,.018,wood,(0,pi/2,0))
    stem((-length/2-.007,y,z),r*.91,r*.91,.018,wood,(0,pi/2,0))
export('pilha-de-madeira')

box((0,0,.09),(.50,.50,.19),stone,.06)
box((0,0,.9),(.22,.22,1.68),oak)
box((0,0,1.79),(.44,.44,.09),iron,.018)
box((0,0,1.97),(.29,.29,.28),glass,.016)
for x in [-.16,.16]:
    for y in [-.16,.16]:
        box((x,y,1.97),(.043,.043,.36),iron,.008)
stem((0,0,2.19),.30,.09,.21,iron,sides=4)
export('lampiao-de-margem')

stem((0,0,.74),.195,.16,1.46,cactus)
stem((0,0,1.49),.16,.085,.18,cactus_light)
stem((-.23,0,.78),.14,.125,.47,cactus,(0,-pi/2.9,0))
stem((-.39,0,1.02),.13,.10,.48,cactus_light)
stem((.19,.01,.65),.12,.10,.37,cactus,(0,pi/2.8,0))
stem((.32,.01,.86),.102,.066,.36,cactus_light)
export('cacto-canelado')

slate = color('Telha de ardósia', (.096, .126, .134))
plaster = color('Cal amarelada', (.49, .39, .253))
shade = color('Sombra interior', (.045, .044, .035))
moss = color('Musgo esverdeado', (.18, .23, .096))
canvas = color('Lona ferrugem', (.48, .245, .132))

# O abrigo é de madeira e cal, com alpendre voltado para o caminho.
box((0,0,.17),(5.45,4.5,.52),stone,.07)
box((0,0,1.42),(5.15,4.2,2.40),plaster,.06)
for x in [-2.56,0,2.56]:
    box((x,-2.14,1.44),(.18,.19,2.52),oak)
for side in [-1,1]:
    box((0,side*1.23,3.03),(5.77,2.81,.19),slate,.04,(side*-0.35,0,0))
box((0,0,3.49),(5.77,.16,.21),oak)
box((-.9,-2.16,.93),(1.03,.15,1.88),oak)
box((1.08,-2.15,1.54),(.93,.16,.73),wood)
box((1.08,-2.25,1.54),(.68,.07,.49),shade,.01)
box((-.90,-2.68,2.18),(1.76,1.14,.14),slate,.04,(.18,0,0))
for x in [-1.68,-.12]:
    box((x,-3.02,1.02),(.14,.14,2.03),oak)
export('abrigo-do-vale')

for x in [-1.49,1.49]:
    for y in [-.65,.65]:
        box((x,y,1.11),(.15,.15,2.22),oak)
box((0,0,.92),(3.18,1.63,.16),wood)
box((0,0,2.22),(3.62,1.95,.12),canvas,.03,(.12,0,0))
box((0,-.93,2.08),(3.62,.08,.23),canvas)
export('banca-ribeirinha')

box((0,0,.44),(.94,.77,.86),oak)
for offset in [-.24,0,.24]:
    box((0,offset,.88),(.96,.23,.095),wood)
for x in [-.35,.35]:
    for y in [-.41,.41]:
        box((x,y,.46),(.14,.079,.91),wood,.012)
export('caixote-de-carga')

def facet(center, scale, mat):
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1, radius=1, location=center)
    obj = bpy.context.object
    obj.scale = scale
    finish(mat)

for i in range(10):
    angle = i * tau / 10
    facet((.65*cos(angle),.62*sin(angle),.11),(.25,.21,.17),stone)
for i in range(3):
    stem((0,0,.09+i*.072),.083,.072,1.0,oak,(0,pi/2,i*pi/3))
export('fogueira-de-cantaria')

facet((0,0,.44),(1.26,.84,.71),stone)
facet((.50,.12,.27),(.80,.70,.46),shade)
export('pedra-do-vale')

facet((0,0,.33),(1.14,.88,.57),stone)
facet((-.09,.09,.64),(.82,.70,.29),moss)
export('pedra-musgosa')
