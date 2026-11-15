# Física: Rapier → Avian (+ tnua)

Migração de 2026-11-14. O motor de física do jogo passou de `bevy_rapier3d 0.37`
(Rapier 0.36 / parry 0.31) para **`avian3d 0.8`**, com o controller do herói no
**`bevy-tnua 0.33`** (`bevy-tnua-avian3d 0.13`). O `bevy_ext_gradmesh` e a
relva GPU (`bevy_grass`, feature `avian`) acompanharam.

O que **não** mudou: os `.bsn` do mundo, o contrato `bevy_ext_scene`
(`ColliderSpec`/`BodySpec`), o parser DSL (`collider=`/`rigidbody=`) e os
valores autorados (tamanho completo, `half-height`). A conversão para a API do
motor vive toda em `viber::physics`.

## Duas armadilhas de unidades (concentradas na conversão)

| DSL (autoral) | Rapier (antes) | Avian (agora) |
|---|---|---|
| `box: size 2 4 6` (tamanho completo) | `cuboid(half)` → 1,2,3 | `cuboid(x,y,z)` **completo** → 2,4,6 |
| `cylinder: radius r; half-height hh` | `cylinder(hh, r)` | `cylinder(r, hh*2)` |
| `capsule: radius r; half-height hh` | `capsule_y(hh, r)` | `capsule(r, hh*2)` |
| trimesh | `trimesh_with_flags(...)` | `try_trimesh_with_config(...)` (o `trimesh` do Avian **panica**) |

Os testes de `physics.rs` (`shape_collider_tests`) verificam as meias-extensões
resultantes — um factor 2 silencioso é o erro clássico desta migração.

## O herói (tnua)

O Avian não traz character controller com chão/autostep. O `bevy-tnua` é um
controller **flutuante** de corpo **dinâmico**: aplica forças para manter o
corpo a `float_height` do chão, com `cling_distance` a segurar descidas e
`max_slope` a recusar rampas.

- A origem do herói continua a ser a **pegada** (pés). O colisor é o composto
  com a cápsula a `HERO_CAPSULE_CENTER`; `float_height = 0.04` (4 cm de folga,
  sem tocar no chão — com contacto, o solver e a spring lutam).
- `TnuaGravity(NEG_Y * 60)` mantém a gravidade do jogo; o `GravityScale(0)`
  vem por required component e desliga a do Avian nesse corpo.
- `speed: 1.0` no config faz `desired_motion` ser a **velocidade em m/s** — o
  `player_movement` do Vael continua a calcular `vel_x/vel_z` como antes.
- O config é um **asset por altura de salto** (`Player.jump_height`, 2.3 m no
  Vael): `attach_hero_controller` cria-o e insere o `TnuaConfig` + o
  `TnuaController<S>` (a receita `<PlayerGLTF>` só traz o corpo — o scheme é
  da app, ver "Integração profunda" abaixo).
- Na **natação** o controller é desligado (`TnuaToggle::Disabled`) e o
  `LinearVelocity` é nosso (empuxo, corrente, impulso para a margem); o salto
  para a margem continua manual.
- O salto é a ação `VaelHeroScheme::Jump` (altura no asset); o **buffer e o
  coyote são do crate** (`input_buffer_time`/`coyote_time` no config; o walk
  basis conta o coyote num timer interno). O jogo só abre a janela de feed
  (`JUMP_BUFFER`) e alimenta; o SFX dispara no arranque real da ação
  (`action_flow_status` em `hero_jump_sfx`).

### Integração profunda (2026-11-15)

O scheme vive no JOGO (`VaelHeroScheme` em `src/player.rs`); o
`HeroPhysicsPlugin<S: TnuaScheme>` da engine ficou genérico e o
`HeroSpatialExt` do adapter Avian deu lugar a um `VaelSpatialExt` local que
exclui o herói da leitura de colliders (o `player_movement` escreve a
`Position` dele no mesmo sistema — B0001). Ações do scheme:

