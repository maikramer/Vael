---
name: viber-editor
description: Usar para abrir ou alterar o editor do Viber sobre o mundo do Vael. Editor Godot-style, paridade com viber scene, Play, inspector. ViberEditor, vibereditor.
---

# Editor

O editor é o crate `editor` dentro de `../Viber`. Liga a engine como
biblioteca (`viber = { path = ".." }`).

```bash
cargo run --manifest-path ../Viber/editor/Cargo.toml -- /caminho/do/Vael
```

Sem caminho, abre `../Vael/world.xml` se o checkout estiver ao lado.

Play (F5) grava e corre `viber run --bridge` noutro processo. As edições
são as ops de `viber scene` / `viber tool` / `viber terrain` — o mesmo
undo, na CLI, no bridge e no inspector. Contrato:
`../Viber/AGENTS.md`, secção "Mundos estáticos + edição por agentes".

Se o editor não consegue mostrar ou gravar uma coisa que a CLI já faz,
o bug é em `../Viber/editor`. Se a CLI também falha, o bug é na engine
(`../Viber/src/scene`).
