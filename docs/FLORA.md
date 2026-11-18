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
- `../bevy_extensions/crates/bevy_ext_viber_engine/src/water_ssr.rs` e
  `.../src/postfx.rs`: guards que ignoram
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

Total: **1634 estações** (1617 no `world/`, 17 em `scenes/menu`). A mata adicional
da entrada norte inclui duas cerejeiras (`tree_cherry`, preset Cherry, duas variantes).

Nos oito carvalhos adultos (sementes do pool, LOD0), o tronco e a folhagem
somam 294 076 triângulos. O modelo anterior somava 315 556. A copa usa
folhas individuais mais pequenas, distribuídas até mais perto do topo do tronco.
Os pools são construídos no `Startup` (`build_pool`, ~100 ms uma vez — o
segundo look só existe antes de o pool ir para os assets). LOD: 45 m / 110 m
e **impostor** a 260 m (um quad por árvore, com 8 vistas do bake).

**Dois looks por pool**: 0 = cores da espécie; 1 = frio (mais azul, menos
saturação) para as árvores acima de 55 m locais (serras e planaltos). Partilham
os meshes; mudam só os materiais.

## Colheita (felling do crate)

- Com o prompt **Cortar**, `J` vira o herói para a árvore e inicia a machadada. O clipe
  dura 0,90 s. Aos 0,26 s a lâmina alcança a casca, o jogo confirma o alcance
  e solta lascas do lado do jogador. A ferramenta acompanha a mão durante a recuperação.
- No último golpe, a árvore tomba para longe do herói. O `bevy_flora` calcula a
  aceleração da queda e o contato com o relevo. O estalo do corte e o som da
  copa batendo no chão acontecem em momentos distintos. O toco mantém colisão.
- Quando o tronco assenta, `J` com o prompt **Cortar em toros** divide a madeira
  em segmentos de 2,2 m. O toco continua a crescer mesmo que o tronco fique inteiro.
- Um dia de jogo após o corte, surge uma muda da mesma variante. Ela cresce
  gradualmente até ao terceiro dia, quando a estação recupera a colisão, a
  resistência e o rendimento originais. A madeira caída desse corte sai da cena.
- O save guarda a data fracionária do corte, direção, altura e estado dos toros
  em `flora-fell.<chave>.{day,dx,dz,cut,bucked}`. Um corte antigo sem data
  começa a contar no primeiro carregamento. O controlador Lua e o opcional
  `VAEL_HARVEST_RUST=1` partilham a queda, as lascas e o rebrote.
- Props antigos (pré-flora, sem `PooledTree`) continuam com o `Falling` da
  engine.

## Vida visual

- **Vento/chuva**: `FloraWind`/`FloraWetness` seguem o `WeatherState`.
- O calendário civil do HUD conduz as copas. Em março as cerejeiras da entrada
  norte florescem; carvalhos e bétulas recuperam as folhas ao longo da primavera.
  Em setembro começa a mudança para cobre e dourado. Em novembro as folhas
  desprendem-se e no inverno a silhueta dos ramos fica exposta. Os pinheiros
  mantêm as agulhas. A neve nas copas segue o manto medido junto ao herói.
- As folhas que o calendário desprende caem até 36 m da câmara. O conjunto de
  folhas em queda ou pousadas tem um teto de 1024. Entre avanços do relógio,
  a brisa de outono solta três folhas em até três árvores caducas próximas
  a cada 3,5 s. As rajadas fortes atingem até seis árvores num raio de 60 m.
- **See-through**: `SeeThroughTrees` na câmara do jogo (alvo = peito do herói,
  raio **1 m**, `keep 0.35`, `inner 0.6`, `in_front 0.9`); só a câmara da
  janela — as do bake são `RenderTarget::Image`. O cone é estreito de
  propósito: só o que está entre a câmara e o corpo esbate; um raio largo
  apagava árvores ao lado do herói.
- **Looks**: por altitude (ver acima).

## Onde observar a fauna

O Vael carrega os oito GLBs animados de `assets/public/assets/meshes/fauna/`.
O manifesto `assets/manifests/fauna.yaml` fixa as referências por seed, escala
e clips de cada espécie. Para regenerar o pacote a partir da raiz do jogo,
corre `tools/regen_remote.sh fauna` com o endpoint remoto configurado.
As instâncias da mesma espécie partilham a cena e o grafo de animação. Cada
animal controla o seu clip segundo o movimento, a proximidade de ameaças ou
a aproximação ao poleiro.

Na entrada norte de Discórdia, os gaios alternam entre os galhos das árvores
plantadas e clareiras onde pousam para bicar o chão. Alguns cruzam a trilha a
cerca de 1,7 m do solo. A aproximação do herói pode levantá-los. Ao anoitecer
chegam as corujas. Os chamados vêm de aves pousadas e seguem o volume de ambiente.

Na margem do lago, as andorinhas passam em bando sobre a água. Há lebres nas
clareiras do portão, do lago e do rio. Elas alternam repouso e saltos e fogem
quando o herói ou uma raposa se aproxima. As raposas percorrem a orla da mata,
os ratos rondam as ruelas, as borboletas visitam as clareiras de dia e as
trutas sobem à superfície no lago e nos rios.

## Gates

| Variável | Default | Efeito |
|----------|---------|--------|
| `VAEL_FLORA` | ligado | `0/false/off` desliga o plantio |
| `VAEL_FLORA_GROVE` | ligado | `0` desliga só a mata da entrada norte |
| `VAEL_FLORA_SHADOW_RADIUS` | 90 | raio (m) das sombras das árvores |
| `VAEL_FLORA_IMPOSTOR` | 260 | distância do impostor (m); `0` desliga |

## QA pela bridge

As teclas sintéticas (`vael-tools debug key j`) passaram a ser aplicadas no
`PreUpdate` do frame seguinte, depois do input core: antes eram escritas a
meio do `Update` e o `harvest_input`/scripts que já tinham corrido não viam o
`just_pressed` (o `[J]` do QA não colhia). Funciona também com
`vael-tools debug step` (frames determinísticos).

Para observar o ciclo na mesma pose, a bridge aceita
`viber.vael.env_set_date(2026, 10, 20)`. A hora continua independente
(`env_set_hour`); o save guarda a data e a hora para retomar a estação ao carregar o slot.

## Limitações

- **Sem entalhe** (`TreeNotch`): reconstrói o tronco e não se aplica a meshes
  partilhadas de um pool. O corte é a `cut_height` do primeiro golpe.
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
vael-tools analyze world.bsn
```

Em jogo (`--scene main --entry dev --bridge 15706`): colher um pinheiro com
`[J]`×3 → tomba com toco e som de queda; `[J]` na árvore deitada → toros;
gravar e recarregar → a árvore volta deitada (sem tombar outra vez). O log não
pode ter `water_ssr` (impostores ligados). Medir com
`vael-tools debug prof --tab frames` na mesma pose, sem builds concorrentes.
