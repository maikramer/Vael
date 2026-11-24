# O Silêncio de Vael — bíblia narrativa do simple-rpg

> Capítulo I · A Nota. O fio que liga as 37 missões, os quatro chefes, os marcos da
> Nota e as cinemáticas. Quem escreve conteúdo novo (falas, missões, cenas) lê isto
> primeiro: nomes, tom e cronologia vivem aqui.

A continuação do Capítulo I, o segredo da coluna, a sequência M01–M14 e os desfechos estão no [enredo da campanha principal](docs/CAMPANHA_PRINCIPAL.md). O [GDD de O Silêncio de Vael](GDD.md) orienta o desenho de jogo e a produção.

## O mundo

**O Vale** é uma bacia verde rodeada por quatro terras que já foram mansas. No meio
fica o chão onde foi **Discórdia** — vila de cerca de madeira, praça de pedra, forja,
capela e um **salão comunal** onde o conselho se sentou desde antes de haver mapas.
O nome é velho e irónico: foi aqui que as quatro terras assinaram a paz. Na primeira
manhã do jogo esse chão chama-se **Concórdia**: as mesmas paredes, em estacas. Os
velhos ainda dizem Discórdia quando falam da noite em que caiu. O mapa diz Concórdia
enquanto a obra não fecha.

No extremo nordeste, acima do Ermo, ergue-se o **Alto de Vael**. Lá em cima
crescem veios de cristal azul que *cantam* quando o vento passa — o **Coro**.
Enquanto o Coro cantou, a bruma ficou nos pântanos, os bichos nos covis e as
estradas abertas. A **Cidadela de Vael**, a sudoeste, guardava o caminho do Coro
e as suas cantoras.

## O que aconteceu (há trinta invernos)

A Cidadela caiu numa só noite. Na mesma noite **Discórdia foi abaixo**: o telhado
do salão abriu, a forja apagou-se, o poço ficou cego. Uma coluna subiu do Vale para
defender a Cidadela e **nunca chegou** — o que resta dela é o **Campo de Batalha**,
onde hoje andam **sombras com os rostos dos soldados**. Quando a última cantora
morreu, o canto do Coro **partiu-se em quatro Notas**. Os sobreviventes voltaram ao
chão partido e riscaram outro nome na primeira tábua: Concórdia, a obra de a levantar
outra vez. Cada Nota foi engolida por uma criatura, e cada
criatura cresceu à medida do pedaço de canto que guardava:

| Nota | Guardião | Terra | Sinal da ferida |
|------|----------|-------|-----------------|
| Nota do Bosque | **A Bruxa do Bosque** (canta numa clareira onde nem os lobos entram) | Floresta Sombria (norte) | sombras saem dos pinhais ao anoitecer |
| Nota do Ermo | **O Verme do Ermo** (a areia treme à volta dele) | Ermo Rubro (este) | as caravanas desaparecem |
| Nota do Lodo | **O Guardião do Lodo** — "o que conta os afogados" | Pântano da Bruma (oeste) | o lago leva barcos, redes e gente |
| Nota dos Picos | **O Ogro dos Picos** (só desce quando a serra fica em silêncio) | Picos Gelados (sul) | a estrada do sul fechou |

Desde então o Coro está calado. É o **Silêncio de Vael**.

## Quem és

