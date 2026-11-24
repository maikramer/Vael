# Texturas — layout, fontes e licenças

Biblioteca de materiais tileable do pool partilhado, servida por URL
(`/assets/textures/…`) à web e à engine nativa.

## Layout — uma pasta por material

```
textures/<material>/
  albedo.webp      # difuso sRGB (WebP q92)
  normal.webp      # normal map OpenGL (WebP q95)
  ao.webp, height.webp, edge.webp, metallic.webp,
  roughness.webp, smoothness.webp   # consoante a origem (WebP q95)
  gen.json         # metadados de geração (só materiais AI)
```

Tudo a **1024×1024**. Normais em convenção **OpenGL** (a que Bevy e three.js
esperam); variantes DirectX (ambientCG `NOR_DX`) são descartadas no import.
WebP porque é o único formato comprimido que ambos os runtimes decodificam
nativamente (browsers + Bevy com a feature `webp`).

## Materiais

### AI — vramd texture2d + Materialize (regeneráveis; metadados de geração
em `gen.json`)

`asphalt`¹, `cobblestone_road`, `desert_sand`, `forest_floor`,
`mountain_stone`, `roof_tiles`, `snow_peak`, `swamp_mud`, `vale_grass`,
`wall_plaster`, `wood_planks`

¹ só albedo, sem PBR nem `gen.json`.

### Madeira e corda das cercas — texture2d + Materialize (2026-11-23)

`wood_oak_hewn`, `wood_pine_dark`, `wood_sun_bleached`, `wood_swamp_tar`,
`wood_shelter_pine`, `wood_weathered_grey`, `wood_lime_washed`,
`wood_hazel_rod`, `rope_hemp`

Regeneráveis com `tools/gen_fence_textures.py` a partir de
`assets/manifests/wood-textures.yaml`. São superfícies **contínuas** — sem
juntas de tábuas nem nós, porque a tábua é geometria do `bevy_ext_fence` e um
nó desenhado repetir-se-ia em cada peça — com o veio ao longo do eixo V.
Além de `albedo`, `normal`, `ao`, `roughness` e `height`, cada pasta traz
`orm` (R = oclusão, G = roughness, B = metálico a zero), o mapa que o
`StandardMaterial` do Bevy 0.20 lê; tudo em `.webp` e `.ktx2` (UASTC, cadeia
de mipmaps). A cor média é a da paleta do `docs/ART.md` §4 (`--match-color`,
graduada pela ferramenta) e a roughness está remapeada para 0,78–0,95. O
`gen.json` traz o prompt, a seed, a seed pedida, os portões de qualidade e a
graduação. Consumidos pelo `bevy_ext_fence` (`world/settings/fences.bsn`,
`docs/FENCES.md`).

### Fotográficas — [ambientCG](https://ambientcg.com), **CC0 1.0** (domínio
público; usar, modificar e redistribuir livremente, sem atribuição)

| Pasta        | Asset ambientCG | Página                                  |
| ------------ | --------------- | --------------------------------------- |
| `dirt`       | Dirt01          | https://ambientcg.com/view?id=Dirt01    |
| `grass`      | Grass01         | https://ambientcg.com/view?id=Grass01   |
| `gravel`     | Gravel01        | https://ambientcg.com/view?id=Gravel01  |
| `sand`       | Sand01          | https://ambientcg.com/view?id=Sand01    |
| `dirt_road`  | Ground107       | https://ambientcg.com/view?id=Ground107 |
| `dirt_trail` | Ground086       | https://ambientcg.com/view?id=Ground086 |
| `pebbles`    | Ground021       | https://ambientcg.com/view?id=Ground021 |

Import (2026-09-02): original 2048² (Ground*_1K-JPG 1024²) reamostrado para
1024² (LANCZOS; normais renormalizadas) e re-codificado em WebP — os bytes
já não são os do zip original. Materiais AI: ver `gen.json` (prompt, seed,
data) de cada pasta.

`pebbles` (2026-09-04): leito de rio/lago — Ground021 1K importado já a 1024²
(albedo q92, normal GL/roughness/ao q95) + KTX2 UASTC via
`Viber/scripts/ktx2_compress_pool.py --loose`. Consumido pelo 13.º slot do
splatter de terreno (`pebbles` em `Viber/src/terrain/splat.rs`).

As camadas `dirt_road` e `dirt_trail` usam também os mapas NormalGL, AmbientOcclusion, Displacement e Roughness dos mesmos pacotes Ground107 e Ground086 em 1K. Os KTX2 de dados são lineares e incluem a cadeia de mipmaps.
