# Árvores procedurais (`bevy_flora`) e a colheita

O mundo não desenha árvores GLB. Cada árvore autorada é uma **estação**
(`<Entity>` com pose, collider de tronco e props de colheita) e a árvore
procedural vive **na própria estação** — não num filho. O `bevy_flora` pode
assim tomar conta dela: tombar, abanar, largar folhas e recolorir.

O porquê: os GLB de árvore do pool pesam ~12k triângulos cada, 5–23 MB com as
texturas, e cada instância carregava a sua subárvore de LOD da engine.

## Onde vive

- `src/flora.rs` (`VaelFloraPlugin`): pools por espécie, plantio das estações,
  looks por altitude, vento/chuva, folhas ao vento e `SeeThroughTrees`.
- `src/harvest.rs`: golpe, loot/XP, tombo com o `bevy_flora`, tocos, toros e a
  persistência do estado tombado.
- `tools/flora_migrate.py`: migra o mundo (XML) de `<GltfScene>` para estações.
- `../Viber/src/water_ssr.rs` e `../Viber/src/postfx.rs`: guards que ignoram
  câmaras offscreen (o bake dos impostores) — sem eles a lente misturava
  alvos de tamanhos diferentes e o wgpu rejeitava o frame.

## A estação

```xml
<Entity name="inst-grp-18-0000" translation="…" euler="…" scale="…"
        rigidbody="type: fixed; mass: 0; gravity-scale: 0"
        collider="shape: cylinder; radius: 0.45; half-height: 2.2"
        destructible="popup-text: Madeira Escura; …; break-style: fall; cut-height: 0.7"
        resource-kind="dark-wood" resource-yield="3"
        prop-flora="pine_dark" />
```

- `prop-flora` (→ `ScriptProps["flora"]`) escolhe a espécie/pool.
- A **pose manda**: a `PooledTree` é inserida na estação, que já tem
  `Transform` (pose/escala da instância) e collider.
- A altura do GLB autorado é **absorvida no pool** (`height_scale` aplica-se a
  `trunk.height`, `trunk.radius` e `foliage.leaf_size`): a árvore nasce à
  escala certa com a escala da estação.
- Os props de colheita ficam intactos; `cut-height` é lido para o corte.

## Espécies e pools

As espécies vivem na cena: `world/settings/flora.bsn` tem um `FloraSettings`
(LOD 45/110 m, impostor 260 m, raio de sombra 90 m, look frio acima de 55 m) e
uma entidade `FloraSpecies` por espécie (`key` = `prop-flora`, `variants`,
`cold_look` e o `TreeParams` completo do `bevy_flora`). Editam-se no Jackdaw
(inspector + painel **Vael: Flora**, com preview real das variantes e das
árvores nas estações); gravar reconstrói os pools no jogo a correr. A tabela
abaixo é a origem (`POOLS` em `src/flora.rs`, usado por
`vael --export-settings` e como reserva se a cena não tiver espécies).

| `prop-flora` | Instâncias | Preset | Variantes |
|---|---|---:|---:|
| `pine_dark` | 487 | Pine | 8 |
| `dead_tree` | 367 | DeadTree | 5 |
| `dead_willow` | 342 | Willow (sem folha) | 6 |
| `tree_oak` | 184 | Oak | 8 |
| `tree_pine` | 150 | Pine | 6 |
| `tree_birch` | 94 | Birch | 6 |
| `palm_tree` | 10 | Palm | 3 |

Total: **1634 estações** (1617 no `world/`, 17 em `scenes/menu`).
Os pools são construídos no `Startup` (`build_pool`, ~100 ms uma vez — o
segundo look só existe antes de o pool ir para os assets). LOD: 45 m / 110 m
e **impostor** a 260 m (um quad por árvore, com 8 vistas do bake).

**Dois looks por pool**: 0 = cores da espécie; 1 = frio (mais azul, menos
saturação) para as árvores acima de 55 m locais (serras e planaltos). Partilham
os meshes; mudam só os materiais.

## Colheita (felling do crate)

- Golpes: `[J]` (o prompt "Cortar"), com tremor da copa a cada golpe
  (`TreeShakes` + `FallingLeaves::per_shake`).
