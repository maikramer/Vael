# Jackdaw: o que se autora e onde

O editor oficial é o **Jackdaw 0.20 instalado** (`jd open .`); o mundo é
`.bsn` e o `AGENTS.md` é o contrato. Este doc responde a uma pergunta só:
**para cada sistema do jogo, onde está hoje a autoria** — no editor (com
campos e gizmos), em dados do jogo (`world/…`, `data/…`) ou em código.

## Build e run

As formas oficiais de buildar e correr o jogo são as do Jackdaw, corridas na
raiz do projeto:

| Comando | O que faz |
|---|---|
| `jd doctor .` | Verifica o setup: toolchain, `jackdaw.toml` (`package = "vael"`, `plugin = "VaelPlugin"`), `JackdawPlugin`, dependências |
| `jd build .` | Compila o jogo com o perfil `dev` deste workspace em `.jackdaw/target/game` e regrava `.jackdaw/{schema,components}.json` — o schema é o que o editor carrega depois de mexer em componentes |
| `jd run .` | `jd build` + lança o binário, **sem argumentos** |
| `jd clean` | Apaga a cache de build do projeto (`.jackdaw/target`); `--cargo` limpa também o `target/` do workspace. Corre-se no projeto (não aceita path) e com o editor fechado |
| `jd open .` | Editor; o Play/PIE corre o mesmo binário |

Os `[[run]]` do `jackdaw.toml` (Play, Play (título), Play (QA)) são do botão
Play do editor: o `jd run` da linha de comandos ignora-os e arranca na cena de
arranque (`scenes/menu.bsn`, atrás do título), não em `main`. Para argumentos
(`--scene main --entry dev --bridge`) usa-se o `vael-tools run`; com
`VAEL_BIN=.jackdaw/target/game/debug/vael` esse atalho reaproveita o binário do
`jd` em vez de exigir um build release em `target/`. Nota: `jd run` e o Play
concorrem pela mesma pasta de build (`.jackdaw/target/game`) — não correr os
dois ao mesmo tempo com código a mudar.

## Editar BSN pela CLI

Na raiz do Vael, `vael-tools bsn inspect world/paths/network.bsn --tag RoadNetwork --json` localiza o nó físico da rede de estradas. Use `show` para ler seu trecho original, `set` para editar uma propriedade, `patch` para aplicar um lote e `insert` para acrescentar uma subárvore. Os três comandos de escrita aceitam `--dry-run`. Depois de gravar, `vael-tools bsn check world.bsn` abre a árvore pelo loader do jogo. A [skill de edição BSN](../.cursor/skills/vael-bsn/SKILL.md) explica seleção por ID, consulta ao schema e edição de prefabs.

## Navegar e ajustar durante o Play

Abra `world.bsn` no Jackdaw e deixe **Scene** e **Game** visíveis em duas
docas. Clique em **Play** para entrar no mundo pela entrada de desenvolvimento.
O painel **Vael: Debugger** encontra o jogador automaticamente enquanto está
aberto e marca a posição dele com um anel na vista **Scene**. A posição vem do jogo pela porta local 15702. Para usar outra porta,
inicie o editor com `VAEL_TOOLS_PORT=<porta> jd open .`.

| Na vista Scene | Ação |
|---|---|
| Arrastar com o botão do meio | Deslocar a vista mantendo o ponto do mundo debaixo do cursor |
| Alt + arrastar com o botão do meio | Orbitar esse ponto em perspectiva |
| Botão direito + WASD | Voar. Shift acelera e a roda com o botão direito ajusta a velocidade |
| **Enquadrar seleção** no Debugger | Ir até o objeto selecionado no Outliner |
| **Localizar jogador** | Enquadrar a posição atual do jogador na cena |
| **Seguir / soltar jogador** | Acompanhar o deslocamento do jogador enquanto se navega. Clique novamente para soltar a vista |

Para experimentar a ligação, abra **Vael: Ambiente**, escolha **23:00** e
depois **12:00**. Observe o céu em **Game** com o Play ainda em execução.
Hora, clima e pós-processamento aceitam edições sucessivas pelo painel ou
pelo Inspector. Nos objetos do mundo, experimente mover um modelo ou mudar
uma propriedade de script. Para objetos internos de um prefab, abra o
arquivo do módulo no seu próprio separador. Os ajustes chegam sem gravar o
`.bsn`. Nas preferências de Play, **Revert** descarta as edições de documento
ao parar e **Keep** permite mantê-las e gravar depois.

