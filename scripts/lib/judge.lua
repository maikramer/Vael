-- O juiz: transforma o registo de ações em veredito.
--
-- O modelo NÃO decide (testámos a cabeça de classificação zero-shot: 5/8 e
-- viés para o primeiro rótulo). O modelo faz o que sabe — lê a frase da ação
-- e extrai as entidades; o veredito é simbólico (requisitos, ordem,
-- proibições) e por isso é explicável e determinístico.
--
-- Regra (`data/verdicts.json`):
--   require: {{kind, id, verb?}}   -- todas têm de aparecer
--   order:   {"matar","visitar",…} -- índices crescentes no registo
--   forbid:  {{kind, id}}          -- qualquer uma invalida
--   window:  segundos (não usada no filtro; o t0 já limita)
local M = {}

local rules = viber.load("lib/rules.lua")
local parley = viber.load("lib/parley.lua")
local actions = viber.load("lib/actions.lua")
local decide = viber.load("lib/decide.lua")

local defs
local MAX_PARSE = 16
-- Cache por frase: o juiz reavalia o mesmo registo a cada [E].
local decision_cache = {}

local function load()
  if defs then
    return
  end
  defs = {}
  local ok, data = pcall(viber.load_json, "data/verdicts.json")
  if ok and type(data) == "table" then
    for _, r in ipairs(data.rules or {}) do
      defs[r.id] = r
    end
  end
end

function M.get(id)
  load()
  return defs and defs[id] or nil
end

function M.started(id)
  return tonumber(viber.game()["judge." .. tostring(id) .. ".t0"]) ~= nil
end

function M.start(id)
  viber.game()["judge." .. tostring(id) .. ".t0"] = viber.time()
  viber.game()["judge." .. tostring(id) .. ".done"] = nil
end

function M.reset(id)
  viber.game()["judge." .. tostring(id) .. ".t0"] = nil
  viber.game()["judge." .. tostring(id) .. ".done"] = nil
end

function M.solved(id)
  return viber.game()["judge." .. tostring(id) .. ".done"] == true
end