- Último golpe: em vez do `Falling` da engine, insere
  `bevy_flora::FellTree { direction, cut_height, speed: 1.0, lying: false }` e
  troca o collider pelo do **toco** (cilindro curto). A árvore tomba com o
  tronco assente no relevo (`FloraGround`) e o crate larga folhas
  (`per_landing`) e emite `TreeLanded` → som de queda (`ChopBreak`).
- Toros: `[J]` na árvore deitada publica "Cortar em toros" e insere
  `BuckTree { log_length: 2.2, min_radius: 0.08 }`; o crate substitui a árvore
  por `TreeLog`s no chão e emite `TreeBucked`.
- Persistência: `flora-fell.<chave>.{dx,dz,cut,bucked}` no `VaelSave.legacy` e
  no `viber.game()`. No load, o `reap_saved` **não despawna**: repõe
  `FellTree { lying: true }` (já deitada, com toco) e, se `bucked`, os toros.
  O reap do `harvest.lua` salta as chaves `flora-fell.*` (o restauro é do
  Rust) e o reap/embate/toros do Rust correm **sem o gate** `VAEL_HARVEST_RUST`.
- **Os dois caminhos de colheita** fazem o mesmo: o Rust (`VAEL_HARVEST_RUST=1`)
  chama o tombo internamente; o **Lua (default)** chama
  `viber.vael.fell_tree(id)` no ramo `break-style: fall` e o Rust move a
  árvore para a raiz com o `FellTree`. Sem essa função o Lua caía no
  `viber.topple` (tomba a estação e despawna, sem toco).
- Props antigos (pré-flora, sem `PooledTree`) continuam com o `Falling` da
  engine.

## Vida visual

- **Vento/chuva**: `FloraWind`/`FloraWetness` seguem o `WeatherState`.
- **Folhas**: `FallingLeaves` (rajadas de shakes e embates) e `DropLeaves` até
  6 árvores num raio de 60 m quando o vento passa de 0.7 (cooldown 2 s).
- **See-through**: `SeeThroughTrees` na câmara do jogo (alvo = peito do herói,
  raio 4 m, `keep 0.35`); só a câmara da janela — as do bake são
  `RenderTarget::Image`.
- **Looks**: por altitude (ver acima).

## Gates

| Variável | Default | Efeito |
|----------|---------|--------|
| `VAEL_FLORA` | ligado | `0/false/off` desliga o plantio |
| `VAEL_FLORA_GROVE` | ligado | `0` desliga só a mata da entrada norte |
| `VAEL_FLORA_SHADOW_RADIUS` | 90 | raio (m) das sombras das árvores |
| `VAEL_FLORA_IMPOSTOR` | 260 | distância do impostor (m); `0` desliga |

## QA pela bridge

As teclas sintéticas (`viber debug key j`) passaram a ser aplicadas no
`PreUpdate` do frame seguinte, depois do input core: antes eram escritas a
meio do `Update` e o `harvest_input`/scripts que já tinham corrido não viam o
`just_pressed` (o `[J]` do QA não colhia). Funciona também com
`viber debug step` (frames determinísticos).

## Limitações

- **Sem entalhe** (`TreeNotch`): reconstrói o tronco e não se aplica a meshes
  partilhadas de um pool. O corte é a `cut_height` do primeiro golpe.
- **Sem estações/outono**: o mundo é de uma estação; ficam as folhas a cair e
  o look frio por altitude. `FloraSeason` de neve exigiria um sinal de neve do
  ambiente.
- O tronco deitado não colide (o toco sim).
- Uma árvore cortada passa a `ProceduralTree` individual (~35k tri): o corte é
  raro, mas essa árvore deixa de ser instanciada.
- QA em ambiente com `bevy_elements`: o `CloudView` do crate pode rebentar com
  uma entidade despawnada (bug em curso no ambiente); para QA de flora correr
  com `VAEL_ELEMENTS=0`.

## Validar

```bash
cargo check --release && cargo test --release
python3 tools/flora_migrate.py --check
VIBER_NO_BUILD=1 viber analyze world.xml
```

Em jogo (`--scene main --entry dev --bridge 15706`): colher um pinheiro com
`[J]`×3 → tomba com toco e som de queda; `[J]` na árvore deitada → toros;
gravar e recarregar → a árvore volta deitada (sem tombar outra vez). O log não
pode ter `water_ssr` (impostores ligados). Medir com
`viber debug prof --tab frames` na mesma pose, sem builds concorrentes.
