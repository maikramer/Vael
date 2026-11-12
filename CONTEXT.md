# CONTEXT.md — Vael

O mundo que valida a engine de ponta a ponta: port do exemplo original
(motor TS), convertido uma única vez por um script de migração (já removido).

## Estado

**Jogável de ponta a ponta** — ecrã de título (Continuar/Novo Jogo/
Definições/Créditos/Sair), morte com Tentar de novo/Carregar/Título,
combate melee + skills + talentos, 37 quests com diálogo, economia (vault,
colheita, loja, hotbar), travel (A Nota, 12 marcos, viagem rápida), save em
slots 0–3 + autosave, mundo vivo (céu procedural, dia/noite, clima, BGM por
zona, partículas) e física (knockback, destrutíveis).
Os ~65 scripts Luau correm com "LOD de IA" (fora do `activation-radius`,
45 m por omissão, o `on_update` nem corre).

A autoria vive no **Jackdaw oficial** (`jd open .`) com as extensões `.jdext`
do Vael em `../bevy_extensions/extensions/`: mundo/cenas, quests, diálogos,
itens/loja, marcos e progressão. O mundo é `world.bsn` +
`world/**/*.bsn` (o `.xml` legado fica ao lado); as cenas são
`scenes/**/*.bsn`. Os dados editáveis estão em `quests/`, `dialogues/` e
`data/`. O `grip` das armas ficou fora do editor (ajusta-se no `.bsn`/
`equipment.lua`).

O `analyze` imprime o relatório de cobertura de tags — é o roteiro do que
falta à engine (hoje: nada — as 5 tags `EngineConfig` já têm consumidor). Qualquer nova
feature da engine deve manter este mundo a 100 % de cobertura.

Detalhes completos (estrutura, assets, comandos): [`README.md`](README.md).
