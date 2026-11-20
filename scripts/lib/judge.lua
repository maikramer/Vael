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

local rules = vael.load("lib/rules.lua")
local parley = vael.load("lib/parley.lua")
local actions = vael.load("lib/actions.lua")
local decide = vael.load("lib/decide.lua")

local defs
-- Quantas entradas RECENTES vão ao modelo (as antigas só se forem uma ação
-- exigida/proibida — ver `cap_entries`).
local MAX_PARSE = 16
-- Cache por frase: o juiz reavalia o mesmo registo a cada [E].
local decision_cache = {}

local function load()
  if defs then
    return
  end
  defs = {}
  local ok, data = pcall(vael.load_json, "data/verdicts.json")
  if not ok or type(data) ~= "table" then
    vael.log("juiz: falha a ler data/verdicts.json — sem veredictos"
      .. (ok and "" or (" (" .. tostring(data) .. ")")))
    return
  end
  for _, r in ipairs(type(data.rules) == "table" and data.rules or {}) do
    -- `defs[r.id]` com r não-tabela (ou sem id) era "table index is nil" e
    -- rebentava o primeiro [E] no altar, não só esta regra. Cada `require`
    -- tem de ter kind/id strings: um requisito malformado rebentava o
    -- `e.res[req.kind][req.id]` do `evaluate` (e o `forbid` no `evaluate_log`).
    if type(r) == "table" and type(r.id) == "string" and r.id ~= "" then
      local ok_rule = true
      if type(r.require) ~= "table" and r.require ~= nil then
        vael.log("juiz: regra '" .. r.id .. "' com require malformado — regra ignorada")
        ok_rule = false
      end
      if ok_rule and type(r.forbid) ~= "table" and r.forbid ~= nil then
        vael.log("juiz: regra '" .. r.id .. "' com forbid malformado — regra ignorada")
        ok_rule = false
      end
      for _, req in ipairs(type(r.require) == "table" and r.require or {}) do
        if type(req) ~= "table" or type(req.kind) ~= "string"
            or type(req.id) ~= "string" then
          vael.log("juiz: regra '" .. r.id .. "' com requisito malformado — regra ignorada")
          ok_rule = false
          break
        end
      end
      for _, f in ipairs(type(r.forbid) == "table" and r.forbid or {}) do
        if type(f) ~= "table" or type(f.kind) ~= "string" or type(f.id) ~= "string" then
          vael.log("juiz: regra '" .. r.id .. "' com proibição malformada — regra ignorada")
          ok_rule = false
          break
        end
      end
      if ok_rule then
        defs[r.id] = r
      end
    end
  end
end

function M.get(id)
  load()
  return defs and defs[id] or nil
end

function M.started(id)
  local t0 = tonumber(vael.game()["judge." .. tostring(id) .. ".t0"])
  -- `vael.time()` é o uptime do PROCESSO e nunca recua: um `t0` no futuro
  -- é resto de outro arranque (o save guarda números). A janela do juramento
  -- morreu com a sessão — devolver `false` faz o altar reabrir as marcas no
  -- [E] seguinte em vez de julgar um registo vazio (ou de ignorar ações até
  -- o relógio alcançar o valor gravado).
  if t0 == nil or t0 > vael.time() then
    return false
  end
  -- O registo de ações também é do save: se ainda tem entradas de outro
  -- processo (t no futuro), esta janela morreu com essa sessão. Limpa-o já e
  -- obriga a um `start` novo — senão o altar julgava um registo vazio assim
  -- que o uptime passasse o `t0` gravado.
  if actions.stale() then
    actions.clear()
    return false
  end
  return true
end

function M.start(id)
  -- Arranque de uma sessão de julgamento num processo novo: o log gravado
  -- no save é de outra linha temporal (uptime recomeçado) e não pode contar.
  if actions.stale() then
    actions.clear()
  end
  vael.game()["judge." .. tostring(id) .. ".t0"] = vael.time()
  vael.game()["judge." .. tostring(id) .. ".done"] = nil
end

function M.reset(id)
  vael.game()["judge." .. tostring(id) .. ".t0"] = nil
  vael.game()["judge." .. tostring(id) .. ".done"] = nil
end