**O Cartógrafo.** Chegas a Concórdia com uma caixa de mapas e um contrato do
conselho: medir a terra outra vez, e o que ainda se pode levantar. A ferramenta é
**a Nota** — o caderno onde se *assinam* os marcos (o sistema de viagem: [F] assina,
[G] viaja). O **Ancião Edmar**, debaixo da lona onde o salão há-de voltar a ter
tecto, é o último que ouviu o Coro cantar. Percebe logo o que o conselho não disse:
quem mede a terra pode encontrar as quatro Notas. A casa, o poço, o portão e o
celeiro levantam-se com madeira e pedra. A forja, o mercado, a atalaia, o salão,
a capela, o estaleiro do lago e o acesso das carroças esperam também por uma peça que um
chefe ainda tem. A ficha de cada canteiro está no [GDD](GDD.md#concórdia--os-canteiros).

## Elenco

- **Ancião Edmar** — lona junto ao poço, em Concórdia, até o salão ter tecto.
  Narrador do prólogo e do epílogo; dá a missão-mãe **As Quatro Notas**. Fala
  devagar, em imagens. PT-PT, "tu".
- **Bram, o ferreiro** — tabuleiro de compra e venda desde a primeira manhã; a
  forja só trabalha quando a bigorna volta do covil do lobo. Prático, sardónico
  ("ferro bom não se negocia barato").
- **Irmã Vessa, a vigia** — orla da floresta; conhece a Bruxa.
- **Kadi, guia de caravanas** — perdeu três caravanas para o Verme.
- **Onno, pescador do lago** — o lago levou-lhe o irmão.
- **Batedora Sig** — guarda a estrada do sul; sabe acordar o Ogro.
- **Veterano do Campo** — sobrevivente da coluna; narra a cena do Campo de Batalha.
- **Mestre da Romaria** — guia a romaria ao Alto de Vael.
- **Mira, a Batedora** · **Capataz do Posto** · os cartógrafos de cada terra
  (Casla, Vela, A Que Conta os Barcos, O Homem dos Mojões).

## Arco do Capítulo I

1. **Prólogo** (cinemática `prologo`, pranchas, na primeira entrada no mundo):
   Discórdia de pé; a noite em que o salão abre, a forja apaga e o poço cega; a
   alva sobre as estacas e a palavra Concórdia. O Ancião, na última legenda:
   «Mede o que ainda se pode levantar.» O controlo começa na lona, não num salão
   inteiro.
2. **As Quatro Notas** (missão `story_notas`, Ancião Edmar): recuperar as quatro
   Notas (item `nota`, 0/4 no diário). Cada terra tem o seu guardião, o seu
   dador de missão e os seus marcos da Nota.
3. **O guardião desperta** (cinemáticas `guardiao-*`, a primeira vez que cada
   chefe vê o herói): nome, terra e o conselho de quem conhece o bicho — a cena
   ENSINA a mecânica do especial.
4. **Nota recuperada** (cinemática `nota`, ao derrotar um guardião): o canto volta
   um pouco; o Ancião sente-o do Vale.
5. **O Campo de Batalha** (cinemática `campo-de-batalha`, ao chegar ao campo): o
   Veterano conta a noite da Cidadela — a razão das sombras.
6. **Epílogo** (cinemática `epilogo`, ao entregar as quatro Notas): o Coro canta
   outra vez. *Fim do Capítulo I* — o mundo continua aberto.

## Tom e regras de escrita

- Português europeu, "tu", frases curtas. Nada de "você" nas falas novas.
- O mundo é bonito e cansado, não sombrio: há humor nos vilões menores (bandidos,
  goblins) e luto nos maiores.
- Cada cena ensina ou recompensa: a dos guardiões dá a dica de combate; a das
  Notas dá progresso visível (n/4).
- Legendas ≤ 2 linhas, ~4 s cada. Títulos curtos.

## Onde vive cada peça

| Peça | Ficheiro |
|------|----------|
| Cinemáticas (`.bsn`) | `world/cinematics.bsn` — painel **Vael: Cinemáticas** no Jackdaw |
| Controlador da história (Notas, prólogo, epílogo) | `scripts/game/story.lua` |
| Gatilhos de área ("só uma vez") | componente `CinematicTrigger` na entidade (`src/cinematic_trigger.rs`) |
| Cena de despertar dos chefes | `lib/fsm.lua` (`intro = "guardiao-…"`) |
| Missão-mãe | `quests/story_quests.json` |
