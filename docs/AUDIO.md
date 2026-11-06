# Áudio do Vale — direção e pipeline

Identidade: **Coro de Vael**. O Vale soa a um coro sem palavras que carrega um
motivo de quatro notas; os cristais respondem com harmónica de vidro e os
instrumentos de madeira dizem onde estás. O motivo parte-se no combate e
resolve-se nos créditos.

## Paleta

| Elemento | Uso |
|----------|-----|
| Coro misto sem palavras | Fio condutor; tema, biomas, combate, créditos |
| Harmónica de vidro / sinos de cristal | As Notas; marcos, santuários, picos |
| Cordas friccionadas (viola da gamba, cello) | Base harmónica; tensão no pântano/masmorra |
| Madeiras graves (fagote, flauta baixa) | Sombra; floresta, pântano, deserto |
| Harpa, alaúde, viola | Vila, menu, créditos |
| Tambor de aro / percussão de mão | Pulso; deserto, combate, chefe |

**Motivo de quatro notas** (descrito nos prompts, não em partitura): quarta
ascendente, segundo grau, terça menor descendente. Repete-se em todos os temas
— inteiro na vila/explorar, invertido no combate, esticado no chefe, resolvido
nos créditos.

## Tempos e durações

Os BGM são loops exatos, alinhados a compassos. Duração = múltiplo de 4/4 ao
BPM do tema.

| Layer | BPM | Duração | Compassos |
|-------|-----|---------|-----------|
| menu | 80 | 48 s | 16 |
| explore | 80 | 60 s | 20 |
| village | 100 | 48 s | 20 |
| dark-forest | 60 | 48 s | 12 |
| desert | 80 | 48 s | 16 |
| swamp | 60 | 48 s | 12 |
| frozen-peaks | 60 | 56 s | 14 |
| dungeon | 60 | 48 s | 12 |
| battle | 120 | 48 s | 24 |
| boss | 90 | 48 s | 18 |
| credits | 72 | 60 s | 18 |

`bgm/race` é legado do exemplo de corrida; fica no pool mas não faz parte da
direção do Vale e não é regenerado por omissão.

## Zonas

A zona de música segue o jogador: cena (menu/interior) > combate > **bioma**
(`BiomeRegion.id` com uma `<MusicLayer>` homónima) > `bgm_zone` (vila,
masmorra, picos, explorar). As layers de bioma são `dark-forest`, `desert`,
`swamp` e `frozen-peaks`; a vila continua a ganhar por estar no vão dos
polígonos.

## Ambiências

Loops contínuos, posicionais quando fazem sentido:

| Clip | Onde | Como |
|------|------|------|
| `world/water_lake`, `water_flow`, `water_waterfall` | água | engine (distância) |
| `ambient/rain_loop` | chuva | engine (weather) |
| `ambient/fire_loop` | fogueira | `campfire.lua` via `viber.sound_loop` |
| `ambient/forest_loop` | dark-forest | `game/ambience.lua` |
| `ambient/swamp_loop` | swamp | `game/ambience.lua` |
| `ambient/peaks_loop` | frozen-peaks | `game/ambience.lua` |
| `ambient/desert_loop` | desert | `game/ambience.lua` |
| `ambient/village_loop` | vila | `game/ambience.lua` |

Os loops de bioma entram pelo bus `ambience` (`bus="ambience"`), não pelo SFX.

## Níveis

- BGM: `mastering` do tier `highest` (−14 LUFS), `base-volume` 0.16–0.24 por
  layer. O volume final fica ≈ −30 LUFS com o mixer a 0.7.
- SFX one-shot: `--lufs -16 --true-peak -1.0` para não somar picos em combate.
- Ambiências: tier `highest`, −16 LUFS; o volume é conduzido pela distância no
  caso da água e por bioma nos loops do script.
- `AudioMixer`: master 1, music 0.7, sfx 0.8, voice 0.9, ambience 0.7.

## Regerar

O pool canónico é `assets/public/assets/audio`; os manifests em
`assets/manifests/audio-*.yaml` são a fonte de verdade.

```bash
cd assets
python3 regen_audio.py --only bgm                 # BGM que falta
python3 regen_audio.py --only bgm --force         # regenera tudo
python3 regen_audio.py --only sfx --force
python3 regen_audio.py --only sfx/ambient --force
```

O script usa o `text2sound` do venv em `../AiGameKit/Text2Sound`, tier
`highest` (24 steps), `--no-enhance` (prompts são autorais), OGG 0.85 e o gate
de costura (música: cauda ≥70% do corpo; ambiência: ≥30%, porque o outro do
SA3 é mais curto e a cauda de água/chuva é naturalmente esparsa). Em ambos o
início tem de ficar entre 50% e 250% do corpo: abaixo soa a intro apagada,
acima o "blast" repete a cada loop. Re-rola a seed +1000 até 6 tentativas em
BGM e 5 em ambiência. Para efeitos one-shot usa `-16 LUFS`/`-1 dBTP`; música e
ambiência ficam no tier. Entradas `legacy: true` (corrida) ficam fora do lote.

As layers novas exigem `world/environment.xml` com `<MusicLayer>` homónima do
`BiomeRegion.id`. Os loops de bioma correm em `scripts/game/ambience.lua`.

Verificação: `viber analyze world.xml` e, com a engine viva,
`viber debug prof --tab audio` para confirmar quantos loops estão a tocar.
