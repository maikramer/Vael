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

## Zonas

O menu usa o tema do título. No mundo aberto, a perseguição de uma criatura
viva acende `battle`, e a de um chefe acende `boss`. O hold de oito segundos
evita cortes entre golpes. Ao sair do combate, a música regressa à vila, à
exploração ou à faixa homónima do `BiomeRegion.id`. O crossfade nominal dura
1,8 s. Os quatro biomas musicais são `dark-forest`, `desert`, `swamp` e
`frozen-peaks`. Os interiores têm acústica de sala.

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

A fauna usa o bus de ambiente. O gaio chama de dia e a coruja à noite, a
partir de um poleiro. A raposa dá um latido curto ao começar a fugir. A
distância e o lado da câmara determinam o ganho e a posição estéreo até 32 m.
Um intervalo mínimo de 12 s impede que um grupo de raposas sobreponha latidos.

## Níveis

Os quatro subgrupos Kira têm controlo de ganho próprio. O ganho de cada
instância continua a seguir a distância ou a transição musical quando o
jogador muda um slider ou quando um diálogo atenua o som. Os valores iniciais
de master, música, efeitos, voz e ambiente são 1, 0.7, 0.8, 0.9 e 0.7.

As 11 músicas têm ganhos autorais entre 0.135 e 0.22. A vila e os biomas
foram equilibrados a partir do loudness dos OGG. O lago, a floresta e a
fogueira usam o modelo SFX do Text2Sound em loops de 16 s. Os pássaros ficam
nos emissores da fauna. A moeda tem um passa-baixo de quatro polos a 3,8 kHz
e compensação de 9 dB. Passos na grama, colheita, impacto metálico, queda de
objeto, baú e confirmação de carregamento também têm filtros próprios nos manifestos.

## Engasgos durante a reprodução

O Vael abre uma saída Kira para o dispositivo. Em Linux com ALSA sobre
PipeWire, o executável pede um período mínimo de 16 384 bytes, cerca de 43 ms
em estéreo float a 48 kHz. O pedido aplica-se ao processo do jogo e respeita
uma configuração explícita em `PIPEWIRE_ALSA`. O período introduz alguma
latência e dá margem à thread de áudio quando o render ou o sistema a atrasam.

Para verificar um engasgo no menu, comparar a captura da saída do processo
com `bgm/menu.ogg` por pelo menos uma volta de 48 s. Em `pw-top`, acompanhar
a **variação** de `ERR` na saída do jogo e no dispositivo durante a captura.
Um bloco de zeros presente só na captura também denuncia uma interrupção,
mesmo quando o contador do nó do jogo fica estável.

No teste do menu a 48 kHz, introduzir 32 ms de atraso no produtor de áudio
a cada dois segundos produziu 12 lacunas de 21,3 ms em 25 s com o período
original de 512 frames. Com 2048 frames, a captura de 55 s incluiu uma
volta completa e 27 atrasos induzidos. O sinal manteve-se alinhado com o
OGG, sem lacunas de silêncio digital acima de 1 ms.

## Regerar e verificar

O pool canónico é `assets/public/assets/audio`. Cada entrada em
`assets/manifests/audio-*.yaml` tem prompt, perfil e seed. O JSON ao lado do
OGG regista o modelo e a seed aceite. O pack dos cinco veículos de corrida
tem `legacy: true` e fica fora da regeneração habitual.

```bash
cd assets
python3 audit_audio.py
python3 regen_audio.py --only fox_bark,jay_call,owl_hoot --force
python3 regen_audio.py --only water_lake,forest_loop,fire_loop --force
python3 regen_audio.py --only bgm
```

A auditoria corre em CPU com NumPy, SoundFile, PyYAML e FFmpeg. Verifica a
decodificação, a duração dos loops, a proveniência, os picos reais e sinais quase mudos (pico abaixo de −55 dBTP). A coluna
`>7,5kHz` mostra a percentagem de energia nos agudos para comparar timbres.

A regeneração usa o venv do Text2Sound no AiGameKit e envia um pedido de cada
vez com prioridade `batch` ao vramd. Usa 24 passos, os prompts do manifesto e
Vorbis com qualidade 0.85. O alvo de geração dos novos ambientes é −19 LUFS,
e o dos one-shots é −16 LUFS. A auditoria mede o resultado depois da codificação.

Antes de substituir um loop, o gerador compara o RMS dos primeiros e últimos
500 ms com o corpo da faixa. A música aceita início entre 60% e 250% e cauda
a partir de 70%. O ambiente aceita início entre 50% e 250% e cauda a partir
de 30%. Uma tentativa rejeitada mantém o OGG anterior em jogo.

Com o Vael aberto, `vael-tools debug --port <porta> lua 'return viber.debug.audio()'`
mostra os ganhos por faixa e os loops pedidos pelos scripts. As faixas com
volume zero estão preparadas para o próximo crossfade. Para verificar o mute
e os timbres, a captura de QA deve vir da saída do processo do Vael.
