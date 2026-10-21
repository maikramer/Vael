local rules = viber.load("lib/rules.lua")
-- npc/forest-wolves.lua: diálogo 100% Lua de Hald, o do Gado (dador da
-- quest `forest_wolves`). Prova do desacoplamento:
--   * `viber.own_system("dialogue")` — a engine CEDE o [E] deste NPC (o
--     fluxo nativo de quests cala; o prompt/ação continuam arbitrados pelo
--     mesmo InteractionFocus);
--   * `viber.say(...)` escreve no BALÃO nativo do HUD;
--   * `rules.quest_def(id)` lê o MESMO JSON autoral (`quests/*.json`) — o
--     script não duplica títulos nem linhas;
--   * aceitar/entregar usa os comandos `quest_accept`/`quest_turn_in`
--     (estado, recompensas e tracker continuam da engine).
--
-- GOTCHA: o top-level corre ANTES do primeiro snapshot do frame — as defs
-- (e todo o ctx) só estão disponíveis dentro do `on_update`. Por isso o
-- `quest_def` é buscado laziamente.

local QUEST = "forest_wolves"
local def = nil

local function say_lines(lines, fallback, remaining)
  local text = table.concat(lines or {}, "\n")
  if text == "" then
    text = fallback
  end
  text = text:gsub("{remaining}", tostring(remaining or 0))
  viber.say(text)
end

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE
  if not st.ready then
    st.ready = true
    viber.set_interaction("Falar com Hald", "e", 3.5)
  end
  if def == nil then
    def = rules.quest_def(QUEST)
  end
  if viber.menu_open() or not viber.interacted("e") then
    return
  end
  viber.face_player()
  viber.gesture("talk")
  local state = rules.quest_state(QUEST)
  local have, need = rules.quest_progress(QUEST)
  local remaining = math.max(0, (need or 1) - (have or 0))
  if state == "not_taken" then
    say_lines(def and def.lines_intro, "“Lobos ao norte. Paga-se bem por presas.”", remaining)
    rules.quest_accept(QUEST)
  elseif state == "active" then
    say_lines(def and def.lines_progress, "“Os lobos ainda andam por aí, forasteiro.”", remaining)
  elseif state == "ready" then
    if rules.quest_turn_in(QUEST) then
      say_lines(def and def.lines_complete, "“Bom trabalho. O gado respira.”", 0)
    else
      say_lines(def and def.lines_progress, "“Os lobos ainda andam por aí, forasteiro.”", remaining)
    end
  else
    say_lines(def and def.lines_complete, "“Já cumpriste. O rebanho agradece.”")
  end
end
