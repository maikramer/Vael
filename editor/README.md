# vael-editor — autoria do Vael sobre Jackdaw

Editor do Vael. É uma App Bevy própria que regista uma extensão no
[Jackdaw](https://crates.io/crates/jackdaw) 0.20 (a mesma linha do Bevy
0.20.1 usada pelo jogo), abre as cenas `.bsn` do mundo e edita os JSON de
conteúdo que o runtime consome.

```bash
# a partir da raiz do Vael
cargo run -p vael-editor -- --root .

# ou, de qualquer pasta
cargo run -p vael-editor -- --root /caminho/para/Vael
```

`--assets` muda o asset root do Jackdaw (default: `<root>/assets/public`).
O pool é meshopt/KTX2; o editor regista o reader de assets do Viber para o
viewport carregar os GLB tal como o jogo.

### Editor Jackdaw standalone

A raiz do Vael também abre no editor oficial (`jd open .`, ou pela launcher).
O `jackdaw.toml` da raiz é a configuração desse editor:

```toml
plugin = "VaelPlugin"   # plugin raiz da lib (descoberta/PIE)
package = "vael"        # membro do workspace que o editor compila
```

O jogo depende de `jackdaw_runtime` com a feature `pie`, e o `VaelPlugin`
adiciona o `jackdaw_runtime::JackdawPlugin`. A descoberta de componentes
lança o binário com `--jackdaw-extract-schema` (o `main.rs` aceita a flag) e
o plugin responde com o schema refletido antes de abrir a janela. O `cargo
run` normal não muda: sem `JACKDAW_PIE` o runtime fica inerte.

O `jackdaw.toml` fixa `bevy = "0.20"`; o requisito do Cargo.toml tem de
admitir essa linha (`bevy = "0.20.1"`, como no `../Viber`). Um pin exato
(`=0.20.1`) é rejeitado pelo gate de import do Jackdaw.

## Mundo e cenas

O mundo é uma árvore `.bsn`:

- `world.bsn` + `world/**/*.bsn` — mundo principal e módulos `<Include>`;
- `scenes/menu.bsn` — cena de arranque (START-SCENE do `config.yaml`);
- `scenes/interiors/*.bsn` — áreas internas (salas; sem terreno);
- `world/autoload/*.bsn` — jogador/câmara/HUD persistentes.

O painel **Vael: Cenas** lista o mundo, o menu e cada interior; abre-os como
abas de cena do Jackdaw (`scene.open`). O jogo lê os mesmos `.bsn` e o
`viber scene`/`tool` continua a editar os `.xml` legados ao lado.

### `WorldNode`

Cada nó do DSL XML é uma entidade com:

```text
bevy_world_xml::bsn::WorldNode {
    tag: "Entity",                 # a tag original
    attrs: [WorldProp { key, value }],  # prop-*, script, collider, lod…
    text: "",
    origin_file: "world/…xml",     # preserva SceneSource/chaves de save
    origin_path: [0, 3],
    pose: true,
}
```

`Transform` e `Name` são a pose/nome (gizmos e hierarquia); `GltfSource` e
`PointLight`/`RectLight`/`DirectionalLight` são previews do viewport. O
**loader do jogo** traduz `Transform`/`Name`/`GltfSource`/luzes de volta para
atributos e reconstrói os `XmlNode`; o `tag`/`attrs` são a autoridade para o
resto (scripts, props, colliders, LODs, template de spawner, UI, etc.).

## Painéis de conteúdo

| Janela | Ficheiros | O que edita |
|---|---|---|
| `Vael: Cenas` | `world.bsn`, `scenes/**/*.bsn` | abre mundo/menu/interiores como abas |
| `Vael: Itens` | `data/items.json` | label, kind, order, stack, loja, uso, arma, requisitos, efeitos |
| `Vael: Marcos` | `data/landmarks.json` | name/label/bioma/raio/survey/peregrino |
| `Vael: Progressão` | `data/progression.json` | curva XP, `kill_xp`, talentos e efeitos |
| `Vael: Diálogos` | `dialogues/*.json` | start, nós, falas, escolhas, goto, flags de quest |
| `Vael: Missões` | `quests/*.json` + `index.json` | formulário completo, parley, stages, requires/next |
| `Vael: Conteúdo` | — | validação cruzada e falhas de carregamento |