| Ação | Gatilho | Notas |
|---|---|---|
| `Jump` | Espaço (`just_pressed`; janela de feed `JUMP_BUFFER`) | coyote/buffer do crate; SFX no arranque real |
| `Dash` | esquiva (`viber.vael.dodge`) e lunge (`attack_motion`) | `displacement` exato do perfil (8.4 m/s); o hop lateral entra no componente Y — efeito físico que se tinha perdido na migração |
| `Knockback` | componente `Knockback` (Lua `viber.push_player`) → `action_interrupt` | incancelável; o settle fresco limpa `current_action` à mão |
| `Crouch` | LeftCtrl/dpad_down (held: `action_start`/`action_end`) | exige `headroom` no walk config (sensor de teto automático); `hero_crouch_collider` troca a cápsula (1.8 → 1.15 m) enquanto o discriminante durar; passo ×0.45 |
| `WallSlide`/`WallJump` | radar (`TnuaObstacleRadar` 0.5×1.2) + input para a parede | gate `VAEL_WALL_MOVES=0` desliga; `same_trigger(Jump)` partilha o Espaço; lança na normal (`horizontal_distance: 2.0`) |
| `Climb` | blip de um prop com `climbable="1"` nos props (`Climbable`) | W/S sobem/descem (3 m/s); sair do raio do radar solta; terreno NUNCA é climbable |

Animação: o `hero_anim_state` (Vael) lê a memória das ações com o
`TnuaAnimatingState` e escreve `AnimStateOverride` (`bevy_ext_core::animation`)
— o `drive_player_animation` da engine honra-o, mas o blending grounded
(idle/walk/run/água) continua dele. Clips novos (crouch, dodge-roll, parede)
entram no GLB do herói e mapeiam em `map_anim_state`.

Sem corpo (apps de teste headless) o caminho manual de sempre continua: burst
de esquiva, impulso de salto e knockback com decaimento próprios.

### O autostep é nosso (`probe_steps`/`apply_steps`)

O tnua não sobe degraus. O assist (em `viber::player`) dispara com a direção
**desejada** (encostado ao degrau a velocidade real é ~0), exige um topo
**horizontal** (`normal.y > 0.95`; sem isto "escadeia" rampas acima de
`max_slope`) e confirma a posição elevada com um shape cast **curto (0.12 m)**
— num lanço de escada o degrau seguinte está sempre a menos de um raio.

Medido no spike do Gate 0 (`/home/maikeu/GitClones/.spikes/tnua_gate`):
repouso sem deriva a `float_height`; escadas de 0.20/0.30/0.40 m sobem; rampas
de 45°/55° sobem nativamente; 65° recusa; parede gera `CollisionStart` com a
entidade; salto de 1.4 m aterra na folga.

### Quem regista o quê (app própria)

O Vael compõe a App em `src/app.rs` (fim do `build_app`). A física entra por
três plugins, por esta ordem:

1. `viber::physics::PhysicsPlugin` — **é o do `bevy_ext_physics`** (o caminho
   reexporta o crate): Avian, o resolver de colliders pendentes e os gizmos.
   **Não conhece o herói** (é reutilizável).
2. `viber::physics::TerrainCollidersPlugin` — o streaming dos colliders das
   colunas voxel. É o único pedaço host-side: lê o `TerrainRuntime` e os
   `TerrainChunk`/`VoxelChunk` do monólito, por isso vive lá e não no crate.
3. `viber::player::HeroPhysicsPlugin::<VaelHeroScheme>::new(vael_hero_config)`
   — `TnuaControllerPlugin` + `TnuaAvian3dPlugin` (em `FixedUpdate`, o mesmo
   horizonte do `PhysicsPlugins::default()`), o asset de config do herói e os
   sistemas `attach_hero_controller`/`probe_steps`/`apply_steps`/
   `recover_stuck_ground`.

A engine standalone regista os mesmos três no `app.rs` dela, com o SEU scheme
(`HeroPhysicsPlugin::<HeroScheme>::new(hero_config)`); qualquer app nova tem de
fazer o mesmo com o scheme próprio, senão o herói fica sem controller (o
`GravityScale` é 0 e o corpo não cai nem anda) ou o terreno fica sem collider.

### Fim da duplicação (onda D, 2026-11-15)

