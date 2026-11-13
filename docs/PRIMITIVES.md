# Primitivas procedurais

O mundo autora props como **primitivas** dentro de `<Composition>` (ou soltas
num grupo). Os colisores Rapier são derivados da mesma geometria.

## Arquitetura (sem o Viber)

O Viber está a sair; o sistema está dividido de forma a sobreviver-lhe:

| Camada | Onde | Papel |
|---|---|---|
| **Contrato** | [`bevy_ext_scene::primitives`](../../bevy_extensions/crates/bevy_ext_scene/src/primitives) | `Shape`, `MaterialSpec`, `Preset`, parser de atributos, bounds e o marcador `PrimitivePart`. Sem render. |
| **Render** | [`bevy_ext_gradmesh`](../../bevy_extensions/crates/bevy_ext_gradmesh) | `gradmesh_shape`, `build_mesh` (gradiente por vértices), `style` (presets) e, com a feature `physics`, o colisor Rapier exato (trimesh para as formas procedurais). |
| **Jogo** | [`src/primitives.rs`](../src/primitives.rs) | `VaelPrimitivesPlugin`: consome o `PrimitivePart`, troca malha/material/colisor e regista o `GradMeshPlugin`. |
| **Editor** | `vael_jackdaw` 0.1.7, `src/primitives.rs` | Lê os `WorldNode` por reflexão e cria previews `EditorHidden` com o mesmo crate. |
| **Viber (transitório)** | `recipes`/`spawn` | Só faz o parse da DSL e deixa um fallback `StandardMaterial` + `PrimitivePart`; já não depende de `bevy_gradmesh`. Sai quando o Vael tiver o spawner do mundo. |

Quando o Vael passar a ser dono do spawn, o mesmo `PrimitivePart` (ou uma
chamada direta ao cache do plugin) entra no spawner e o render não muda.

## Tags

| Tag | Campos | Notas |
|---|---|---|
| `Box` / `Cuboid` | `half-size="x y z"`, `round="0..1"` | `round > 0` → cubo com arestas boleadas (o colisor continua o cubo). |
| `Sphere` | `radius` | |
| `Cylinder` | `radius`, `half-height` | |
| `Capsule` | `radius`, `half-height` | `half-height` é o troço cilíndrico (como `Capsule3d`). |
| `Cone` | `radius`, `half-height`, `round` | Cone/rounded cone; colisor `Collider::cone`. |
| `Pyramid` | `sides`, `radius`, `half-height`, `round` | `sides=4` → telhado de quatro águas. |
| `Torus` | `radius`, `thickness`, `start`, `span` | Ângulos em graus. |
| `Lathe` | `profile="r y r y …"`, `segments` | Perfil (raio, altura) revolvido em Y. O perfil define o eixo local: para um colisor centrado, centra o y no objeto. |
| `Sweep` | `profile="x y x y …"`, `path="x y z …"`, `closed`, `caps` | Perfil 2D varrido por um caminho 3D (corrimãos, tubos, estradas). |
| `ExtrudedPolygon` | `points="x y x y …"`, `holes="… | …"`, `depth`, `round` | Polígono XY com buracos, extrudido em Z (paredes com portas/janelas). |

`Plane` continua no `StandardMaterial` (decals/soalhos texturados): o crate não
tem plano e a aproximação por cubo fino mudaria o z-fighting.

Formatos dos pontos: lista plana separada por espaços (a convenção do resto da
DSL); `;` entre pontos também é aceite. `holes` separa contornos com `|`.

## Material

| Atributo | Efeito |
|---|---|
| `base-color` | sRGB (`#rrggbb`); cor do estilo. |
| `metallic`, `roughness` | PBR do `StandardMaterial` base. |
| `opacity` | `< 1` → translúcido (`NotShadowCaster`); `0` é invisível. |
| `emissive` / `emissive-color` | Emissão própria. |
| `texture` / `texture-url`, `normal-map` | **Não amostrados** pelo material gradiente; o URL escolhe o preset e entra no aviso de mapas ignorados. |
| `texture-tile-size` | Alimenta o ruído do preset `Auto` (mapa desconhecido). |
| `preset` / `style` | `wood`, `stone`, `cobblestone`, `metal`, `cloth`, `flat`. Ganha ao URL. |