-- Núcleo puro: recebe entradas com `verb` e `res` (kind -> {id=true}).
function M.evaluate_log(rule, entries)
  local reqs = rule.require or {}
  local matched, used = {}, {}
  for _, req in ipairs(reqs) do
    for i, e in ipairs(entries) do
      if not used[i]
        and (not req.verb or e.verb == req.verb)
        and type(e.res) == "table"
        and type(e.res[req.kind]) == "table"
        and e.res[req.kind][req.id] then
        used[i] = true
        matched[#matched + 1] = { req = req, index = i }
        break
      end
    end
  end

  local order_ok = true
  if type(rule.order) == "table" and #rule.order > 0 then
    local last = 0
    for _, verb in ipairs(rule.order) do
      local found = nil
      for _, m in ipairs(matched) do
        if m.req.verb == verb and m.index > last then
          found = m.index
          break
        end
      end
      if not found then
        order_ok = false
        break
      end
      last = found
    end
  end

  local forbidden = nil
  for _, f in ipairs(rule.forbid or {}) do
    for _, e in ipairs(entries) do
      if type(e.res) == "table"
        and type(e.res[f.kind]) == "table"
        and e.res[f.kind][f.id] then
        forbidden = f
        break
      end
    end
  end

  local missing = {}
  for _, req in ipairs(reqs) do
    local ok = false
    for _, m in ipairs(matched) do
      if m.req == req then
        ok = true
        break
      end
    end
    if not ok then
      missing[#missing + 1] = req
    end
  end

  local need = #reqs
  local got = #matched
  local verdict = "wrong"
  if forbidden then
    verdict = "wrong"
  elseif need == 0 then
    verdict = "wrong"
  elseif got == need and order_ok then
    verdict = "correct"
  elseif got > 0 then
    verdict = "partial"
  end
  return {
    verdict = verdict,
    got = got,
    need = need,
    order_ok = order_ok,
    forbidden = forbidden,
    missing = missing,
    matched = matched,
  }
end

-- Lê o registo desde `t0`, manda cada frase ao modelo e julga.
function M.evaluate(id)
  local rule = M.get(id)
  if not rule then
    return nil
  end
  local t0 = tonumber(viber.game()["judge." .. tostring(id) .. ".t0"]) or 0
  local entries = actions.since(t0)
  -- Cap: um registo antigo não pode custar dezenas de inferências.
  if #entries > MAX_PARSE then
    local trimmed = {}
    for i = #entries - MAX_PARSE + 1, #entries do
      trimmed[#trimmed + 1] = entries[i]
    end
    entries = trimmed
  end
  -- Caminho novo: o d1 diz qual requisito cada ação cumpre. A regra
  -- (ordem/proibições/recompensa) continua simbólica e explicável.
  if decide.ready() then
    local reqs = rule.require or {}
    local verb_label = {
      matar = "matou", visitar = "visitou", equipar = "equipou",
      usar = "usou", comprar = "comprou", vender = "vendeu",
    }
    local questions = {}
    for i, req in ipairs(reqs) do
      local v = verb_label[req.verb] or req.verb or "fez"
      questions["r" .. i] = decide.noul(
        "A ação mostra que o herói " .. v .. " " .. decide.label(req.kind, req.id) .. "?",
        "não mostra", "mostra"
      )
    end
    for _, e in ipairs(entries) do
      local answers = decision_cache[e.text]
      if not answers then
        answers = decide.decide({ action = e.text }, questions) or {}
        decision_cache[e.text] = answers
      end
      for i, req in ipairs(reqs) do
        local a = answers["r" .. i]
        if a and (tonumber(a.noul) or 0) >= 0.5 then
          e.res = e.res or {}
          e.res[req.kind] = e.res[req.kind] or {}
          e.res[req.kind][req.id] = true
          e.verb = req.verb
        end
      end
      -- O registo já sabe o id/verb da ação: é a verdade determinística e
      -- também alimenta as proibições (que não têm pergunta `noul`).
      if e.kind and e.id then
        e.res = e.res or {}
        e.res[e.kind] = e.res[e.kind] or {}
        e.res[e.kind][e.id] = true
      end
    end
    local result = M.evaluate_log(rule, entries)
    result.entries = #entries
    result.model = true
    viber.game()["judge." .. tostring(id) .. ".last"] = result.verdict
    return result
  end

  local model_ready = viber.nlu and viber.nlu.ready and viber.nlu.ready()
  for _, e in ipairs(entries) do
    local parsed = nil
    if model_ready then
      local ok, p = pcall(viber.nlu.parse, e.text)
      if ok then
        parsed = p
      end
    end
    e.res = parley.resolve(e.text, parsed)
    -- Fallback determinístico: o registo já sabe o id. Sem modelo (ou sem
    -- extração) o juiz continua a funcionar; o modelo é a camada semântica.
    if e.kind and e.id then
      e.res[e.kind] = e.res[e.kind] or {}
      e.res[e.kind][e.id] = true
    end
  end
  local result = M.evaluate_log(rule, entries)
  result.entries = #entries
  result.model = model_ready
  viber.game()["judge." .. tostring(id) .. ".last"] = result.verdict
  return result
end

function M.reward(id, result)
  local rule = M.get(id)
  if not rule or not result then
    return
  end
  if result.verdict == "correct" then
    viber.game()["judge." .. tostring(id) .. ".done"] = true
    local rewards = rule.reward or {}
    if rewards.xp and rewards.xp > 0 then
      rules.add_xp(rewards.xp)
    end
    for _, item in ipairs(rewards.items or {}) do
      local name, count = tostring(item):match("^([^:]+):?(%d*)$")
      if name and name ~= "" then
        rules.item_add(name, tonumber(count) or 1)
      end
    end
    viber.toast("Juramento cumprido (+" .. (rewards.xp or 0) .. " XP)")
    viber.sound("quest_complete")
  elseif result.verdict == "partial" then
    viber.toast("O altar reconhece parte do juramento.")
    viber.sound("notification")
  else
    viber.sound("error")
  end
end

-- Teste puro do juiz (bridge: `viber.load("lib/judge.lua").self_test()`).
function M.self_test()
  local function e(verb, kind, id)
    return { verb = verb, kind = kind, id = id, res = { [kind] = { [id] = true } } }
  end
  local rule = {
    require = {
      { kind = "criatura", id = "wolf", verb = "matar" },
      { kind = "local", id = "forest-crossroads-well", verb = "visitar" },
      { kind = "item", id = "torch", verb = "equipar" },
    },
    order = { "matar", "visitar", "equipar" },
    forbid = { { kind = "item", id = "stone" } },
  }
  local cases = {
    {
      name = "ordem certa",
      entries = {
        e("matar", "criatura", "wolf"),
        e("visitar", "local", "forest-crossroads-well"),
        e("equipar", "item", "torch"),
      },
      want = "correct",
    },
    {
      name = "ordem trocada",
      entries = {
        e("equipar", "item", "torch"),
        e("visitar", "local", "forest-crossroads-well"),
        e("matar", "criatura", "wolf"),
      },
      want = "partial",
    },
    {
      name = "falta a tocha",
      entries = {
        e("matar", "criatura", "wolf"),
        e("visitar", "local", "forest-crossroads-well"),
      },
      want = "partial",
    },
    {
      name = "proibido",
      entries = {
        e("matar", "criatura", "wolf"),
        e("visitar", "local", "forest-crossroads-well"),
        e("equipar", "item", "torch"),
        e("usar", "item", "stone"),
      },
      want = "wrong",
    },
    { name = "nada feito", entries = {}, want = "wrong" },
  }
  local errors = {}
  for _, c in ipairs(cases) do
    local r = M.evaluate_log(rule, c.entries)
    if r.verdict ~= c.want then
      errors[#errors + 1] = c.name .. ": " .. r.verdict .. " ~= " .. c.want
    end
  end
  return #errors == 0, errors
end

return M