O `src/physics.rs` do monólito tem agora ~200 linhas: reexporta o crate e fica
só com o streaming do terreno. Um só tipo de cada:

| Tipo | Dono |
|---|---|
| `PendingCollider`, `ColliderResolved`, `ColliderCache`, `BakedCollider`, `RayQuery`/`RayHit`, `TerrainCollisionStatus` | `bevy_ext_physics` |
| `VoxelCollider` | `bevy_ext_core` (o `bevy_ext_nav` filtra obstáculos pelo MESMO tipo que a física insere), re-exportado pelo `bevy_ext_physics` |
| `ColumnColliderBake`, `PHYSICS_CHUNK_RADIUS`, `collision_keep_within`, `column_xz_distance`, `nearest_column_distance`, `mesh_vertices_indices` | `bevy_ext_terrain_runtime::host` (o contrato do host para a física do terreno) |
| `ragdoll` | `bevy_ext_physics` (`pub use` no monólito) |

O bug que motivou isto, medido a 2026-11-15: registar só o plugin do crate
enquanto o spawn do monólito usava os tipos do monólito deixava o mundo sem
collider de terreno — `colliders_trimesh: 25` em vez de ~1159,
`terrain_collider_ready: false` para sempre e o `spawn_gate` a soltar o herói
aos 0.00 s. O herói parecia de pé porque os snaps analíticos do `src/player.rs`
o seguram à cota (andava na praça e saltava, mas atravessaria falésias).

O que **fica** de propósito no monólito: `physics_fx` (lê o `TerrainRuntime` do
monólito para `ground_near`, e o `apply_falling` usa os `WorldBorderConfig`/
`InteriorSceneConfig` do host) e o `stream_voxel_colliders`. Ambos migram se e
quando o terreno do monólito passar a usar o `bevy_ext_terrain_runtime`; nessa
altura o `TerrainCollidersPlugin` desaparece e o plugin do crate fica sozinho.

Correcção de arrasto: o `bevy_ext_terrain_runtime` declarava
`avian3d = { default-features = false, features = ["3d"] }` e o `Collider`/
`TrimeshFlags` do parry **não existem** sem `parry-f32` — compilado como
dependência a unificação de features escondia-o; `cargo test` no crate
sozinho rebentava. Passou a `["3d", "parry-f32"]`.

Gate da onda D, medido no mundo principal com o binário de release
2026-11-15 (o mesmo run que antes dava 25 trimeshes com os dois `physics`):
`colliders_trimesh: 1159`, `colliders_compound: 326`, `colliders_cuboid: 679`,
`rigidbodies: 4071`, `terrain_collider_ready: true`, herói grounded, andar
2,2 m em 1,5 s e salto de +1,4 m na amostragem (2,3 autorados) com aterragem.
Os raios de cima batem nos `chunk <x>-<z>` do terreno a y=38,73.

## Agenda

- `PhysicsPlugins::default()` — **`FixedPostUpdate`** (timestep fixo, 64 Hz). É
  o arranjo suportado: o `PhysicsTransformPlugin` do Avian injeta
  `mark_dirty_trees` no schedule e em `PostUpdate` isso colide com a instância
  do `TransformPlugin` do Bevy (o `jackdaw_runtime` ordena contra ela e o
  schedule rebenta ao inicializar: "more than one instance").
- Plugins do tnua em **`FixedUpdate`** (antes do step, como nos exemplos) e o
  step assist (`probe_steps`/`apply_steps`) atrás da lógica do tnua.
- O `player_movement` continua no `Update` e escreve a basis/intenção; ela é
  consumida no próximo step fixo (até um frame de latência).
- A câmara do Vael corre em `PostUpdate` (depois do `FixedPostUpdate`, logo lê
  a pose já escrita pelo writeback; a ordenação contra `PhysicsSystems` é
  documental, atravessa schedules).
- Sem interpolação, a 60 fps contra 64 Hz fixos há um micro-judder; se
  incomodar, ligar `TransformInterpolation` do Avian.

## Contactos e sensores

- `CollisionStart`/`CollisionEnd` + `CollisionEventsEnabled` (substitui
  `ActiveEvents`). O herói é dinâmico, por isso os pares com estáticos e
  cinemáticos geram contacto — o `ActiveCollisionTypes::KINEMATIC_STATIC` que o
  Rapier exigia para o KCC **desapareceu**.
