# dialogues/ — grafos de diálogo nativos

Um ficheiro por `dialogue-id` (o mesmo do `<DialogueNPC>`). A engine lê esta
pasta ao arrancar; o `[E]` avança o nó e os dígitos `1`..`9` escolhem.

Sem ficheiro, o `<DialogueNPC>` cai no fluxo de quest nativo (linhas
`intro/progress/complete` de `quests/*.json`). Com ficheiro, o grafo manda —
por isso os NPCs de quest continuam no diálogo Lua (`scripts/npc/dialogue.lua`),
que é sensível ao estado da missão. Os grafos daqui são para NPCs ambientais
estacionários (lojas, capela, salões).

## Formato

```json
{
  "start": "start",
  "nodes": {
    "start": {
      "speaker": "Bera",
      "text": "Bem-vindo à casa longa!",
      "choices": [
        { "text": "Que histórias se contam?", "goto": "historias" },
        { "text": "Adeus.", "goto": "adeus" }
      ]
    }
  }
}
```

- `start` tem de existir em `nodes`.
- `speaker` vazio = o texto aparece sem prefixo.
- `voice`/`portrait` são opcionais (`voice` toca o clip de SFX indicado).
- `quest_accept` / `quest_turn_in` numa escolha aceitam/entregam a quest com o
  mesmo `id` do ficheiro. Usar só em NPCs cujo estado não se perde — um grafo
  não consulta o progresso antes de mostrar as escolhas.
- Cada escolha aponta para um nó existente. Um nó sem escolhas mostra o texto
  e espera pelo próximo `[E]`.

Os grafos dos NPCs ambientais (`chapel_priest`, `house_b_scholar`,
`longhouse_innkeeper`, `house_a_cook`, `house_a_innkeeper`, `house_c_cook`)
são ligados por `prop-dialogue_id` em `scripts/townsfolk.lua`: com esse prop,
o script deixa de registar `ScriptInteraction` e o `[E]` é do nativo.
