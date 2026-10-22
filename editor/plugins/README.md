# Plugins do editor — Vael

O `vibereditor` carrega todos os `*.lua` desta pasta quando abre o projeto. O
editor não tem regras de jogo: tudo o que sabe o que é uma quest, um diálogo,
um item, uma loja ou um marco vive aqui, em plugins do Vael.

A referência completa da API (manifesto, `ctx`, widgets, eventos, FS, prefs,
hot-reload e segurança) está em
[`Viber/editor/PLUGINS.md`](../../../Viber/editor/PLUGINS.md).

## Plugins

| Ficheiro | Painel | O que faz |
|---|---|---|
| `grip_preview.lua` | Grip | Prende a arma/ferramenta ao osso do herói, edita offsets/escala e aplica as props ao nó (`groups="grip-preview"`). |
| `quests.lua` | Missões | Edita `quests/*.json`; cria/apaga quests, valida NPC/local/item e mantém o `quests/index.json`. |
| `dialogues.lua` | Diálogos | Edita `dialogues/*.json`; nós, escolhas, `goto`, `start`, locale e flags de quest. |
| `content_items.lua` | Itens | Edita `data/items.json` (labels, pilhas e preços `shop.buy/sell`). |
| `content_landmarks.lua` | Marcos | Edita `data/landmarks.json`; valida entidades do mundo, biomas e surveys. |
| `content_progression.lua` | Progressão | Edita `data/progression.json` (XP, abates, talentos e efeitos). |
| `content_lint.lua` | Conteúdo | Validação cruzada de quests ↔ diálogos ↔ itens ↔ marcos ↔ locale; publica no Problems. |
| `content_nodes.lua` | — | Templates do jogo no Add Node (ex.: `<ResourceNode>`). |

## Ciclo de trabalho

1. `viber editor world.xml` (ou `cargo run -p vibereditor -- ../Vael/world.xml`
   a partir do Viber).
2. Abre a aba **Plugins** no dock inferior e escolhe o painel.
3. Edita; **Gravar** escreve o JSON de conteúdo com `write_atomic` (fica um
   `.bak` ao lado). As edições de cena (`Grip › Aplicar ao nó`) entram no undo
   do editor.
4. Os scripts do jogo não correm no modo Edit. Reinicia o **Play** para o jogo
   ler o JSON novo (`rules.lua`/`shop.lua`/`landmarks.lua` carregam no arranque).
5. `Ctrl+K` lista os comandos dos plugins (ex.: “Quests: nova”,
   “Conteúdo: validar tudo”).

Os plugins recarregam sozinhos ~1 s depois de gravares o `.lua`; o estado de
sessão (seleção, drafts) perde-se, mas as prefs (`editor.prefs`) ficam.

## Dados de conteúdo

| Ficheiro | Consumidor |
|---|---|
| `quests/*.json` + `quests/index.json` | `scripts/lib/rules.lua` |
| `dialogues/*.json` | `scripts/townsfolk.lua` |
| `data/items.json` | `scripts/lib/rules.lua`, `scripts/game/shop.lua`, `scripts/ui/journal.lua` |
| `data/landmarks.json` | `scripts/lib/landmarks.lua`, `scripts/game/travel.lua` |
| `data/progression.json` | `scripts/lib/rules.lua`, `scripts/ui/journal.lua`, `scripts/poi/song-seed.lua` |

`quests/index.json` é a lista de ficheiros de quests. O plugin mantém-na ao
criar/apagar ficheiros; sem ela, `rules.lua` usa a lista de fallback.

## Exemplo mínimo

```lua
return {
  api = 1,
  id = "hello",
  title = "Hello",
  panels = {
    { id = "main", title = "Hello", render = function(ctx)
        return {
          type = "column", gap = 6, children = {
            { type = "heading", text = "Hello" },
            { type = "label", text = "Projeto: " .. ctx.project.name },
            { type = "button", id = "ping", text = "Ping" },
          },
        }
      end },
  },
  on_event = function(ctx, event)
    if event.kind == "click" and event.id == "ping" then
      ctx.editor.toast("success", "pong")
    end
  end,
}
```

## Smoke sem abrir janela

```bash
cd ../Viber
PLUGIN_SMOKE_DIR=../Vael/editor/plugins \
  cargo test -p vibereditor plugins::tests:: -- --nocapture
```

O teste carrega os manifestos, corre `lint`, `on_start`/`on_update` e `render`
de todos os plugins contra o `world.xml`, e carrega `rules.lua`/`landmarks.lua`
(mais loja e diário) com um stub de `viber.*`.