- A flag de sensor deixou de vir no evento: pergunta-se ao componente `Sensor`.
- **Limitação do Avian**: pares non-dynamic (kinematic↔static) não geram
  eventos. Os sensores com script contra criaturas (corpos cinemáticos) teriam
  de ser cobertos por `shape_intersections`; nenhum script do Vael depende
  disso hoje.
- O caminho `KinematicCharacterControllerOutput.collisions` do `luau/contacts.rs`
  foi removido: o herói dinâmico toca de facto nos sólidos, e o evento chega.

## Raycasts fora de sistemas

O `SpatialQuery` do Avian é um `SystemParam`. Para os sítios que correm em
sistemas exclusivos ou operadores (`&mut World`), `viber::physics::ray_from_world`
executa um sistema cacheado e devolve `RayHit { entity, distance, normal, point }`.
`RayQuery` tem `solid` (raio dentro de geometria) e `include_sensors`
(o `viber.raycast` inclui; o chão/oclusor não). Usam-no: sentidos (LOS),
picking da cena, bridge e o chão de teleporte.

## Ferramentas e debug

- Wireframe: `PhysicsDebugPlugin` + `GizmoConfigStore`/`PhysicsGizmos` (o
  antigo `DebugRenderContext`).
- Pausa: `Time<Physics>` + o trait `PhysicsTime` (`pause`/`unpause`).
- `viber.debug.physics` mantém a forma (a chave JSON continua `rapier`), mas o
  Avian não expõe os contadores do pipeline: `step_ms` vem das âncoras do
  profiler e as fases ficam a zero.
- `Sleeping` passou a **marcador**; `viber.debug.stats`/painel Física contam
  presença, não estado.
- Ragdoll: as juntas são **entidades** no Avian (`SphericalJoint`), guardadas
  em `RagdollActive.joints` e recolhidas com os corpos.

## Jackdaw

`jackdaw.toml` fica com `[modules] physics = false`. O módulo opcional
`jackdaw_avian_integration` fixa `avian3d 0.8` a partir de um **fork**
(`jbuehler23/avian`): ligá-lo punha duas cópias do Avian no build (tipos
`Collider` distintos, o mesmo problema dos "gémeos" de crates). Os gizmos de
colisor do editor continuam no `vael_scene_jackdaw`, que lê `ColliderSpec` por
reflexão e não precisa do módulo.

## Limpeza e fecho (2026-11-14, segunda passagem)

- **Bug corrigido — `collider="auto"` com metade do tamanho**: os bounds
  renderizados são meias-extensões (`Aabb::half_extents`) e o `cuboid` do
  Avian leva tamanho completo. `bounds_collider` dobra os valores e há teste de
  regressão (`auto_bounds_collider_has_the_full_size`). O mesmo erro foi
  auditado nos restantes construtores (`box`/`cylinder`/`capsule`/cone,
  primitivas, NPC, ragdoll, gradmesh) e não existe.
- **Código morto**: `HERO_JUMP_HEIGHT` removido (a altura vem do
  `Player.jump_height`); `scripted_collision_bits` já não existe.
- **Avisos**: engine e Vael sem avisos próprios (restam os pré-existentes de
  `cfg(dlss)`, `distance_to_path` e os do CelerAI).
- **Textos legados**: UI (`Bevy · Avian`), descrições do profiler, ajuda do
  `viber.debug`, `bridge/selection` (`physics_toi`), docs do gradmesh e do
  engine (`AGENTS.md`) actualizados. Os locks do Vael e do engine já não têm
  `bevy_rapier3d`/`rapier3d`.
- **Workspace**: `avian3d = "0.8"` acrescentado às dependências; o
  `bevy_rapier3d` fica só para os crates extraídos excluídos do workspace
  (`bevy_ext_physics`, `bevy_ext_terrain_runtime`,
  `bevy_ext_ai`, `bevy_ext_nav`, `bevy_ext_engine`; o
  `bevy_ext_scene_runtime` foi apagado a 2026-11-15), que não compilam nem
  entram em lock — portá-los é follow-up, não gate.