function M.solved(id)
  return vael.game()["judge." .. tostring(id) .. ".done"] == true
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
    if forbidden then
      break
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
  local reason = "missing"
  if forbidden then
    verdict = "wrong"
    reason = "forbidden"
  elseif need == 0 then
    verdict = "wrong"
    reason = "empty"
  elseif got == need and order_ok then
    verdict = "correct"
    reason = "ok"
  elseif got == need then
    verdict = "partial"
    reason = "order"
  elseif got > 0 then
    verdict = "partial"
    reason = "missing"
  end
  return {
    verdict = verdict,
    reason = reason,
    got = got,
    need = need,
    order_ok = order_ok,
    forbidden = forbidden,
    missing = missing,
    matched = matched,
  }
end

-- Corte do registo para a passagem pelo modelo (cada entrada custa uma
-- inferência por requisito). A cauda entra sempre; o resto só se for
-- EXACTAMENTE uma ação que a regra pede ou proíbe — sem isso, uma ação
-- exigida antiga (regra com ordem longa) desaparecia e o veredicto ficava
-- "missing" mesmo com o herói a ter cumprido o voto.
local function cap_entries(rule, entries)
  if #entries <= MAX_PARSE then
    return entries
  end
  local reqs = rule.require or {}
  local forbids = rule.forbid or {}
  local keep = {}
  for i = 1, #entries do
    local e = entries[i]
    for _, req in ipairs(reqs) do
      if (not req.verb or e.verb == req.verb) and e.kind == req.kind and e.id == req.id then
        keep[i] = true
        break
      end
    end
    if not keep[i] then
      for _, f in ipairs(forbids) do
        if e.kind == f.kind and e.id == f.id then
          keep[i] = true
          break
        end
      end
    end
  end
  for i = #entries - MAX_PARSE + 1, #entries do
    keep[i] = true
  end
  local out = {}
  for i, e in ipairs(entries) do
    if keep[i] then
      out[#out + 1] = e
    end
  end
  return out
end

-- Exposto para o harness (`self_test`) provar que o corte não perde ações.
M.cap_entries = cap_entries

-- Lê o registo desde `t0`, manda cada frase ao modelo e julga.
function M.evaluate(id)
  local rule = M.get(id)
  if not rule then
    return nil
  end
  local t0 = tonumber(vael.game()["judge." .. tostring(id) .. ".t0"]) or 0
  local entries = cap_entries(rule, actions.since(t0))
  -- Caminho novo: o d1 diz qual requisito cada ação cumpre. A regra
  -- (ordem/proibições/recompensa) continua simbólica e explicável.
  if decide.ready() then
    local reqs = rule.require or {}
    local verb_label = {
      matar = "matou", visitar = "visitou", equipar = "equipou",
      usar = "usou", comprar = "comprou", vender = "vendeu",
      entoar = "entoou", colher = "colheu", forjar = "forjou",
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
      local cache_key = tostring(id) .. "\0" .. tostring(e.text)
      local answers = decision_cache[cache_key]
      if not answers then
        answers = decide.decide({ action = e.text }, questions) or {}
        decision_cache[cache_key] = answers
      end
      for i, req in ipairs(reqs) do
        local a = answers["r" .. i]
        if a and (tonumber(a.noul) or 0) >= 0.5 then
          e.res = e.res or {}
          e.res[req.kind] = e.res[req.kind] or {}
          e.res[req.kind][req.id] = true
          -- O verb do registo é a verdade (cada ação é gravada com o seu):
          -- um falso positivo do d1 num requisito de OUTRO verb não pode
          -- reescrevê-lo, senão `evaluate_log` casa o requisito errado e a
          -- ordem (`order`) valida com uma marca que nunca aconteceu.
          e.verb = e.verb or req.verb
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
    vael.game()["judge." .. tostring(id) .. ".last"] = result.verdict
    return result
  end

  local model_ready = vael.nlu and vael.nlu.ready and vael.nlu.ready()
  for _, e in ipairs(entries) do
    local parsed = nil
    if model_ready then
      local ok, p = pcall(vael.nlu.parse, e.text)
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
  vael.game()["judge." .. tostring(id) .. ".last"] = result.verdict
  return result
end

function M.reward(id, result)
  local rule = M.get(id)
  if not rule or not result then
    return
  end
  if result.verdict == "correct" then
    vael.game()["judge." .. tostring(id) .. ".done"] = true
    local rewards = rule.reward or {}
    if rewards.xp and rewards.xp > 0 then
      rules.add_xp(rewards.xp)
    end
    if rewards.gold and rewards.gold > 0 then
      rules.vault_add("gold", rewards.gold)
    end
    for _, item in ipairs(rewards.items or {}) do
      local name, count = tostring(item):match("^([^:]+):?(%d*)$")
      if name and name ~= "" then
        rules.item_add(name, tonumber(count) or 1)
      end
    end
    local toast = "Juramento cumprido (+" .. (rewards.xp or 0) .. " XP"
    if rewards.gold and rewards.gold > 0 then
      toast = toast .. ", +" .. rewards.gold .. " ouro"
    end
    toast = toast .. ")"
    vael.toast(toast)
    vael.sound("quest_complete")
  elseif result.verdict == "partial" then
    vael.toast("O altar reconhece parte do juramento.")
    vael.sound("notification")
  else
    vael.sound("error")
  end
end

-- Teste puro do juiz (bridge: `vael.load("lib/judge.lua").self_test()`).
function M.self_test()
  local function e(verb, kind, id)
    return { verb = verb, kind = kind, id = id, res = { [kind] = { [id] = true } } }
  end
  local synthetic_rule = {
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
      rule = synthetic_rule,
      entries = {
        e("matar", "criatura", "wolf"),
        e("visitar", "local", "forest-crossroads-well"),
        e("equipar", "item", "torch"),
      },
      want = "correct",
      want_reason = "ok",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "ordem trocada",
      rule = synthetic_rule,
      entries = {
        e("equipar", "item", "torch"),
        e("visitar", "local", "forest-crossroads-well"),
        e("matar", "criatura", "wolf"),
      },
      want = "partial",
      want_reason = "order",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "falta a tocha",
      rule = synthetic_rule,
      entries = {
        e("matar", "criatura", "wolf"),
        e("visitar", "local", "forest-crossroads-well"),
      },
      want = "partial",
      want_reason = "missing",
      want_got = 2,
      want_need = 3,
    },
    {
      name = "proibido",
      rule = synthetic_rule,
      entries = {
        e("matar", "criatura", "wolf"),
        e("visitar", "local", "forest-crossroads-well"),
        e("equipar", "item", "torch"),
        e("usar", "item", "stone"),
      },
      want = "wrong",
      want_reason = "forbidden",
    },
    {
      name = "nada feito",
      rule = synthetic_rule,
      entries = {},
      want = "wrong",
      want_reason = "missing",
      want_got = 0,
      want_need = 3,
    },
    {
      name = "hunter_oath positivo",
      rule_id = "hunter_oath",
      entries = {
        e("matar", "criatura", "wolf"),
        e("visitar", "local", "forest-crossroads-well"),
        e("equipar", "item", "torch"),
      },
      want = "correct",
      want_reason = "ok",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "hunter_oath ordem errada",
      rule_id = "hunter_oath",
      entries = {
        e("equipar", "item", "torch"),
        e("visitar", "local", "forest-crossroads-well"),
        e("matar", "criatura", "wolf"),
      },
      want = "partial",
      want_reason = "order",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "desert_ascetic positivo",
      rule_id = "desert_ascetic",
      entries = {
        e("visitar", "local", "desert-arch"),
        e("matar", "criatura", "scorpion"),
        e("equipar", "item", "sword"),
      },
      want = "correct",
      want_reason = "ok",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "desert_ascetic forbid potion",
      rule_id = "desert_ascetic",
      entries = {
        e("visitar", "local", "desert-arch"),
        e("matar", "criatura", "scorpion"),
        e("equipar", "item", "sword"),
        e("usar", "item", "potion"),
      },
      want = "wrong",
      want_reason = "forbidden",
    },
    {
      name = "desert_ascetic parcial",
      rule_id = "desert_ascetic",
      entries = {
        e("visitar", "local", "desert-arch"),
      },
      want = "partial",
      want_reason = "missing",
      want_got = 1,
      want_need = 3,
    },
    {
      name = "swamp_purifier positivo",
      rule_id = "swamp_purifier",
      entries = {
        e("visitar", "local", "swamp-shack"),
        e("matar", "criatura", "slime"),
        e("usar", "item", "antidote"),
      },
      want = "correct",
      want_reason = "ok",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "swamp_purifier forbid bandit",
      rule_id = "swamp_purifier",
      entries = {
        e("visitar", "local", "swamp-shack"),
        e("matar", "criatura", "slime"),
        e("usar", "item", "antidote"),
        e("matar", "criatura", "bandit"),
      },
      want = "wrong",
      want_reason = "forbidden",
    },
    {
      name = "swamp_purifier parcial",
      rule_id = "swamp_purifier",
      entries = {
        e("visitar", "local", "swamp-shack"),
        e("matar", "criatura", "slime"),
      },
      want = "partial",
      want_reason = "missing",
      want_got = 2,
      want_need = 3,
    },
    {
      name = "peaks_vigil ordem certa",
      rule_id = "peaks_vigil",
      entries = {
        e("equipar", "item", "torch"),
        e("matar", "criatura", "shade"),
        e("visitar", "local", "peaks-crystal-mine"),
      },
      want = "correct",
      want_reason = "ok",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "peaks_vigil ordem errada",
      rule_id = "peaks_vigil",
      entries = {
        e("matar", "criatura", "shade"),
        e("equipar", "item", "torch"),
        e("visitar", "local", "peaks-crystal-mine"),
      },
      want = "partial",
      want_reason = "order",
      want_got = 3,
      want_need = 3,
    },
    {
      name = "peaks_vigil parcial",
      rule_id = "peaks_vigil",
      entries = {
        e("equipar", "item", "torch"),
        e("visitar", "local", "peaks-crystal-mine"),
      },
      want = "partial",
      want_reason = "missing",
      want_got = 2,
      want_need = 3,
    },
    {
      name = "peaks_vigil nada feito",
      rule_id = "peaks_vigil",
      entries = {},
      want = "wrong",
      want_reason = "missing",
      want_got = 0,
      want_need = 3,
    },
  }
  local errors = {}
  for _, c in ipairs(cases) do
    local rule = c.rule
    if not rule and c.rule_id then
      rule = M.get(c.rule_id)
      if not rule then
        errors[#errors + 1] = c.name .. ": regra ausente (" .. tostring(c.rule_id) .. ")"
      end
    end
    if rule then
      local r = M.evaluate_log(rule, c.entries)
      if r.verdict ~= c.want then
        errors[#errors + 1] = c.name .. ": verdict " .. tostring(r.verdict) .. " ~= " .. tostring(c.want)
      end
      if c.want_reason and r.reason ~= c.want_reason then
        errors[#errors + 1] = c.name .. ": reason " .. tostring(r.reason) .. " ~= " .. tostring(c.want_reason)
      end
      if c.want_got ~= nil and r.got ~= c.want_got then
        errors[#errors + 1] = c.name .. ": got " .. tostring(r.got) .. " ~= " .. tostring(c.want_got)
      end
      if c.want_need ~= nil and r.need ~= c.want_need then
        errors[#errors + 1] = c.name .. ": need " .. tostring(r.need) .. " ~= " .. tostring(c.want_need)
      end
    end
  end
  -- Corte do registo (`cap_entries`): com mais de MAX_PARSE entradas, uma
  -- ação EXIGIDA antiga e uma PROIBIDA antiga têm de sobreviver; as ações
  -- irrelevantes do meio podem sair.
  local long = {
    e("matar", "criatura", "wolf"),
    e("usar", "item", "potion"),
    e("usar", "item", "antidote"),
    e("usar", "item", "moss_potion"),
  }
  for i = 1, 20 do
    long[#long + 1] = e("usar", "item", "stone" .. i)
  end
  long[#long + 1] = e("usar", "item", "stone")
  local cap_rule = {
    require = {
      { kind = "criatura", id = "wolf", verb = "matar" },
    },
    order = { "matar" },
    forbid = { { kind = "item", id = "stone" } },
  }
  local capped = M.cap_entries(cap_rule, long)
  if capped[1] ~= long[1] then
    errors[#errors + 1] = "corte: a ação exigida antiga saiu do registo"
  end
  local last = capped[#capped]
  if last ~= long[#long] then
    errors[#errors + 1] = "corte: a ação proibida antiga saiu do registo"
  end
  local cap_result = M.evaluate_log(cap_rule, capped)
  if cap_result.verdict ~= "wrong" or cap_result.reason ~= "forbidden" then
    errors[#errors + 1] = "corte: proibida antiga não invalidou o voto"
  end
  local only_req = {
    require = { { kind = "criatura", id = "wolf", verb = "matar" } },
    order = { "matar" },
  }
  local capped_req = M.cap_entries(only_req, long)
  if M.evaluate_log(only_req, capped_req).verdict ~= "correct" then
    errors[#errors + 1] = "corte: requisito antigo perdido no veredicto"
  end
  return #errors == 0, errors
end

return M
