-- Diálogo genérico de um dador. O id da quest vem de prop-dialogue_id.
-- Aceita, mostra progresso e entrega. Sem a quest nativa da engine.
--@export dialogue_id: string = ""

local rules = vael.load("lib/rules.lua")
local opening = vael.load("lib/opening.lua")
local marks = vael.load("lib/landmarks.lua")
local concordia = vael.load("lib/concordia.lua")
local talk = vael.load("ui/say.lua")
local decide = vael.load("lib/decide.lua")

-- O UiInput focado come as teclas: não disparar [E]/diálogo enquanto se escreve.
local function typing()
  return vael.ui and vael.ui.focused and vael.ui.focused() ~= nil
end

-- CADEIAS: uma quest entregue com `next` passa o dador para a seguinte
-- (`rules.quest_chain`). O prop continua a apontar a primeira.
local function quest_id()
  local id = vael.prop("dialogue_id")
  if id and id ~= "" and rules.quest_chain then
    return rules.quest_chain(id)
  end
  return id
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
  talk.say(text)
  -- Balão aberto: 1/2/V são da conversa, não da hotbar/equipamento.
  vael.ui.set("dialogue.until", vael.time() + 0.75)
end

function on_update(_dt)
  local id = quest_id()
  if not id or id == "" then
    return
  end
  local st = vael.state()
  local def = rules.quest_def(id)
  local title = def and def.title or id
  local state = rules.quest_state(id)
  local label = state == "ready" and (title .. " · pronta") or title
  local region
  local show_note = id == "forest_witch" and state == "ready" and vael.game()["campaign.opening"]
  if show_note then label = "Mostrar a Nota do Bosque a Vessa" end
  local hidden = state == "not_taken" and not rules.quest_available(id)
  if hidden then
    label = "Cumprimentar"
  elseif id == "story_notas" and state == "not_taken" then
    local dx, dz = opening.offset()
    if not dx or dx * dx + dz * dz > 25 then st.contract_open = false end
    if st.contract_open and concordia.open("house") then
      region, label = opening.region(dx, dz)
    else
      label = "Abrir o contrato dobrado"
    end
  end
  opening.prompt(st, label)
  if vael.menu_open() or talk.active() or typing() or not vael.interacted("e") then
    return
  end
  -- Herói caído: o modal de morte só abre ao fim dos 2,2 s de `dying`, e
  -- nessa janela o [E] ainda chega aqui. O dador não aceita/entrega missão
  -- (ouro, XP, itens) a um cadáver — o mesmo portão do quadro de avisos.
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return
  end
  vael.face_player()
  vael.gesture("talk")
  local have, need = 0, 1
  if rules.quest_progress then
    have, need = rules.quest_progress(id)
  end
  -- Coletáveis: o progresso gravado fica stale se o jogador vender o item;
  -- o que conta é o cofre AGORA.
  -- Etapa EM CURSO (quests com `stages`).
  local obj = def and rules.quest_objective(id)
  if obj and obj.type == "collect" then
    have = rules.vault_get(obj.target or "")
  end
  local remaining = math.max(0, (need or 1) - (have or 0))
  if hidden then
    if id == "forest_nightwatch" then
      talk.say("Dorme, meu filho, que o pinhal acorda.")
    else
      talk.say("Que a estrada te seja leve.")
    end
  elseif id == "story_notas" and state == "not_taken" then
    if not concordia.open("house") then
      talk.say("Mede o que ainda se pode levantar. A primeira obra é a tua casa: oito de madeira e seis de pedra.", 7)
    elseif not st.contract_open then
      st.contract_open = true
      say(def.lines_intro, "Escolhe uma terra.", 0)
    elseif region and vael.vael.opening_action("contract", region) then
      st.contract_open = false
      rules.sync_collect()
      talk.say("Contrato assinado. Levanta o portão da cerca. Depois ensinas à criança como voltar da Atalaia do Vale.", 7)
      vael.save_slot(0)
    end
  elseif id == "story_notas" and state == "ready" and not concordia.open("hall") then
    talk.say("Traz as Notas ao salão quando a trave estiver no sítio.")
  elseif id == "story_notas" and state == "ready" and vael.game()["campaign.opening"]
      and rules.quest_state("story_first_fold") ~= "done" then
    talk.say("Primeiro mostra à criança o caminho de volta.")
  elseif state == "not_taken" then
    if not rules.quest_accept(id) then return end
    if BOSS_QUEST[id] then vael.game().tracked_quest = id end
    -- Aceitar pode mudar a meta (etapas): relê o que falta para a fala.
    have, need = rules.quest_progress(id)
    remaining = math.max(0, (need or 1) - (have or 0))
    local kind = BOSS_QUEST[id]
    if kind and vael.game()["nota_" .. kind] then
      rules.quest_set_progress(id, need or 1)
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
  elseif show_note then
    if rules.quest_turn_in(id) then
      say(def.lines_complete, "O último verso é meu.")
      vael.save_slot(0)
    end
  elseif state == "ready" then
    local p = rules.quest_parley(id)
    local d1_ready = decide.ready()
    if p and p.report and d1_ready then
      talk.say(p.report.prompt or "Diz-me o que encontraste.")
      talk.ask({
        prompt = p.report.prompt,
        example = p.report.example,
        state = { npc = def and def.title or id, quest = id },
        questions = decide.expect_questions(p.report.expect, p.report.prompt),
        on_result = function(answers, text)
          local ok, msg = rules.quest_turn_in_decision(id, text, answers)
          if ok then
            say(def and def.lines_complete, "Entregue.", 0)
          else
            talk.say(msg or p.report.hint or "Não te ouvi.")
          end
        end,
        on_unavailable = function()
          if rules.quest_turn_in(id) then
            say(def and def.lines_complete, "Entregue.", 0)
          else
            say(def and def.lines_progress, "Ainda não tens o que a missão pede.", remaining)
          end
        end,
        on_cancel = function()
          talk.say("Quando quiseres contar, estou aqui.")
        end,
      })
    elseif p and p.report then
      -- Sem o d1, o relato corre pelo NLU (o backend antigo da barra):
      -- `quest_turn_in_parley` valida a palavra e é o ÚNICO caminho da
      -- entrega parcial (`parley.partial`). Sem isto o único chamador do
      -- `partial` era código morto e os 3 dadores de collect entregavam
      -- tudo de uma vez.
      talk.say(p.report.prompt or "Diz-me o que encontraste.")
      talk.ask({
        prompt = p.report.prompt,
        example = p.report.example,
        state = { npc = def and def.title or id, quest = id },
        on_result = function(parsed, text)
          local ok, msg = rules.quest_turn_in_parley(id, text, parsed)
          if ok then
            say(def and def.lines_complete, "Entregue.", 0)
          else
            talk.say(msg or p.report.hint or "Não te ouvi.")
          end
        end,
        on_unavailable = function()
          if rules.quest_turn_in(id) then
            say(def and def.lines_complete, "Entregue.", 0)
          else
            say(def and def.lines_progress, "Ainda não tens o que a missão pede.", remaining)
          end
        end,
        on_cancel = function()
          talk.say("Quando quiseres contar, estou aqui.")
        end,
      })
    else
      if rules.quest_turn_in(id) then
        say(def and def.lines_complete, "Entregue.", 0)
      else
        say(def and def.lines_progress, "Ainda não tens o que a missão pede.", remaining)
      end
    end
  elseif state == "active" then
    say(def and def.lines_progress, "Ainda não.", remaining)
  else
    talk.say("Já tratámos disso.")
  end
end
