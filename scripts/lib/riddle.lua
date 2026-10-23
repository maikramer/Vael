-- Enigmas mudos: a pista é som/objeto/estátua (nunca texto); a resposta é
-- uma palavra (ou três, no caso das oferendas) e o modelo é o juiz semântico.
--
-- O que o modelo acrescenta ao `if answer == "lobo"`:
--   * aceita sinónimos e plurais ("lobo", "lobos", "alcateia");
--   * valida o TIPO (criatura/item/local), não só a string;
--   * extrai VÁRIAS entidades de uma frase ("pedra, madeira e fruta") e o
--     enigma pode exigir todas.
--
-- Estado em `viber.game()["riddle.<id>"]` (primitivo, persiste no save).
local M = {}

local rules = viber.load("lib/rules.lua")
local parley = viber.load("lib/parley.lua")
local marks = viber.load("lib/landmarks.lua")
local decide = viber.load("lib/decide.lua")

local defs          -- id -> riddle
local by_site       -- entity name -> riddle

local function load()
  if defs then
    return
  end
  defs, by_site = {}, {}
  local ok, data = pcall(viber.load_json, "data/riddles.json")
  if not ok or type(data) ~= "table" then
    viber.log("enigmas: falha a ler data/riddles.json")
    return
  end
  for _, r in ipairs(data.riddles or {}) do
    defs[r.id] = r
    if r.site and r.site ~= "" then
      by_site[r.site] = r
    end
  end
end

function M.get(id)
  load()
  return defs and defs[id] or nil
end

function M.by_site(site)
  load()
  return by_site and by_site[site] or nil
end

function M.solved(id)
  return viber.game()["riddle." .. tostring(id)] == true
end

function M.mark_solved(id)
  viber.game()["riddle." .. tostring(id)] = true
end

-- A resposta do jogador satisfaz o enigma?
-- `all` exige todas as entidades (as três oferendas); senão basta uma.
function M.solve(id, text, parsed)
  local r = M.get(id)
  if not r then
    return false, "Enigma desconhecido."
  end
  local res = parley.resolve(text, parsed)
  if r.all then
    for _, e in ipairs(r.expect or {}) do
      local set = res[e.kind]
      if type(set) ~= "table" or not set[e.id] then
        return false, r.hint or "Falta uma das oferendas."
      end
    end
    return true, nil
  end
  local ok = parley.match_expect(res, r.expect)
  if not ok then
    return false, r.hint or "Não te ouvi dizer o nome."
  end
  return true, nil
end

-- Julgamento com as respostas do d1: cada expect tem uma pergunta "rN".
function M.solve_decision(id, text, answers)
  local r = M.get(id)
  if not r then
    return false, "Enigma desconhecido."
  end
  local expect = r.expect or {}
  local matched = 0
  for i = 1, #expect do
    local a = answers and answers["r" .. i]
    if a and (tonumber(a.noul) or 0) >= 0.5 then
      matched = matched + 1
    end
  end
  if (r.all and matched == #expect) or (not r.all and matched > 0) then
    return true, nil
  end
  return false, r.hint or "Não te ouvi dizer o nome."
end

-- Marcos que o enigma exige terem sido medidos com [F] (rota não verbal).
function M.require_met(id)
  local r = M.get(id)
  if not r or type(r.require_marks) ~= "table" then
    return true, 0
  end
  local missing = 0
  for _, name in ipairs(r.require_marks) do
    if not marks.is_marked(name) then
      missing = missing + 1
    end
  end
  return missing == 0, missing
end

function M.reward(id)
  local r = M.get(id)
  if not r then
    return
  end
  local rewards = r.reward or {}
  if rewards.xp and rewards.xp > 0 then
    rules.add_xp(rewards.xp)
  end
  local missing = 0
  for _, item in ipairs(rewards.items or {}) do
    local name, count = tostring(item):match("^([^:]+):?(%d*)$")
    local want = tonumber(count) or 1
    if name and name ~= "" and want > 0 then
      local added = rules.item_add(name, want) or 0
      if added < want then
        missing = missing + (want - added)
      end
    end
  end
  local msg = "Enigma resolvido"
  if rewards.xp and rewards.xp > 0 then
    msg = msg .. " (+" .. rewards.xp .. " XP)"
  end
  if missing > 0 then
    msg = msg .. " — " .. missing .. " item(ns) não coube(ram)."
  end
  viber.toast(msg)
  viber.sound("quest_complete")
end

-- Abre a barra de fala com a pergunta do enigma. O chamador (santuário)
-- decide o prompt de interação; aqui só tratamos da resposta.
function M.ask(id, on_solved)
  local r = M.get(id)
  if not r then
    return false
  end
  local say = viber.load("ui/say.lua")
  if not decide.ready() then
    viber.say("O santuário espera uma voz, mas o vento leva-a.")
    return false
  end
  viber.say(r.prompt)
  return say.ask({
    prompt = r.prompt,
    example = r.example,
    timeout = 30,
    state = { riddle = r.prompt, clue = r.clue },
    questions = decide.expect_questions(r.expect, r.prompt),
    on_result = function(answers, text)
      local ok, hint = M.solve_decision(id, text, answers)
      if ok then
        M.mark_solved(id)
        M.reward(id)
        viber.say(r.solved or "O santuário aquieta-se.")
        if on_solved then
          on_solved()
        end
      else
        viber.say(hint or r.hint or "Não te ouvi.")
      end
    end,
    on_cancel = function()
      viber.say("O santuário continua à espera.")
    end,
  })
end

-- Teste do juiz (bridge: `viber.load("lib/riddle.lua").self_test()`).
function M.self_test()
  load()
  local cases = {
    { "echo_wolf", "lobos", true },
    { "echo_wolf", "sombra", false },
    { "three_gifts", "pedra, madeira e fruta", true },
    { "three_gifts", "pedra e madeira", false },
    { "three_gifts", "fruta e pedra e madeira", true },
    { "odd_statue", "sombra", true },
    { "odd_statue", "lobos", false },
  }
  local errors = {}
  for _, c in ipairs(cases) do
    local ok = M.solve(c[1], c[2], nil)
    if ok ~= c[3] then
      errors[#errors + 1] = c[1] .. " / " .. c[2] .. ": esperava " .. tostring(c[3])
    end
  end
  return #errors == 0, errors
end

return M