As janelas abrem no dock inferior. **Tools → Vael** tem recarregar, gravar
tudo e validar. Os botões de cada painel gravam só o ficheiro do painel.

## Gravação de conteúdo

- `write_json_atomic`: escreve um `.tmp`, copia o ficheiro atual para
  `.bak` e faz rename. Um crash a meio não deixa JSON truncado.
- O JSON é escrito com 2 espaços e newline final, como os ficheiros atuais.
- Campos que o crate não conhece ficam em `extra` e sobrevivem à gravação.
- Campos opcionais ausentes não são materializados (`""`, `0`, `[]`).
- `quests/index.json` acompanha a gravação de quests (adiciona e ordena).
- Se um ficheiro falhou a carregar, gravar esse alvo é **recusado**: nunca
  escreve os defaults por cima de um JSON que não conseguiu ler.
- O undo do Jackdaw é do documento de cena (`.bsn`); as gravações de ficheiros
  não entram no undo (os operadores são `writes_disk`).

## Validação

O painel **Conteúdo** corre as regras de `vael-content`:
- quests: id/unicidade, npc no índice do mundo, bioma, objetivo (incluindo
  `craft`), `visit` em marcos, `collect` em itens, parley, recompensas,
  `requires`/`next`/`stages` e ficheiros fora do índice;
- diálogos: start/goto, texto obrigatório, locale `pt`/`en`, alcançabilidade
  e flags de quest;
- itens, marcos e progressão.

O índice do mundo é lido de `world.bsn` (ou `world.xml` legado) com o mesmo
`load_world` do runtime, que expande os includes, e de `scenes/**/*.bsn`
(atributos `name`). `Error` deve bloquear a gravação; `Warning` não.

## Limites desta versão

- **Grip** (preview da arma na mão) não foi portado: exige o herói carregado
  e attach ao osso. Os props `grip-*` continuam no XML/`equipment.lua`.
- **Terreno, composições e UI não têm preview** no viewport: o editor mostra
  a hierarquia e os attrs; o jogo continua a ser a referência visual.
- **CLI de cena XML-only**: `viber scene`, `viber tool settle/scatter` e
  `viber bake` operam nos `.xml` legados. `viber analyze` e `viber scene lint`
  já leem `.bsn`.
- **Play/PIE**: configurado. O `main.rs` passa
  `jackdaw_runtime::maybe_windowless` ao `build_app` pelo hook
  `AppOptions::default_plugins`, por isso o modo *embedded* desenha no
  viewport (`JACKDAW_PIE_WINDOWLESS`). Sem `JACKDAW_PIE` o runtime fica
  inerte e o jogo abre a janela normal.
- **Debug do jogo dentro do editor**: o `build_app` continua a carregar o
  mundo/scripts antes de o plugin do PIE arrancar; o schema de componentes
  (`--jackdaw-extract-schema`) sai depois dessa montagem.
- O primeiro build do editor compila o Jackdaw + a fachada Viber e demora; o
  `default-members = ["."]` do workspace garante que `cargo run`/`cargo test`
  do jogo não o compilam.

## Testes

```bash
cargo test -p bevy_world_xml   # round-trip XML→BSN→XmlNode
cargo test -p vael-content     # round-trip dos JSON reais + regras
cargo test -p vael-editor      # smoke sem janela (extensão, cenas, IO)
```

O smoke do editor usa `jackdaw_editor_core::testing::editor_test_app_with`
(sem Winit) e verifica as sete janelas, o carregamento do conteúdo e o índice
do mundo `.bsn`.
