---
name: vael
description: Usar ao editar o jogo Vael — world.xml estático, cenas de interiores, quests JSON, scripts Luau, pool de assets em assets/. Mundo Viber, quests, cenas, HUD.
---

# Vael

Jogo em cima da engine irmã `../Viber`. Regras de edição: `AGENTS.md` desta pasta.

- Mundo estático. Poses finais. `<StaticSpawner>` e `place` são erro de parse.
- Editar com `viber scene` / `viber tool` / `viber terrain`, ou o editor
  (`cargo run --manifest-path ../Viber/editor/Cargo.toml -- <esta pasta>`).
- Quests em `quests/*.json`. Scripts em `scripts/`. Interiores em
  `scenes/interiors/`.
- `/assets/…` resolve na pasta do jogo e depois em `assets/public`
  (`config.yaml`).
- GLB novo: skill `asset-gen`, output em `assets/public`.
- API em falta ou bug de runtime: skill `viber-engine`, editar `../Viber`.