## Matriz

| Sistema | Onde se autora | Como se vê no editor |
|---|---|---|
| Geometria, hierarquia, prefabs | `.bsn` (Outliner/viewport) | directo |
| Colisores, corpos, destrutíveis, colheita | `ColliderSpec`/`BodySpec`/`Destructible`/`Harvestable` | inspector *Physics*/*Gameplay* + gizmos |
| LOD, flora, partículas, portais, exclusões, scripts | `ModelLod`/`FloraStation`/`ParticleSpec`/`PortalSpec`/`SpawnExclusionSpec`/`ScriptRef` | inspector + gizmos |
| Terreno, caves, arcos, falésias, paint | `VoxelTerrain`/`Cave`/`Arch`/`Cliff` + `world.data/` | painel *Voxel Terrain* |
| **Pavimento (ruas, praças, largos)** | `paving="1"` num `<GroundDecal>`/`<Road>`/`<RoadNetwork>`, com `paving-cell`/`-corner`/`-priority` (ver [`PAVING.md`](PAVING.md)) | inspector, nos atributos do nó — ainda sem gizmos da grelha |
| Ambiente, PostFX, flora (definições) | `world/settings/*.bsn` | painéis *Ambiente*, *PostFX*, *Flora* |
| **Câmara do jogo** | `CameraSpec` no `<OrbitCamera>` (`world/autoload/player.bsn`) | inspector *Câmara* + frustum no viewport |
| **Câmara de interior** | `InteriorSceneSpec` nas salas (`scenes/interiors/*.bsn`) | inspector *Câmara* + rectângulo da sala e pose da câmara |
| **Áudio e biomas** | `AudioMixerSpec`/`MusicLayerSpec`/`BiomeRegionSpec` (`world/environment.bsn`, `scenes/menu.bsn`) | inspector *Áudio* + contorno do bioma |
| **Cinemáticas** | `<Cinematic>`/`<Shot>` em `world/cinematics.bsn` | painel **Vael: Cinemáticas** + trajecto dos planos |
| **Gatilhos de cinemática** | `vael::cinematic_trigger::CinematicTrigger` na entidade | inspector *Câmara* + anel do raio |
| Quests, diálogos, itens, marcos, progressão | `quests/*.json`, `dialogues/*.json`, `data/*.json` | painéis *Missões*, *Diálogos*, *Itens*, *Marcos*, *Progressão* |
| Regras de combate/economia/vitais, IA, HUD | Rust (`src/*.rs`)/Lua | — (não autorável) |

As contagens vivem nos painéis, não aqui.

## Componentes tipados (o mecanismo)

O runtime continua a ser o parser do Viber; o que muda é onde o dado mora.
Cada tipo do `bevy_ext_scene` implementa `AttrSpec`: ida e volta **exacta**
com os atributos do DSL (números comparam-se por valor, para `"1.0"` poder
voltar como `"1"`). O loader escreve-os de volta nos atributos antes de
derivar nomes/poses, por isso o jogo não sabe a diferença.

- Migrar: `cargo run -p bevy_ext_world --example bsn_migrate -- <ficheiros.bsn>`
  (idempotente; só move o que volta igual).
- Gate: `cargo run -p bevy_ext_world --example bsn_equiv -- antes.bsn depois.bsn`
  (tem de dizer `equivalentes`).
- O lift é **por tag**: `at`, `size`, `id`, `radius` e `height` existem em
  muitas tags e um lift global mudava o significado do atributo.
- Um tipo que o nó **já tem** como componente não é levantado outra vez (um
  ficheiro gravado pelo editor pode ter o componente e o atributo homónimo ao
  mesmo tempo; duplicar o componente mudava o documento).
- `Option<T>`: o emissor grava `Option::Some(…)` ou o valor nu; o leitor dos
  specs normaliza (`specs::normalize_options`). Não confiar no `try_apply`.

Um componente **do jogo** (`vael::…`) não passa pelo `AttrSpec`: o
`editor_link` aplica-o como *componente nativo* a partir do patch do
documento. É assim que o `CinematicTrigger` chega à entidade.

## Câmara

`CameraSpec` cobre `distance/height/pitch/fov/follow-lag/turn-lag/
min-terrain-distance/target/mouse-sensitivity`. O comportamento (follow com
low-pass, colisão com o terreno, auto-câmara, shake/kick) continua no
`src/camera.rs`; os campos não mudam a física, só os valores autorados.

`InteriorSceneSpec` cobre `at/size` (sala) e `camera-distance/camera-pitch/
camera-yaw`. No Vael, a distância define o zoom, o yaw orienta o movimento e a
câmara acompanha o herói com uma zona morta. A panorâmica abranda nos limites
de `at/size` e a vista sobe junto à parede da frente. Eixos com `room-size`
explícito mantêm o enquadramento por divisão.

Durante uma cinemática os sistemas de câmara do Vael **não correm**
(`no_cinematic`): o `drive_camera` da engine é o único dono do `OrbitCamera`.

## Cinemáticas

Autoria no painel **Vael: Cinemáticas** (janela nova; por omissão na doca de
baixo, abre pelo menu de janelas do editor):

- lista os filmes e os planos com o tempo **acumulado** (o parser encadeia
  planos por ordem de documento; não há `start` autoral);
- `◀/▶` filme e plano, "Seleccionar plano" (Outliner + frame);
- **Tocar no jogo** / **Parar**: JSON-RPC para o bridge do jogo
  (`VAEL_TOOLS_PORT`, por omissão 15702). O jogo tem de estar no Play;
- **Novo plano (viewport)**: cria um `<Shot>` no fim do filme com
  `from`/`look` da câmara do viewport (`duration=4`), como um passo de undo.
  Se a escrita do componente falhar, a pose fica no painel como texto para
  colar no inspector;
- **Pose da câmara**: só mostra `from="…" look="…"` (para afinar à mão).

Gizmos: trajecto `from→to` (âmbar), alvos `look` (azul), planos em foco
destacados; o filme em foco sempre, os outros no modo "perto".

A autoria e o play/stop de cinemáticas são do painel **Vael: Cinemáticas**
(lê a `CinematicLibrary` carregada do `.bsn`). O CLI `viber cinematic` foi
aposentado; não há caminho XML para este mundo.

### Gatilhos

Uma entidade com `CinematicTrigger { cinematic, radius, once }` dispara na
**entrada** do raio (não enquanto fica dentro), respeita menus e cenas a
tocar e, com `once`, grava `cine.<id>` em `VaelSave.markers` (lê também o
`cine_<id>` legado dos saves antigos). O script antigo
(`scripts/cinematic-trigger.lua`) foi removido.

No editor o tipo é um **espelho** compilado na extensão
(`../bevy_extensions/extensions/vael_scene_jackdaw/src/trigger.rs`) com o mesmo caminho de tipo:
os tipos que só vêm no schema do jogo ficam no documento e não se leem por
ECS — sem espelho não haveria ícone, anel do raio nem verificação do id. Os
campos dos dois lados têm de coincidir.

## Edição ao vivo

O contrato completo está no `AGENTS.md` ("Jackdaw no jogo"). Em resumo: sob
o Play (`JACKDAW_PIE`) as entidades levam `SceneNodeId`; `Transform` e
componentes reflectidos aplicam-se directo; mudar `WorldNode`/specs re-spawna
o nó pela engine (`live_edit.rs`); só props (`CustomProperties`) faz patch do
`ScriptProps` sem reiniciar. Nós internos de módulos precisam do separador do
módulo aberto.

## QA

1. `jd build && jd extension pack && jd extension install --trust-publisher
   vael_scene_jackdaw-<versão>-*.jdext` (subir a versão para o instalador
   aceitar; ele compara versões, não conteúdo).
2. `jd open .`: inspector com *Câmara*/*Áudio*; gizmos de câmara, sala,
   bioma, gatilho e planos; painel lista os filmes do `cinematics.bsn`.
3. Play + "Tocar no jogo": a cena corre sem brigar com a câmara do Vael.
4. Gatilho: andar até ele uma vez (toca), sair e voltar (não repete);
   o save fica com `cine.<id>` nos marcadores.