- **`.jackdaw/components.json` é cache local** (não versionado): ainda lista
  `bevy_rapier3d` porque foi gerado antes; o `jd open .` regenera-o com os
  tipos Avian/tnua registados.

## Gates de QA

**Verificado nesta migração (2026-11-14):**
- `cargo check --lib` no engine, no Vael e no `bevy_ext_gradmesh`.
- `cargo test --lib` do Vael: **378 testes, 0 falhas** (inclui movimento do
  herói sem corpo, primitivas, água, barco).
- Jogo vivo (`--scene main --entry dev`, Avian): mundo carrega com 89k
  entidades, 1159 trimeshes, 4071 corpos; herói **de pé** a
  `superfície + 0.04` com `grounded=true`; andar 6 m em 1.5 s; salto com pico
  2.36 m e aterragem que volta a `grounded=true`; `viber.raycast` bate no
  trimesh da coluna; `viber.debug.colliders` mostra os props em `Static` com
  as meias-extensões certas.
- Spike do Gate 0 (fora do jogo): escadas 0.2/0.3/0.4 m, rampas 45/55/65°,
  parede com `CollisionStart`, salto, repouso — com chão em cubo **e em
  trimesh**.

**Verificado na revisão (2026-11-15, app própria composta):**
- `cargo check --lib` do monólito, do `bevy_ext_physics` e do Vael.
- Cena `menu` viva (`--scene menu --bridge`, `VAEL_ELEMENTS=0`):
  `terrain_collider_ready: true`, `colliders_trimesh: 32` para 16 colunas,
  herói `grounded=true`, e o `viber.raycast` (BRP) bate em `chunk 1-1` a
  y≈8.2 em quatro pontos da clareira. Com o plugin do crate extraído (tipos
  distintos) o mesmo run dava `terrain_collider_ready: false` e nenhum
  trimesh de terreno — é o gate da onda D.
- `viber.debug.physics()` devolve as fases do Avian (`step_ms` 3.2 ms na
  cena leve, `ncontacts`/`nconstraints` > 0).

**Verificado na onda D (2026-11-15, tipos unificados):**
- `cargo test`: `bevy_ext_physics` 44, `bevy_ext_terrain_runtime` 5 (os
  `ColumnColliderBake`/distâncias mudaram-se para o `host`), Vael 381 e
  monólito **813** (as 4 falhas que restam são do `luau`/`bevy_ext_luau` da
  wave F, não da física). A suite do `bridge::tests` não é paralela
  (`--test-threads=1`): vários testes abrem a mesma porta/estado global.
- Mundo principal (`--scene main --entry dev`, binário de release pós-merge
  `exit4/integrate`): `colliders_trimesh: 1159`, `colliders_compound: 326`,
  `colliders_cuboid: 679`, `rigidbodies: 4071`, `terrain_chunks: 739`,
  `terrain_collider_ready: true`; herói a `38.7704` (superfície + 0.04) com
  `grounded=true` e `vel_y≈0`; andar 6,0 m em 1,5 s; salto 38.77→40.79→38.77.
- **Cuidado no arranque**: o `wait-ready` sai antes de as colunas sob o herói
  estarem assadas. Nessa janela o `spawn_gate` larga-o com
  `TerrainCollisionStatus.ready == false`, o chão analítico põe-no dentro do
  collider e ele cai (o `src/player.rs` resgata-o; medido 5× na carga). Fica
  bom sozinho em ~10 s. Gate de QA: poll de `viber.debug.stats()` até
  `terrain_collider_ready: true` antes de andar.
- Cena `menu`: `terrain_collider_ready: true`, 32 trimeshes para 16 colunas e
  os mesmos hits de `chunk 1-1` — sem regressão face ao estado anterior ao
  switch.

**Pendente / bloqueado:**
- `cargo test --lib` do engine: bloqueado pelo WIP de `terrain/water.rs` de
  outra sessão (código de teste a usar itens privados do `bevy_ext_water`).
