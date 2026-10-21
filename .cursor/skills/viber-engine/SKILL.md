---
name: viber-engine
description: Usar quando a tarefa precisa da engine Viber — API Luau em falta, bug de runtime, terreno, física, cenas, bridge de QA, ou cargo test. Editar o checkout irmão ../Viber. Viber engine, Bevy, viber run, viber scene.
---

# Engine Viber

Checkout: `../Viber`. O editor é `../Viber/editor`. A lei é
`../Viber/AGENTS.md` e `../Viber/docs/` (`LUA_API.md`, `ASSETS.md`, `BRIDGE.md`,
`UI.md`). Lê a secção da tarefa. Não copies o AGENTS inteiro para o jogo.

## Quando editar a engine

Falta uma primitiva que o Lua não alcança, o `analyze` mente, ou o runtime
não faz o que o XML pede. O conteúdo do Vael (falas, poses, quests) fica neste
repo.

```bash
cd ../Viber && cargo test
cd ../Viber && cargo run -- run ../Vael/world.xml
cd ../Viber && cargo run -- analyze ../Vael/world.xml
```

`viber run` dentro do checkout reconstrói a engine. Mundos estáticos só.
`viber scene` / `viber tool` / `viber terrain` são a mesma API do editor.

## QA vivo

Uma engine por mundo. Não mates o processo.

```bash
viber session status
viber session up
viber session claim --owner <tarefa>    # exit 3 = ocupado, não insistas
viber debug --world <mundo> screenshot -o shot.png
viber session release --owner <tarefa>
```

Doc: `../Viber/docs/BRIDGE.md`.
