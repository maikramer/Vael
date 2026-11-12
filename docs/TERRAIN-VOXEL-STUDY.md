# Estudo: migrar o terreno do Vael para `voxel-framework`

Data: 2026-11-11. Âmbito: substituir o terreno do Viber (`src/terrain/`) pelo
crate [`voxel-framework`](https://crates.io/crates/voxel-framework) 0.9.1.
Método: leitura do código dos dois lados (Viber `main`, tarball do crate) e
contagem analítica; não foi corrido `cargo test` nem benchmark do crate.
O crate não compila contra o Bevy do Vael (ver §4.1), por isso não houve gate
de integração.

## Veredito

**Não migrar.** O `voxel-framework` é um engine para jogos de voxels cúbicos
(Minecraft-like): chunks fixos de 16³, voxel de 1 m, dois níveis de LOD,
streaming esférico e mesh greedy com cores por vértice. O terreno do Vael é
outra coisa: heightfield autoral de 4 km como input de um SDF, isosuperfície
suave (transvoxel), 5 LODs, splat de 12–13 camadas, água, estradas, cavernas e
arcos autorais, navmesh e colisão Rapier.

Há três bloqueios que, sozinhos, fecham a migração:

1. **Bevy**: o crate exige `bevy ^0.18`; o Viber/Vael estão em `0.20.1`. Um
   `Plugin` do Bevy 0.18 não entra numa `App` 0.20 — não é downgrade de
   dependência, é outro ECS. Só copiar o código (fork) resolve.
2. **Licença**: LGPL-3.0-or-later. Vael e Viber não têm ficheiro `LICENSE`
   (logo, por omissão, todos os direitos reservados). Distribuir um jogo
   proprietário com uma biblioteca LGPL ligada estaticamente exige materiais
   de relink e cumprir as obrigações da GPLv3; um fork obriga a publicar as
   alterações. É uma decisão jurídica, não técnica.
3. **Escala de streaming**: o crate carrega uma **esfera 3D** de chunks. A
   950 m (render-distance do Vael) são **877 256 chunks**; a 200 m, 8 144.
   O Vael mantém ~689 colunas e ~900 caixas. Não é um custo 2×, é ~1000×.

O que o crate substituiria — mesher, streaming, LOD, colisão de voxel — é
~2,7k linhas de produção do Viber, e fá-lo-ia com menos capacidade. O que o
Vael usa de facto (carve, features, splat, água, navmesh, física, autoria,
save, Lua) não existe no crate e continuaria a ser nosso. A migração não
reduz manutenção: troca 21k linhas nossas por um fork de 21k linhas alheias,
mais as que faltam.

**Recomendação**: manter o terreno; limpar o legado interno (§7); reavaliar o
crate se e quando houver 1.0, Bevy 0.20 e repositório público — e, mesmo aí,
só para um protótipo/spin-off de voxels, não para o Vael.

---

## 1. O sistema atual (Viber + Vael)

O terreno atual **já não é um heightfield 2.5D renderizado**. O `.ahgt` é
input; o runtime constrói um SDF `densidade(p) = p.y − altura(p.xz)`, combina
mods 3D e mesh a isosuperfície por transvoxel. Cavernas, arcos, pontes e
falésias são SDFs analíticos reconstruídos do XML, não volumes persistidos.

| Item | Valor | Onde |
|---|---|---|
| Módulo | 32 822 linhas totais; **19 686 de produção**; 35 ficheiros | `Viber/src/terrain/**` |
| Testes | **412** unitários no módulo + **38** de integração (`terrain_*`/`voxel_*`) | `Viber/src/terrain`, `Viber/tests` |
| Dados do Vael | `.ahgt` 29,45 MB (4096², `u16` zlib), `.paint` 33 MB | `Vael/world.data/` |
| Mundo | 4000 m × 4000 m, altura máx. 200 m, 1 m/texel, quantum vertical 3,05 mm | `Vael/world.bsn` (`WorldNode` `Terrain`) |
| Render | `render-distance=950`, 5 níveis de LOD (1/2/4/8/16 m), colunas de 64 m | `world.bsn`, `bake.json` |
| Residentes | ~689 colunas / ~900 caixas (medido em `src/terrain/CONTEXT.md:34`) | Viber |
| Colisão | trimesh Rapier por coluna, assado no swap; banda de colisão ≈108,8 m | `src/physics.rs`, `plugin.rs` |
| Material | material bindless único; 13 slots de camada, 4 planos splat RGBA8, triplanar nas paredes | `layer_material.rs`, `chunk.wesl` |
| Splat Vael | 4 × 4000² × 4 B ≈ **244 MiB** (CPU/cache), `splat-texel=1` | `splat.rs` |
| Features usadas no Vael | 5 Cave, 5 Chamber, 5 Shaft, 14 Cliff, 3 Arch, 1 RockFeatures, 22 TerrainPad, 30 Road, 39 GroundDecal, 11 Lake, 1 River; 0 Cut, 0 Plateau, 0 Bridge-voxel | `Vael/world/**` |
| Consumo | Lua: 14 chamadas a `viber.ground_below`, 2 a `viber.in_water`; Rust: `flora.rs` usa `TerrainRuntime::sample`/`reader()` | `Vael/scripts`, `Vael/src/flora.rs` |
| Navmesh | grelha XZ de 0,75 m sobre `surface_top`; caves/overhangs ficam de fora | `Viber/src/nav/backend.rs` |
| Fonte de verdade | `world.bsn` estático + `.ahgt` pós-bake; runtime não re-carva (`static_world`) | `Viber/src/terrain/runtime.rs:964` |

Características que o jogo depende e que não são triviais de reproduzir:

- Carve determinístico e com journal (`BrushGrid`), ordem canónica
  plateau → pads → lagos → rios → cuts → estradas → decals.
- SDF com `Union`/`Subtract` para cavernas, arcos, pontes e paredes de
  falésia; a ordem dos mods é significativa.
- Mesher transvoxel com transições 2:1 e colisão trimesh idêntica ao render.
- Splat global + pintura manual + material bindless com 13 camadas.
- Água como corpos (lago/rio) com shader de profundidade, espuma e cascatas.
- Autoria: `viber terrain brush|paint|sample`, editor, undo e hot-reload.
- Integração: Lua, save (`DeltaGrid`), navmesh, física, grass/foliage,
  minimapa, cinemáticas.

## 2. O que é o `voxel-framework` 0.9.1

| Item | Valor |
|---|---|
| Release | 0.9.1, publicado em 2026-11-12T00:58Z; primeira versão 0.4.0 em 2026-09-04 (8 releases em ~2 meses) |
| Licença | **LGPL-3.0-or-later** |
| Bevy | `^0.18` (o Vael/Viber usam 0.20.x; o Vael fixa `=0.20.1`) |
| Rust | MSRV 1.89, edition 2024 |
| Tamanho | 21,6k linhas de código (crates.io); 33,6k linhas em `src/` com testes e UI |
| Dependências | bevy, serde, serde_json, toml, tempfile, tracing, windows-sys |
| Repositório | o URL no manifesto responde **404** (verificado em 2026-11-11); código só no tarball do crates.io |
| Modelo | `Chunk<V>` 16×16×16, voxel de 1 m, 1 byte + 1 byte de estado por voxel |
| Streaming | esfera 3D de raio `load_distance` (default 150 m; UI 4–32 chunks = 64–512 m) |
| LOD | **2 níveis**: passo 1 abaixo de 200 m, passo 2 a partir daí; >2 foi descartado |
| Mesh | greedy meshing neighbor-aware, AO por vértice, textura por camada |
| Render | 1 entidade `Mesh3d` por chunk, material partilhado; texture array com sol fixo no shader, ou `StandardMaterial` com cores por vértice |
| Colisão | push-out esfera/cápsula contra AABBs de voxel; **não usa Rapier** |
| Persistência | `chunk_X_Y_Z.bin`, 8 KB por chunk, save no unload/`ChunkModified` |
| Player | plugin próprio (6DOF/grounded) com input e movimento próprios |
| Fora de âmbito | rede/multiplayer, editor, pipeline de assets, água como sistema, navmesh, autoria de terreno |
| Geradores | trait `ChunkGenerator::generate(coord, &mut Chunk)`; FBM embutido; custom possível |

O crate está bem organizado (catálogo de plugins por categoria, geração
assíncrona, mesher com vizinhos, persistência por chunk). É uma boa base para
um sandbox cúbico ou um protótipo de mineração/construção. Não é um engine de
terreno autoral.

## 3. Comparação

| Dimensão | Vael/Viber hoje | `voxel-framework` 0.9.1 |
|---|---|---|
| Bevy | 0.20.1 | 0.18 (incompatível) |
| Licença | proprietária (sem `LICENSE`) | LGPL-3.0-or-later |
| Representação | heightfield → SDF → isosuperfície | volume cúbico denso por chunk |
| Unidade vertical | contínua (SDF) | voxel de 1 m |
| Chunk | coluna 64 m + caixas 32³ | 16³ fixo |
| LOD | 5 (1/2/4/8/16 m), transições 2:1 | 2 (1/2 m), sem transvoxel |
| Streaming | colunas por distância XZ, orçamento por frame | esfera 3D, sem orçamento |
| Residentes a 950 m | ~689 colunas / ~900 caixas | 877 256 chunks (teórico) |
| Memória de dados | 29 MB + 33 MB + splat 244 MiB | 8 KB/chunk; 7,0 GB se 877k residentes |
| Visual | superfície suave + splat 13 camadas + paredes triplanar | faces planas, AO, textura por camada ou cor |
| Água | corpos com shader, profundidade, espuma, cascatas | voxel de água do palette |
| Física | Rapier, trimesh por coluna | push-out próprio contra voxels |
| Navmesh | rerecast/landmass | não tem |
| Autoria | CLI, editor, pincéis, XML features, undo | não tem |
| Save | save do jogo (`DeltaGrid`, patches) | ficheiros por chunk |
| Scripting | Luau (`viber.ground_below`, etc.) | não tem |
| Testes | 412 unit + 38 integração | testes do crate, sem integração connosco |
| Estabilidade | contrato interno | 0.x, API e formato de save instáveis |

## 4. Bloqueios, por ordem de decisão

### 4.1 Bevy 0.18 vs 0.20 (bloqueio duro)

`voxel-framework` compila contra `bevy ^0.18`; o Viber exige `bevy 0.20.1`
(caret; `Viber/Cargo.toml:42`) e o Vael fixa `=0.20.1`. O trait `Plugin`, `App`, `Component`, `Resource`,
`Mesh` e o render são tipos de versões diferentes. Não se podem adicionar os
plugins do crate a uma app 0.20 — nem sequer partilhar uma entidade. Ter as
duas versões no mesmo binário é possível para Cargo, mas inútil para nós.

Saídas: (a) baixar Viber+Vael para Bevy 0.18 (inviável: a engine usa APIs de
0.20 e o editor também); (b) copiar o código e portar (fork); (c) esperar.
O crate não publica suporte a 0.20.

### 4.2 Licença LGPL em jogo proprietário

Vael e Viber não têm `LICENSE` (direitos reservados por omissão). A LGPL-3.0
permite uso comercial, mas com obrigações: relinkabilidade (objetos ou
biblioteca dinâmica), disponibilização do código da biblioteca e, se houver
alterações, publicação dessas alterações sob a mesma licença; além das
cláusulas da GPLv3 (patentes, anti-tivoização). Num binário Rust ligado
estaticamente, isso costuma significar distribuir os objetos do jogo ou
mudar a crate para `dylib`. Para um jogo na Steam, é um processo a validar
com jurista antes de qualquer linha de integração. Não é um "não" absoluto,
mas é um custo recorrente que não existe hoje.

### 4.3 Streaming esférico e densidade

`streaming_system` percorre `[-r, r]³` e carrega todo o chunk cujo **centro
3D** está a menos de `load_distance` (`world/systems.rs:109-147`). Numa
heightfield só interessam colunas de superfície; a esfera pede também o ar
acima e a rocha abaixo.

| `load_distance` | chunks (esfera) | memória só de dados (8 KB/chunk) |
|---:|---:|---:|
| 150 m (default) | 3 448 | 28 MB |
| 200 m (limiar de LOD) | 8 144 | 67 MB |
| 512 m (máximo da UI) | 137 376 | 1,13 GB |
| **950 m (Vael)** | **877 256** | **7,19 GB** |

O Vael renderiza a 950 m com ~689 colunas. Baixar para 150–200 m mudaria o
jogo (vistas abertas, landmarks, cinemáticas). Aumentar a distância no crate
não é uma questão de memória: são 877k entidades, tarefas de geração e
meshes. A geração ainda teria de preencher cada chunk abaixo da superfície
(4096 voxels) — o volume total do mundo é 3,2 G voxels.

### 4.4 LOD e linguagem visual

O crate usa dois níveis e descarta passos >2 porque "produzem quads planos
visualmente chocantes" (`mesh/lod.rs`). A 950 m o terreno continuaria a 2 m,
com degraus de 1 m. O Vael usa 5 níveis e chega a 16 m no horizonte.

O mesh é greedy: faces planas por tipo de voxel, normais por face, AO por
vértice. O terreno atual é uma isosuperfície suave com normais de gradiente
e transições de LOD sem costura. O splat de 13 camadas com triplanar nas
paredes não tem equivalente: o crate tem uma textura por camada e um
`StandardMaterial` com cores por vértice. E o caminho com texture array usa
**sol fixo no shader** (`render/chunk.wgsl`), fora do ciclo dia/noite, IBL,
sombras e post-FX do Vael. Adotar o crate seria mudar a direção de arte e o
sistema de iluminação, não só o mesher.

### 4.5 Sistemas que continuariam nossos

- Água: lagos/rios com máscara, profundidade, espuma, cascatas e som.
- Estradas/trilhos/decals de chão (30 estradas, 39 decals no Vael).
- Grass/foliage e colocação de props.
- Navmesh e IA.
- Física Rapier e colliders de props.
- Carve autoral: pads, lagos, rios, cuts, falésias, arcos, pontes, cavernas.
- Splat e pintura manual.
- CLI/editor/undo, Lua, save, minimapa, cinemáticas.

O crate não fornece nenhum destes. Mesmo que o streaming e o mesher fossem
adotados, ~80–90% do módulo de terreno continuaria a existir.

### 4.6 Física e colisão

O terreno do Vael colide por trimesh Rapier por coluna, com o mesmo triângulo
do render e collider assado no swap. O crate resolve cápsulas contra AABBs de
voxel e deixa a resposta de velocidade ao chamador. Adotá-lo implica manter
dois modelos de colisão (voxel para o terreno, Rapier para props/personagens)
ou escrever um bake de trimesh a partir dos chunks — exatamente o que já
temos, mas contra outro formato.

### 4.7 Autoria e dados

O mundo é autoral e estático. O crate não tem importador de heightmap, nem
editor, nem features; o gerador embutido é FBM. Seria preciso um
`ChunkGenerator` nosso que amostrasse o `.ahgt` e avaliasse os SDFs — possível,
mas nesse caso o valor do crate reduz-se ao streaming/mesher. A persistência
dele (8 KB/chunk, 812 500 chunks para o mundo inteiro) também não substitui o
`.ahgt` + `.paint` + patches de save: é um formato mais caro e sem o pipeline
de autoria.

### 4.8 Maturidade e cadeia de fornecimento

- 0.9.1 publicado há horas; 8 releases em ~2 meses; API explicitamente
  instável entre minors e formato de persistência sem garantia 1.0.
- O repositório indicado no manifesto responde 404; não há issue tracker
  público nem histórico acessível — o código é o tarball.
- Uma dependência destas num sistema de 21k linhas e num save de jogador é
  risco de fornecedor maior do que a manutenção interna que se quer evitar.

## 5. O que uma migração exigiria

### A. Adotar como dependência (sem fork)

Bloqueado por 4.1. Nem chega a ser uma opção enquanto o crate não suportar
Bevy 0.20.

### B. Fork + porte + extensões

Portar Bevy 0.18 → 0.20; trocar o streaming esférico por colunar/cilíndrico
com orçamento; acrescentar LODs 3–5 com stitching; escrever o material splat
e o caminho de parede; reimplementar água, navmesh, física Rapier, autoria,
save, Lua e editor contra o novo modelo; manter o fork sob LGPL e publicar as
alterações. Isto não é uma migração: é reescrever o terreno com outro nome,
com uma licença pior e um upstream que não acompanha. O esforço é maior do que
o módulo atual e o resultado perde qualidade visual e ferramentas.

### C. Não migrar

Manter o terreno e reduzir a manutenção por dentro (§7). É a única opção que
reduz risco e custo no horizonte do Vael.

## 6. Onde o crate seria útil

- Protótipo/spin-off de voxels cúbicos (sandbox, mineração/construção, jam),
  num binário Bevy 0.18 separado.
- Referência de desenho: catálogo de plugins por categoria, geração assíncrona
  com `AsyncComputeTaskPool`, mesher neighbor-aware, byte de estado por voxel.
  São ideias boas; não justificam substituir o runtime.
- Se algum dia houver um modo "voxel cúbico" no Vael, ele seria um segundo
  renderer/modo de mundo, não o terreno principal.

## 7. Alternativas para "não manter tudo"

Sem adotar o crate, há gavetas óbvias:

1. **Apagar legado morto.** `src/terrain/sampler.rs` tinha 988 linhas e só era
   referido por `mod.rs`, `mesh.rs` (doc) e docs; o runtime usa `BrushGrid`.
   Remover `HeightSampler` e as reexportações respetivas reduz
   ~1k linhas e elimina uma segunda fonte de verdade de amostragem; o
   `ResolvedPad` (usado pelo carve e pelo scatter) mantém-se.
   **Executado em 2026-11-11**: `sampler.rs` removido; o único item vivo,
   `ResolvedPad`, mudou-se para `src/terrain/pads.rs` (com os imports de
   `features`, `runtime`, `probes`, `scatter`, `reflect` e `nav_navmesh`
   atualizados).
2. **Corrigir a documentação obsoleta.** `terrain/mod.rs` ainda descreve
   "sampler único" e skirts; `CONTEXT.md` diz "100% voluminoso" e
   `seal_faces`; `mesh.rs` refere `build_chunk_mesh`; `runtime.rs` fala de
   material por chunk. Isto faz cada leitura custar mais do que devia.
3. **Fronteira explícita.** Avaliar extrair `viber::terrain` para um crate do
   workspace com uma API pública estreita (`TerrainRuntime`, `TerrainReader`,
   specs, `VoxelField`). Ganha-se compile/test isolados e torna-se a fronteira
   visível; não é para fazer às cegas porque `terrain` depende de `physics`,
   `recipes` e `nav`.
4. **Inventário de features.** `Cut`, `Plateau` e `bank="gorge|overhang"` não
   são usados no Vael (0 ocorrências); continuam a ser usados nos mundos do
   Viber. Não apagar da engine; documentar como "não usado pelo Vael" para
   evitar regressões de QA.
5. **Guardas de performance.** Manter os testes de saúde do mesh e o
   `chunk_build_bench`; o custo documentado é 5,8 ms por caixa LOD0 e 3,5 ms
   de orçamento por frame (`plugin.rs:255`). Isto é o que impede a regressão
   silenciosa que motivaria uma migração.
6. **Observar o crate.** Reavaliar quando houver 1.0, Bevy 0.20, repositório
   público e uma resposta clara sobre licença. Até lá, não é candidato.

## 8. Decisão seguinte

Opções, por ordem de custo/benefício:

1. **Arquivar a migração** e executar o plano de limpeza (itens 1–3 de §7).
2. **Estudo de fronteira**: medir o acoplamento real de `terrain` com
   `physics`/`recipes`/`nav` e decidir se a extração para crate compensa.
3. **Protótipo separado** com `voxel-framework` (Bevy 0.18) para testar a
   linguagem visual de voxels, sem tocar no Vael.
4. **Fork** só se o objetivo passar a ser um jogo de voxels cúbicos e a
   licença for aceite; nesse caso o caminho é um projeto novo, não o terreno
   do Vael.

## Anexo: reproduzir os números

```bash
# Versão, licença, Bevy e datas
curl -s https://crates.io/api/v1/crates/voxel-framework/versions \
  | python3 -c 'import json,sys; [print(v["num"], v["created_at"], v["license"]) for v in json.load(sys.stdin)["versions"]]'

# Constantes do crate (16³, voxel 1 m, LOD único de 200 m)
grep -n 'CHUNK_SIZE\|VOXEL_SIZE\|LOD0_MAX_DISTANCE' \
  /tmp/vf-study/src/voxel-framework-0.9.1/src/{chunk/mod.rs,mesh/lod.rs}

# Esfera de chunks a várias distâncias
python3 - <<'EOF'
import math
for d in (150,200,512,950):
    r=math.ceil(d/16); n=0
    for dx in range(-r,r+1):
        for dy in range(-r,r+1):
            for dz in range(-r,r+1):
                if math.dist(((dx+.5)*16,(dy+.5)*16,(dz+.5)*16),(0,0,0))<=d: n+=1
    print(d,n)
EOF

# Estado do Vael
rg -n 'tag: "Terrain"' Vael/world.bsn
du -h Vael/world.data/terrain.ahgt Vael/world.data/terrain.ahgt.paint
```
