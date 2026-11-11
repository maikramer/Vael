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

### Refine (Hi3D)

O bloco `refine:` do `game.yaml` corre o Hi3D entre o topology-fix e o paint,
com uma imagem por passe. Está em **mono-vista** (`views: 1`).

**Multiview por Text2D está desligado** (2026-11-11): o FLUX.3 Klein é um
modelo de edição e copia a imagem de referência. As vistas `back/left/right`
saíam ~99% idênticas à frontal (diff médio 2/255; `watchtower_back.png` é a
mesma imagem da frente), por isso a cadeia não acrescentava informação e
custava 4-6 passes por asset. Para religar, as vistas têm de ser reais — render
do mesh nos ângulos pedidos ou um modelo view-conditioned — e depois
`refine: {views: 4}`; o sidecar em `_intermediate/<id>_refined.glb.multiview.json`
invalida painted/LOD/collision quando as vistas mudam. Personagens/criaturas
ficam sempre de fora (o Hi3D muda a pose).

### Paint / xatlas

O `_to_paint` segue a fórmula do `Shared/paint_budget.py` (~270k faces a 2048).
O `_refined` do Hi3D **não** pode ser simplificado directamente: tem arestas
não-manifold (4 faces por aresta) e o Decimate estilhaça-o (longhouse: 5.186
componentes/86k bordos a 150k). O xatlas é superlinear com as ilhas — 40k ~3 s,
150k >200 s, 265k >600 s — e o bake trust cai a ~29% (mosaico em jogo). O
pipeline refixa o `_refined` **antes** do simplify com
`text3d topology-fix --morph-close <0,5 mm/m> --keep-internal-shells`; a 265k o
unwrap fica em ~160 s e o atlas em ilhas grandes (medido a 2026-11-11). Não
baixar o orçamento para 150k: menos faces por ilha = mosaico.

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
