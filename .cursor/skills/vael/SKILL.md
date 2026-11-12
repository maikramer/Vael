---
name: vael
description: Usar ao editar o jogo Vael — world.bsn estático (Jackdaw), cenas de interiores, quests JSON, scripts Luau, pool de assets em assets/. Mundo Viber, quests, cenas, HUD.
---

# Vael

Jogo em cima da engine irmã `../Viber`. Regras de edição: `AGENTS.md` desta pasta.

- Mundo estático em `world.bsn` + `world/**/*.bsn` (o `.xml` legado fica ao
  lado). Poses finais. `<StaticSpawner>` e `place` são erro de parse.
- Editar o mundo/cenas no **Jackdaw oficial** (`jd open .`; cada nó é um
  `WorldNode`). O conteúdo (quests/itens/marcos) vem das extensões `.jdext`
  do Vael em `../bevy_extensions/extensions/`.
- `viber scene` / `viber tool settle|scatter` / `viber terrain` continuam
  XML-only: chegam aos `.xml` legados, não ao `.bsn`.
- Quests em `quests/*.json`. Scripts em `scripts/`. Interiores em
  `scenes/interiors/*.bsn` (áreas internas do scene manager do Viber).
- `/assets/…` resolve na pasta do jogo e depois em `assets/public`
  (`config.yaml`).
- GLB novo: skill `asset-gen`, output em `assets/public`.
- API em falta ou bug de runtime: skill `viber-engine`, editar `../Viber`.
