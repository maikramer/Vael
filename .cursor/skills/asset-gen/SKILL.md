---
name: asset-gen
description: Usar para gerar ou regenerar assets do Vael com as ferramentas AiGameKit — gameassets resume, text3d finish, texturas KTX2, vramd. Output só na pasta assets deste jogo.
---

# Geração de assets do Vael

As tools vivem em `../AiGameKit` e correm pelo CLI (venv já no PATH).
Não abras esse monorepo no workspace para gerar um prop. Se uma tool
falhar por bug de código, aí edita `../AiGameKit/<Tool>` e
`vramd respawn <backend>`.

Pool deste jogo: `assets/` (`game.yaml`, `manifests/`, binários em
`public/assets`). O `config.yaml` do jogo aponta `assets.roots` para
`assets/public`.

```bash
cd assets
gameassets resume --profile game.yaml --manifest manifests/village
text3d finish public/assets/meshes/<pack>/<nome>_lod0.glb
vramd status
```

`resume` é idempotente. `output_dir` dos manifests é `../public/assets`
(relativo à pasta do manifest). Veículos da corrida não estão aqui —
ficam em `../Viber/examples/racer-assets`.

Antes de um job GPU: `vramd status`. Se a fila está ocupada, espera ou
cancela o job. Não mates processos da GPU. VRAM livre abaixo do pico e
workers idle: `vramd zero`.

Receita de um asset novo: `../AiGameKit/docs/MANIFEST_AUTHORING.md`.
Happy path = `category` + `size_m`. KTX2/UASTC no finish. Meshopt ligado.