- Perf p95 do passo: a máquina estava com load ~10 e dois `rustc` concorrentes
  (4-6 fps no jogo); o `physics.step` medido foi 0.55 ms. Repetir com a máquina
  limpa e comparar com `PERF_BASELINE.md` (1.5/2.2 ms).
- Escada real da Cidadela e natação em água funda: o spike cobre a física, mas
  faltam as coordenadas do mundo para o teste de jogo.
- Fixtures de paridade Rapier: não capturadas (a troca de dependências
  aconteceu com outras sessões a segurar o worktree). `tools/avian_baseline.py`
  (`capture`/`compare`) fica pronto para um checkout limpo.
- `VIBER_CONTACT_DEBUG=1` (bump dos NPCs) e save/load: não exercitados.

## Gotchas

- **`CollisionEventsEnabled` (e `Sensor`) têm de ser inseridos ANTES do
  `Collider`**: o Avian cria o proxy da árvore de colliders no
  `Add<Collider>` e **não tem observer** para `CollisionEventsEnabled` —
  inserida depois, o proxy fica com a flag a zero para sempre e o collider
  nunca emite `CollisionStart`/`CollisionEnd` (todos os contactos com script
  morriam em silêncio). `stamp_collider_role` documenta a ordem e todos os
  sítios de spawn (props, resolver pendente, primitivas, spawner, gradmesh)
  inserem o papel primeiro. `Sensor` tem observer, mas segue a mesma ordem.
- **`MessageReader` em apps sem física**: `dispatch_contacts` recebe
  `Option<MessageReader<CollisionStart/End>>`; apps de teste/headless sem
  `PhysicsPlugins` não têm as mensagens inicializadas e o sistema panicava.
- **Snaps analíticos têm de deixar a folga do `float_height`**: o sensor de
  chão do tnua é um disco (altura 0) no *pé* do herói. Se o chão analítico (ou
  o piso de interior) puser os pés exactamente na superfície, o disco fica
  coplanar com o trimesh, o `shape_cast` com `ignore_origin_penetration` não
  devolve hit, o tnua considera o herói airborne e a gravidade prende-o ali
  (`grounded=false`, `vel_y≈0`, herói não anda). Todos os snaps escrevem
  `superfície + HERO_FLOAT_HEIGHT`. O disco em si funciona contra trimesh
  (verificado no spike com o chão em trimesh); o problema é só a coplanaridade.
- **Aterragens duras voltam a colar o corpo ao chão** (o mesmo disco coplanar
  após o contacto). `recover_stuck_ground` desencrava: airborne com
  `|vel_y| < 0.5` durante 0.2 s → sobe `float_height + 0.01` e o sensor volta
  a ver o chão. O limiar evita o apex do salto (~8 ms com `v` pequena).
- **`PhysicsGizmos` nasce LIGADO**: adicionar o `PhysicsDebugPlugin` desenha o
  wireframe de todos os colliders do mundo e enche a VRAM (o Rapier era
  `RapierDebugRenderPlugin::disabled()`). O `PhysicsPlugin` desliga-o no
  `Startup` (liga só com `VIBER_PHYSICS_DEBUG`/`--physics-debug` ou pelo toggle
  do profiler).
- **Não pôr o Avian no `PostUpdate`**: duplica o `mark_dirty_trees` do Bevy e
  qualquer ordenação contra ele (o `jackdaw_runtime` faz) panica ao inicializar
  o schedule. Usar `PhysicsPlugins::default()` (`FixedPostUpdate`).
- `app.finish()` é obrigatório fora de `App::run()` (testes/sistemas
  headless): o `SpatialQueryPlugin` regista os diagnostics no `finish` e os
  sistemas de raycast/shape cast panicam sem o recurso.
- Sem `RenderPlugin`, o `ColliderCachePlugin` precisa de `init_asset::<Mesh>()`
  + `add_message::<AssetEvent<Mesh>>()`.
- `SpatialQuery` (lê `Position`) e `Query<&mut Position>` no mesmo sistema
  conflituam (B0001): planear num sistema, aplicar noutro.
- Escrever `Transform` num corpo dinâmico não faz nada: o writeback reescreve-o
  a partir de `Position`/`Rotation`. Teleportes e o chão analítico escrevem
  `Position`.
