-- NPC de uma quest de decisão: o jogador pergunta por texto livre, o d1
-- classifica a pergunta contra as pistas e, no fim, contra o desfecho.
--
-- O JSON do puzzle vive em `data/quest_puzzles/<quest_id>.json`:
--   intro, ask, example, clues[{key,description,reply}], required,
--   final_prompt, final_options[{key,description}], final_answer,
--   final_reply_ok/bad, endings[{text,items}], hint.
--
-- A quest em `quests/*.json` só dá o título, as linhas e as recompensas; o
-- objetivo é `{type:"decision", target:"...", count:1}`. O script marca o
-- objetivo com `rules.quest_set_progress` e entrega com `quest_turn_in`.
--@export quest_id: string = ""

local rules = viber.load("lib/rules.lua")
local talk = viber.load("ui/say.lua")
local decide = viber.load("lib/decide.lua")

local function typing()
  return viber.ui and viber.ui.focused and viber.ui.focused() ~= nil
end

local function flag(id, key)
  return viber.game()["puzzle." .. id .. "." .. key] == true
end

local function set_flag(id, key)
  viber.game()["puzzle." .. id .. "." .. key] = true
end

local function known_text(id, puzzle)
  local out = {}
  for _, c in ipairs(puzzle.clues or {}) do
    if flag(id, c.key) then
      out[#out + 1] = c.reply
    end
  end
  return table.concat(out, "\n")
end

local function known_count(id, puzzle)
  local n = 0
  for _, c in ipairs(puzzle.clues or {}) do
    if flag(id, c.key) then
      n = n + 1
    end
  end
  return n
end

local function required(puzzle)
  return math.max(1, tonumber(puzzle.required) or #(puzzle.clues or {}))
end

local function all_known(id, puzzle)
  return known_count(id, puzzle) >= required(puzzle)
end

local function say_lines(lines, fallback)
  local text = table.concat(lines or {}, "\n")
  if text == "" then
    text = fallback or ""
  end
  if text ~= "" then
    viber.say(text)
    viber.ui.set("dialogue.until", viber.time() + 0.75)
  end
end

local function ask_clues(id, puzzle)
  if not decide.ready() then
    viber.say("As palavras perdem-se no vento. (O modelo de decisão não está pronto.)")
    return false
  end
  local options = {}
  for _, c in ipairs(puzzle.clues or {}) do
    options[#options + 1] = { key = c.key, description = c.description }
  end
  options[#options + 1] = { key = "none", description = "outra pergunta, ou nada" }

  viber.say(puzzle.ask or "O que perguntas?")
  return talk.ask({
    prompt = puzzle.ask or "O que perguntas?",
    example = puzzle.example,
    timeout = 30,
    state = {
      npc = puzzle.giver or "",
      quest = id,
      clues = known_text(id, puzzle),
    },
    questions = { q = decide.choice(puzzle.ask or "O que perguntas?", options) },
    on_result = function(answers, _text)
      local key = answers.q and answers.q.choice
      local clue
      for _, c in ipairs(puzzle.clues or {}) do
        if c.key == key then
          clue = c
          break
        end
      end
      if not clue then
        viber.say(puzzle.no_answer or "Não sei do que falas.")
        return
      end
      if flag(id, clue.key) then
        viber.say(clue["repeat"] or "Já te disse isso. Ouve com atenção.")
        return
      end
      set_flag(id, clue.key)
      viber.say(clue.reply)
      if all_known(id, puzzle) then
        viber.say(puzzle.reveal or "Já tens o suficiente. Fala comigo outra vez.")
      else
        local left = required(puzzle) - known_count(id, puzzle)
        viber.say(puzzle.more or ("Falta perceber " .. left .. " coisa(s)."))
      end
    end,
    on_unavailable = function()
      viber.say("As palavras perdem-se no vento. Tenta outra vez.")
    end,
    on_cancel = function()
      viber.say(puzzle.cancel or "Pensa melhor. Eu espero.")
    end,
  })
end

local function grant_items(items)
  for _, item in ipairs(items or {}) do
    local name, count = tostring(item):match("^([^:]+):?(%d*)$")
    if name and name ~= "" then
      rules.item_add(name, tonumber(count) or 1)
    end
  end
end

local function ask_final(id, puzzle, st)
  if not decide.ready() then
    viber.say("As palavras perdem-se no vento. (O modelo de decisão não está pronto.)")
    return false
  end
  local options = puzzle.final_options or {}
  viber.say(puzzle.final_prompt or "O que dizes?")
  return talk.ask({
    prompt = puzzle.final_prompt or "O que dizes?",
    example = puzzle.final_example,
    timeout = 30,
    state = {
      npc = puzzle.giver or "",
      quest = id,
      clues = known_text(id, puzzle),
    },
    questions = { q = decide.choice(puzzle.final_prompt or "O que dizes?", options) },
    on_result = function(answers, _text)
      local key = answers.q and answers.q.choice
      local ending = puzzle.endings and puzzle.endings[key]
      if ending then
        viber.say(ending.text or puzzle.final_reply_ok)
        grant_items(ending.items)
        rules.quest_set_progress(id, 1)
        rules.quest_turn_in(id)
        st.done = true
        return
      end
      if key == puzzle.final_answer then
        viber.say(puzzle.final_reply_ok)
        rules.quest_set_progress(id, 1)
        rules.quest_turn_in(id)
        st.done = true
      else
        viber.say(puzzle.final_reply_bad or "Errado. Ouve outra vez.")
        if puzzle.hint then
          viber.say(puzzle.hint)
        end
      end
    end,
    on_unavailable = function()
      viber.say("As palavras perdem-se no vento. Tenta outra vez.")
    end,
    on_cancel = function()
      viber.say(puzzle.final_cancel or "Ainda não. Pensa melhor.")
    end,
  })
end

function on_update(_dt)
  local id = viber.prop("quest_id")
  if not id or id == "" then
    return
  end
  local st = viber.state()
  local def = rules.quest_def(id)
  local puzzle = st.puzzle
  if not puzzle then
    local ok, data = pcall(viber.load_json, "data/quest_puzzles/" .. id .. ".json")
    if not ok or type(data) ~= "table" then
      return
    end
    puzzle = data
    st.puzzle = puzzle
  end
  if not st.ready then
    st.ready = true
    viber.set_interaction("Falar com " .. (puzzle.giver or (def and def.title) or id), "e", 3.5)
  end
  if viber.menu_open() or talk.active() or typing() or not viber.interacted("e") then
    return
  end
  viber.face_player()
  viber.gesture("talk")

  local state = rules.quest_state(id)
  if state == "not_taken" then
    rules.quest_accept(id)
    say_lines(puzzle.intro, def and table.concat(def.lines_intro or {}, "\n"))
    ask_clues(id, puzzle)
  elseif state == "active" then
    if all_known(id, puzzle) then
      ask_final(id, puzzle, st)
    else
      ask_clues(id, puzzle)
    end
  elseif state == "ready" then
    if rules.quest_turn_in(id) then
      say_lines(puzzle.after or (def and def.lines_complete), "Entregue.")
    else
      say_lines(def and def.lines_progress, "Ainda não.")
    end
  else
    say_lines(puzzle.after or (def and def.lines_complete), "Já tratámos disso.")
  end
end
