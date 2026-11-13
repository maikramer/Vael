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
    viber.log("enigmas: falha a ler data/riddles.json"
      .. (ok and "" or (" (" .. tostring(data) .. ")")))
    return
  end
  for _, r in ipairs(data.riddles or {}) do
    -- Entrada primária (edição à mão): `defs[r.id]` com r não-tabela era
    -- "table index is nil" e rebentava o primeiro [E] em qualquer santuário.
    if type(r) == "table" and type(r.id) == "string" and r.id ~= "" then
      defs[r.id] = r
      if type(r.site) == "string" and r.site ~= "" then
        by_site[r.site] = r
      end
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

local function expect_list(r)
  local raw = r and r.expect
  if type(raw) ~= "table" then
    return {}
  end
  if #raw > 0 then
    return raw
  end
  local list = {}
  for k, v in pairs(raw) do
    local kind = k == "marco" and "local" or k
    if type(kind) == "string" and type(v) == "string" then
      list[#list + 1] = { kind = kind, id = v }
    end
  end
  return list
end

local function matches_entry(r, res, e)
  local set = res[e.kind]
  if type(set) ~= "table" then
    return false
  end
  if e.id == "*" then
    if e.kind == "local" and type(r.require_marks) == "table" and #r.require_marks > 0 then
      for _, name in ipairs(r.require_marks) do
        if set[name] then
          return true
        end
      end
      return false
    end
    return next(set) ~= nil
  end
  return set[e.id] == true
end

local function decision_expect(r)
  local out = {}
  for _, e in ipairs(expect_list(r)) do
    if e.id == "*" and e.kind == "local" and type(r.require_marks) == "table" and #r.require_marks > 0 then
      for _, name in ipairs(r.require_marks) do
        out[#out + 1] = { kind = "local", id = name }
      end
    else
      out[#out + 1] = e
    end
  end
  return out
end

-- A resposta do jogador satisfaz o enigma?
-- `all` exige todas as entidades (as três oferendas); senão basta uma.
function M.solve(id, text, parsed)
  local r = M.get(id)
  if not r then
    return false, "Enigma desconhecido."
  end
  local met, missing = M.require_met(id)
  if not met then
    return false, "Faltam " .. tostring(missing) .. " marcos por medir."
  end
  local res = parley.resolve(text, parsed)
  local expect = expect_list(r)
  if r.all then
    for _, e in ipairs(expect) do
      if not matches_entry(r, res, e) then
        return false, r.hint or "Falta uma das oferendas."
      end
    end
    return #expect > 0, nil
  end
  for _, e in ipairs(expect) do
    if matches_entry(r, res, e) then
      return true, nil
    end
  end
  return false, r.hint or "Não te ouvi dizer o nome."
end

-- Julgamento com as respostas do d1: cada expect tem uma pergunta "rN".
function M.solve_decision(id, text, answers)
  local r = M.get(id)
  if not r then
    return false, "Enigma desconhecido."
  end
  local met, missing = M.require_met(id)
  if not met then
    return false, "Faltam " .. tostring(missing) .. " marcos por medir."
  end
  local expect = decision_expect(r)
  local matched = 0
  for i = 1, #expect do
    local a = answers and answers["r" .. i]
    if a and (tonumber(a.noul) or 0) >= 0.5 then
      matched = matched + 1
    end
  end
  if (r.all and #expect > 0 and matched == #expect) or (not r.all and matched > 0) then
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
  if rewards.gold and rewards.gold > 0 then
    rules.vault_add("gold", rewards.gold)
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
  local parts = {}
  if rewards.xp and rewards.xp > 0 then
    parts[#parts + 1] = "+" .. rewards.xp .. " XP"
  end
  if rewards.gold and rewards.gold > 0 then
    parts[#parts + 1] = "+" .. rewards.gold .. " ouro"
  end
  local msg = "Enigma resolvido"
  if #parts > 0 then
    msg = msg .. " (" .. table.concat(parts, ", ") .. ")"
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
  local use_decision = decide.ready()
  local resolved = false
  local asked = say.ask({
    prompt = r.prompt,
    example = r.example,
    timeout = 30,
    allow_fallback = true,
    state = { riddle = r.prompt, clue = r.clue },
    questions = use_decision and decide.expect_questions(decision_expect(r), r.prompt) or nil,
    on_result = function(parsed, text)
      resolved = true
      local ok, hint
      if use_decision and decide.ready() and type(parsed) == "table" then
        ok, hint = M.solve_decision(id, text, parsed)
      else
        ok, hint = M.solve(id, text, parsed)
      end
      if ok then
        M.mark_solved(id)
        M.reward(id)
        say.say(r.solved or "O santuário aquieta-se.")
        if on_solved then
          on_solved()
        end
      else
        say.say(hint or r.hint or "Não te ouvi.")
      end
    end,
    on_unavailable = function()
      resolved = true
      say.say("O santuário espera uma voz, mas o vento leva-a.")
    end,
    on_cancel = function()
      resolved = true
      say.say("O santuário continua à espera.")
    end,
  })
  if asked and not resolved then
    say.say(r.prompt)
  end
  return asked
end

-- Teste do juiz (bridge: `viber.load("lib/riddle.lua").self_test()`).
function M.self_test()
  load()
  local errors = {}
  local cases = {
    { "echo_wolf", "lobos", true },
    { "echo_wolf", "sombra", false },
    { "three_gifts", "pedra, madeira e fruta", true },
    { "three_gifts", "pedra e madeira", false },
    { "three_gifts", "fruta e pedra e madeira", true },
    { "odd_statue", "sombra", true },
    { "odd_statue", "lobos", false },
    { "desert_venom", "escorpião e cristal", true },
    { "desert_venom", "escorpião", false },
    { "desert_venom", "cristal", false },
    { "peaks_echo", "cristal e sombra", true },
    { "peaks_echo", "pedra e sombra", true },
    { "peaks_echo", "sombra", false },
    { "peaks_echo", "lobos", false },
  }
  for _, c in ipairs(cases) do
    local ok = M.solve(c[1], c[2], nil)
    if ok ~= c[3] then
      errors[#errors + 1] = c[1] .. " / " .. c[2] .. ": esperava " .. tostring(c[3])
    end
  end

  local route = M.get("menhir_route")
  if not route then
    errors[#errors + 1] = "menhir_route: enigma ausente"
  else
    local game = viber.game()
    local saved = {}
    for _, name in ipairs(route.require_marks or {}) do
      local key = marks.marked_key(name)
      saved[key] = game[key]
      game[key] = nil
    end

    local ok_missing = M.solve("menhir_route", "arco do deserto", nil)
    if ok_missing ~= false then
      errors[#errors + 1] = "menhir_route sem marcos: esperava false"
    end

    -- Um marco ainda por medir (falta o terceiro).
    if route.require_marks and #route.require_marks >= 2 then
      game[marks.marked_key(route.require_marks[1])] = true
      game[marks.marked_key(route.require_marks[2])] = true
      local ok_partial = M.solve("menhir_route", "arco do deserto", nil)
      if ok_partial ~= false then
        errors[#errors + 1] = "menhir_route com marcos parciais: esperava false"
      end
    end

    for _, name in ipairs(route.require_marks or {}) do
      game[marks.marked_key(name)] = true
    end
    local route_cases = {
      { "arco do deserto", true },
      { "obelisco do sol", true },
      { "círculo de menires", true },
      { "poço da encruzilhada", false },
      { "lobos", false },
    }
    for _, c in ipairs(route_cases) do
      local ok = M.solve("menhir_route", c[1], nil)
      if ok ~= c[2] then
        errors[#errors + 1] = "menhir_route / " .. c[1] .. ": esperava " .. tostring(c[2])
      end
    end

    for key, val in pairs(saved) do
      game[key] = val
    end
    for _, name in ipairs(route.require_marks or {}) do
      local key = marks.marked_key(name)
      if saved[key] == nil then
        game[key] = nil
      end
    end
  end

  return #errors == 0, errors
end

return M

