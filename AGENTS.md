# AGENTS.md — Vael

Jogo. A engine é o checkout irmão `../Viber` (editor em `../Viber/editor`).
As ferramentas de geração são o checkout irmão `../AiGameKit` (CLIs no PATH).
Este ficheiro é curto de propósito. O contrato da engine está em
`../Viber/AGENTS.md`; lê a secção que a tarefa pede, não o ficheiro inteiro.

## Irmãos

| Checkout | Quando |
|----------|--------|
| `.` (Vael) | Mundo, scripts, quests, cenas, pool `assets/` |
| `../Viber` | Bug ou API em falta na engine. `cargo test` lá. `viber run` a partir desse checkout reconstrói |
| `../Viber/editor` | `cargo run --manifest-path ../Viber/editor/Cargo.toml -- .` |
| `../AiGameKit` | Gerar ou corrigir uma tool. Não abras o monorepo no workspace por defeito |

Workspace do dia-a-dia: `Vael.code-workspace` (esta pasta + `../Viber`).

## Só estático

`world.xml` (`<world static="true">`) + `world/**` + `world.data/` são o mundo.
As poses do XML são finais. O runtime não assenta props nem corre
`<StaticSpawner>` / `place` (erro de parse). Única geração em runtime:
`<DynamicSpawner>` (criaturas).

Editar com `viber scene` / `viber tool` / `viber terrain` (offline ou `--live`)
ou o editor. Hand-editar uma pose à espera que a engine a ajuste não funciona.
Depois de pincéis: `viber tool settle`. Árvores e dressing:
`viber tool scatter`, não um re-bake. Vegetação `.vinst` reescreve-se com
`viber tool scatter` e não entra no undo — git ou re-scatter.

`world.data/terrain.ahgt` e `.paint` são estado do mundo. Commita-os.

## Mapa

| Caminho | O que é |
|---------|---------|
| `world.xml` | raiz estática e os `<Include src="/world/…">` |
| `world/**.xml` | cidades, criaturas, landmarks, `spawn/…`, HUD |
| `scenes/interiors/` | salas (cenas próprias, sem terreno) |
| `quests/*.json` | 27 quests, lidas do disco (reiniciar a engine; sem `cargo build`) |
| `scripts/**.lua` | comportamento. Hot-reload ao gravar |
| `ui/*.css` | HUD e menu |
| `assets/` | pool. `game.yaml` + `manifests/`. Binários em `assets/public/` |
| `config.yaml` | `assets.roots: assets/public` |
| `qa-*.xml` | mundos QA. Incluem `world/**` — editar o módulo muda o mundo |

## Cenas

Cada sala é `scenes/interiors/<sala>.xml` com `<SceneEntry name="from-town">`
e `<ScenePortal name="exit" target="main" entry="@back" auto-radius="1.6">`.
Portas da vila: `world/cities/discordia/portals.xml`. Jogador, câmara e HUD
são autoloads (`world/autoload/*.xml`). Sala nova:
`viber scene new-scene interiors/<nome> --interior "20 16"` mais um portal
na vila. `viber scene lint` valida alvos.

## Gotchas

- Acrescentar filhos a `<Entity … />` cria um irmão na origem. Abre a tag.
  `viber scene insert` não tem este problema. O `analyze` não apanha.
- `shaders/*.wesl` são reescritos em cada `run`. Não editar à mão.
- Depois de editar: `viber analyze` (exit 1 em erro).

## Correr

```bash
cd ../Viber && cargo run -- run ../Vael/world.xml
cd ../Viber && cargo run -- analyze ../Vael/world.xml
viber run          # com o cwd nesta pasta e o binário no PATH
```

QA vivo: skill `viber-engine` (sessão `viber session`, uma engine de cada vez).