Presets: `wood` desenha planks ao longo do X local + grão; `stone` ruído
grosso; `cobblestone` ruído mais fino e forte; `metal` ruído muito fino;
`cloth` stripes verticais; `flat` sem padrão. Cada objeto leva ainda um
gradiente vertical neutro assado em vertex colours (mais escuro em baixo).

## Colisores

`collider="auto"` numa primitiva (ou qualquer parte de uma `<Composition>` sem
física autoral) gera:

- `Box`/`Cuboid`/`RoundedCuboid` → cubo exato;
- `Sphere` → ball; `Cylinder` → cylinder; `Capsule` → capsule; `Cone` → cone;
- `Pyramid`/`Lathe`/`Sweep`/`ExtrudedPolygon`/`Torus` → **trimesh exato** da
  malha (fallback para o cubo das bounds se o bake falhar).

## Exemplos

Torre quebrada (um lathe em vez de cilindros empilhados):

```xml
<Lathe profile="3.2 0 3.2 1.2 2.7 1.4 2.7 4.4 2.55 4.6 2.55 8 2.4 8.2 2.4 12.2"
       segments="32" texture-url="/assets/textures/mountain_stone/albedo.ktx2"
       texture-tile-size="3" base-color="#9a948a" roughness="0.95" preset="stone" />
```

Pilar de ponte com base alargada:

```xml
<Lathe profile="0.95 -3.3 0.86 -3.0 0.8 -2.8 0.8 2.8 0.86 3.0 0.95 3.3"
       segments="24" base-color="#7d7d78" roughness="0.9" preset="stone" />
```

Barril:

```xml
<Lathe profile="0.42 -0.55 0.5 -0.45 0.56 -0.2 0.58 0 0.56 0.2 0.5 0.45 0.42 0.55"
       segments="24" base-color="#7a5c3a" roughness="0.82" preset="wood"
       collider="auto" />
```

Parede com porta (polígono com buraco):

```xml
<ExtrudedPolygon points="-3 -1.5 3 -1.5 3 1.5 -3 1.5"
                 holes="-1 -1.5 1 -1.5 1 0.8 -1 0.8"
                 depth="0.4" round="0.02" base-color="#93aa86" preset="stone" />
```

Corrimão contínuo (sweep):

```xml
<Sweep profile="-0.08 0 0.08 0 0.08 0.12 -0.08 0.12 -0.08 0"
       path="0 1 -10 0 1.2 0 0 1.4 10" base-color="#7a5c3a" preset="wood" />
```

## Gates

- `viber analyze world.bsn` e `viber-engine analyze world.bsn`.
- Contrato/render: `cargo test -p bevy_ext_scene primitives`,
  `cargo test -p bevy_ext_gradmesh` e
  `cargo test -p bevy_ext_gradmesh --features physics` (no `../bevy_extensions`).
- Engine (transitório): `cargo test -p viber --lib composition_tests` e
  `cargo test -p viber --test composition_spawn` (marker + fallback).
- Jogo: `CARGO_TARGET_DIR=../Viber/target cargo check` no Vael.
- O preview do Jackdaw usa o mesmo crate (`vael_jackdaw` 0.1.7); se o viewport
  não mostrar as primitivas, confirmar que o editor foi reiniciado depois de
  instalar a extensão.
- A/B de render: `VAEL_PRIMITIVES=standard` mantém o fallback do engine;
  `VAEL_PRIMITIVES_DETAIL=minimal|low|medium|high|extreme` muda o detalhe.
  Para o engine isolado (`viber run`), as formas procedurais aparecem como
  caixas de bounds — o render é do jogo.
