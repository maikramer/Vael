---
name: vael
description: Usar ao editar o jogo Vael — world.bsn estático (Jackdaw), cenas de interiores, quests JSON, scripts Luau, pool de assets em assets/. Mundo Vael, quests, cenas, HUD.
---

# Vael

Jogo em cima do runtime próprio `crates/vael_engine` (gameplay) e dos
crates `bevy_ext_*` da engine em `../bevy_extensions`.
Regras de edição: `AGENTS.md` desta pasta.

- Mundo estático em `world.bsn` + `world/**/*.bsn`. Poses finais.
  `<StaticSpawner>` e `place` são erro de parse.
- Editar o mundo/cenas no **Jackdaw oficial** (`jd open .`; cada nó é um
  `WorldNode`). O conteúdo (quests/itens/marcos) vem das extensões `.jdext`
  do Vael em `../bevy_extensions/extensions/` e `extensions/`.
- A CLI XML antiga foi aposentada; `vael-tools`
  cobre analyze/lint/bsn/run/debug.
- Quests em `quests/*.json`. Scripts em `scripts/`. Interiores em
  `scenes/interiors/*.bsn` (áreas internas do scene manager).
- `/assets/…` resolve na pasta do jogo e depois em `assets/public`
  (`config.yaml`).
- GLB novo: skill `asset-gen`, output em `assets/public`.
- API em falta ou bug de runtime: skill `vael-tools`, editar
  `../bevy_extensions`.
