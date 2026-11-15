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
  Vael): `attach_hero_config` cria-o e insere o `TnuaConfig`.
- Na **natação** o controller é desligado (`TnuaToggle::Disabled`) e o
  `LinearVelocity` é nosso (empuxo, corrente, impulso para a margem). Lunge e
  knockback somam-se à `desired_motion`.
- O salto é a ação `HeroScheme::Jump` (altura no asset); o buffer/coyote/
  cooldown continuam no `player_movement`.

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
