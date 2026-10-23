local rules = viber.load("lib/rules.lua")
local talk = viber.load("ui/say.lua")
local decide = viber.load("lib/decide.lua")
-- notice-board.lua: quadro de avisos da praça — bounties repetíveis da
-- guarda (quest npc == "notice_board" nos JSONs).
--
-- Com o NLU disponível, [E] abre a barra de fala: uma palavra ("lobos",
-- "goblins", "lenha") escolhe o cartaz e serve de relato. Sem NLU, mantém-se
-- o ciclo antigo (entrega → aceita → progresso).
local BOUNTIES = { "city_wolves", "city_bandits", "city_goblins", "city_wood" }

local function typing()
  return viber.ui and viber.ui.focused and viber.ui.focused() ~= nil
end

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    st.index = 0
    viber.set_interaction("Ler o quadro", "e", 3.0)
  end
  if viber.menu_open() or talk.active() or typing() then
    return
  end
  -- O `menu_open` não inclui o ecrã de morte: durante os 2,2 s de `dying` o
  -- [E] ainda chega aqui e o quadro entregava bounties (ouro/XP/itens) a um
  -- cadáver. O mesmo portão dos restantes POIs e do baú.
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return
  end
  if not viber.interacted("e") then
    return
  end

  local function speak(id, which)
    local def = rules.quest_def(id)
    local lines = def and def[which]
    local have, need = rules.quest_progress(id)
    local obj = def and def.objective
    if obj and obj.type == "collect" then
      -- Vender o item depois de a quest ficar pronta deixava `{remaining}=0`.
      have = rules.vault_get(obj.target or "")
    end
    local text = table.concat(lines or {}, "\n")
    text = text:gsub("{remaining}", tostring(math.max(0, (need or 1) - (have or 0))))
    if text ~= "" then
      viber.say(text)
      viber.ui.set("dialogue.until", viber.time() + 0.75)
    end
  end

  -- Fluxo antigo: entrega ganha a aceitar; sem NLU é o que resta.
  local total = #BOUNTIES
  local function find(status)
    for step = 1, total do
      local i = ((st.index or 0) + step - 1) % total + 1
      local id = BOUNTIES[i]
      if rules.quest_state(id) == status then
        return i, id
      end
    end
    return nil, nil
  end
  local function cycle()
    local i, id = find("ready")
    if id then
      if rules.quest_turn_in(id) then
        speak(id, "lines_complete")
      else
        speak(id, "lines_progress")
      end
      st.index = i
      return
    end
    i, id = find("not_taken")
    if id then
      speak(id, "lines_intro")
      rules.quest_accept(id)
      st.index = i
      return
    end
    i, id = find("active")
    if id then
      speak(id, "lines_progress")
      st.index = i
      return
    end
    viber.toast("Nenhuma bounty nova — termine as que aceitou.")
  end

  -- Uma palavra solta ("lobos") faz o d1 responder "nenhum"; os aliases
  -- autorais resolvem o caso exato. O d1 cobre frases e sinónimos.
  local function pick_alias(text)
    for _, want in ipairs({ "ready", "not_taken", "active" }) do
      for _, id in ipairs(BOUNTIES) do
        if rules.quest_state(id) == want and rules.parley_matches(id, text, nil) then
          return id
        end
      end
    end
    return nil
  end

  if not decide.ready() then
    cycle()
    return
  end

  local options = {}
  for _, id in ipairs(BOUNTIES) do
    local def = rules.quest_def(id)
    local obj = def and def.objective or {}
    local kind = obj.type == "collect" and "item" or "criatura"
    local label = obj.target and decide.label(kind, obj.target) or ""
    options[#options + 1] = {
      key = id,
      description = (def and def.title or id) .. (label ~= "" and (" — " .. label) or ""),
    }
  end
  options[#options + 1] = { key = "none", description = "nenhum aviso; outra coisa" }

  viber.say("O que procuras? Diz uma palavra.")
  talk.ask({
    prompt = "O que procuras?",
    example = "lobos, goblins, lenha",
    state = { board = "praça da vila" },
    questions = { q = decide.choice("Qual aviso procura o jogador?", options) },
    on_result = function(answers, text)
      local id = answers.q and answers.q.choice
      if not id or id == "none" then
        id = pick_alias(text)
      end
      if not id then
        viber.say("Nenhum aviso sobre isso.")
        return
      end
      local state = rules.quest_state(id)
      if state == "ready" then
        if rules.quest_turn_in(id) then
          speak(id, "lines_complete")
        else
          speak(id, "lines_progress")
        end
      elseif state == "not_taken" then
        speak(id, "lines_intro")
        rules.quest_accept(id)
      elseif state == "active" then
        speak(id, "lines_progress")
      end
    end,
    on_unavailable = function()
      cycle()
    end,
    on_cancel = function()
      viber.say("O quadro continua lá.")
    end,
  })
end
