-- Diálogo genérico de um dador. O id da quest vem de prop-dialogue_id.
-- Aceita, mostra progresso e entrega. Sem a quest nativa da engine.
--@export dialogue_id: string = ""

local rules = viber.load("lib/rules.lua")
local marks = viber.load("lib/landmarks.lua")

local function quest_id()
  return viber.prop("dialogue_id")
end

-- Quests de chefe: se o guardião já caiu antes de aceitar, a entidade foi
-- despawnada e a quest ficava impossível. Ao aceitar, credita o abate.
local BOSS_QUEST = {
  forest_witch = "witch",
  desert_worm = "sand-worm",
  swamp_bogwarden = "bog-warden",
  peaks_ogre = "boss",
}

local function say(lines, fallback, remaining)
  local text = table.concat(lines or {}, "\n")
  if text == "" then
    text = fallback
  end
  text = text:gsub("{remaining}", tostring(remaining or 0))
  viber.say(text)
  -- Balão aberto: 1/2/V são da conversa, não da hotbar/equipamento.
  viber.ui.set("dialogue.until", viber.time() + 0.75)
end

function on_update(_dt)
  local id = quest_id()
  if not id or id == "" then
    return
  end
  local st = viber.state()
  local def = rules.quest_def(id)
  local title = def and def.title or id
  local state = rules.quest_state(id)
  local label = state == "ready" and (title .. " · pronta") or title
  if st.label ~= label then
    st.label = label
    viber.set_interaction(label, "e", 3.5)
  end
  if viber.menu_open() or not viber.interacted("e") then
    return
  end
  viber.face_player()
  viber.gesture("talk")
  local have, need = 0, 1
  if rules.quest_progress then
    have, need = rules.quest_progress(id)
  end
  -- Coletáveis: o progresso gravado fica stale se o jogador vender o item;
  -- o que conta é o cofre AGORA.
  local obj = def and def.objective
  if obj and obj.type == "collect" then
    have = rules.vault_get(obj.target or "")
  end
  local remaining = math.max(0, (need or 1) - (have or 0))
  if state == "not_taken" then
    rules.quest_accept(id)
    local kind = BOSS_QUEST[id]
    if kind and viber.game()["nota_" .. kind] then
      for _ = 1, math.max(1, need or 1) do
        rules.report_kill(kind)
      end
    end
    -- Marcos já MEDIDOS ([F]) antes de aceitar: `report_visit` só conta
    -- quests ativas, por isso a visita perdia-se e o jogador tinha de
    -- refazer o caminho. Credita-os agora (mesmo tratamento do chefe).
    if obj and obj.type == "visit" then
      for token in tostring(obj.target or ""):gmatch("%S+") do
        if marks.is_marked(token) then
          rules.report_visit(token)
        end
      end
    end
    say(def and def.lines_intro, "Aceite.", remaining)
  elseif state == "ready" then
    if rules.quest_turn_in(id) then
      say(def and def.lines_complete, "Entregue.", 0)
    else
      say(def and def.lines_progress, "Ainda não tens o que a missão pede.", remaining)
    end
  elseif state == "active" then
    say(def and def.lines_progress, "Ainda não.", remaining)
  else
    viber.say("Já tratámos disso.")
  end
end
