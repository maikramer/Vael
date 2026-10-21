# Shared asset pool

**Uma** pasta com todos os assets gerados do repo — manifests canónicos e os
binários finais (GLB/PNG/JSON). Nenhum exemplo guarda cópias nem symlinks: os exemplos servem `public/assets/`
daqui por URL, e a engine nativa Viber consome-o como root extra declarado no
`config.yaml` de cada jogo (`Viber/docs/ASSETS.md`).

## Packs

`audio-bgm` · `audio-sfx-combat` · `audio-sfx-creatures` · `audio-sfx-ui` ·
`audio-sfx-vehicles` · `audio-sfx-world` · `characters` · `desert` · `farm` ·
`forest` · `infra` · `interiors` · `props` · `swamp` · `terrain` ·
`vegetation` · `vehicles` · `village`

### Áudio (BGM + SFX)

Os clips vivem em `public/assets/audio/` com o rid do manifest como caminho:

```
audio/bgm/*.ogg              # loops seamless (SA3 Music; -d exacto, @120 BPM)
audio/sfx/combat/*.ogg       # hits/swings/colheita/arco/escudo
audio/sfx/creatures/*.ogg    # roars/ferimentos/mortes/slime/lobo
audio/sfx/player/*.ogg       # hurt/heal do herói
audio/sfx/ui/*.ogg           # moedas/menus/save/notificações/eventos de corrida
audio/sfx/vehicles/*.ogg     # motor/nitro/skid/crash/buzina
audio/sfx/world/*.ogg        # portas/baús/fogo/passos/itens
```

Regen curado (gate de costura para BGM — re-rola seeds até cauda ≥70%):

```bash
cd Text2Sound && .venv/bin/python regen_audio.py          # só o que falta
.venv/bin/python regen_audio.py --force                   # tudo
.venv/bin/python regen_audio.py --only bgm/boss           # itens específicos
```

Ou pelo caminho GameAssets (sem gate): `gameassets resume --profile
game.yaml --manifest manifests/audio-bgm` (um por pack). Os manifests são a
fonte única de verdade (prompt, categoria, duração, seed); os sidecars
`.ogg.json` ao lado de cada clip trazem a proveniência da geração.

## Happy path de regen (GPU)

O batch corre a partir **desta** pasta e escreve em `public/assets/` daqui:

```bash
cd assets
gameassets resume --profile game.yaml --manifest manifests/village
gameassets resume --profile game.yaml --manifest manifests/characters
# … um por pack; `ls manifests/` lista todos
```

`resume` é idempotente: detecta por item o estado em disco (PNG/shape/paint) e
só gera o que falta — com os binários já presentes, o batch valida e faz skip.

> `output_dir` de cada manifest resolve-se contra a pasta **do manifest**, não
> contra a do `game.yaml`. Como os manifests vivem em `manifests/`, o valor
> correto é `../public/assets`; com `public/assets` o resume aponta para
> `manifests/public/assets`, não encontra nada e marca tudo como `need_image`.

## Como o Vael consome isto

O `config.yaml` do jogo declara `assets.roots: assets/public`. A pasta do
jogo responde primeiro (overrides); o que faltar vem deste pool. URLs no XML
ficam `/assets/…`. Detalhe: `../Viber/docs/ASSETS.md`.

Binários versionados (texturas, ícones, BGM, heightmaps, GLBs finais) entram
por Git LFS. `_intermediate/`, `images/` e `sky/` ficam locais — a receita
está nos manifests.

## O que não vive aqui

O mundo (`world.xml`, `world/**`, `world.data/`, `scripts/`, `quests/`) fica
na raiz do Vael, não neste pool. Ícones, texturas, áudio e meshes do jogo
vivem em `public/assets/`. Um override por-mundo é um ficheiro com o mesmo
caminho na raiz do jogo (a 1.ª asset root).
