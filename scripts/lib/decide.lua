-- Ponte entre os scripts e a `viber.decision` (d1-omni).
--
-- Constrói perguntas tipadas (`choice`/`noul`/`score`) e resolve rótulos
-- legíveis para os critérios. O modelo é um classificador semântico: a
-- pergunta e as opções têm de estar na língua do estado (PT neste jogo).
local M = {}

local DIR = "assets/public/d1"
local rules = viber.load("lib/rules.lua")
local marks = viber.load("lib/landmarks.lua")
local aliases

function M.available()
  return viber.decision and viber.decision.available
    and viber.decision.available(DIR)
end

function M.ready()
  return viber.decision and viber.decision.ready and viber.decision.ready()
end

function M.load()
  if M.available() then
    viber.decision.load(DIR)
  end
end

function M.noul(instructions, false_text, true_text)
  return {
    type = "noul",
    instructions = instructions,
    criteria = { ["false"] = false_text or "não", ["true"] = true_text or "sim" },
  }
end

function M.choice(instructions, options)
  return { type = "choice", instructions = instructions, criteria = options }
end

function M.score(instructions, levels)
  return { type = "score", instructions = instructions, criteria = levels }
end

-- Rótulo humano de uma entidade, para as descrições das opções.
-- `torch`/`sword` são kinds de equipamento (game/equipment.lua) e não vivem em
-- `data/items.json`: sem o fallback para o primeiro alias o juiz perguntava
-- "o herói equipou torch?" sobre uma ação registada como "equipou tocha".
local function alias_table()
  if not aliases then
    local ok, data = pcall(viber.load_json, "data/nlu_aliases.json")
    aliases = ok and type(data) == "table" and data or {}
  end
  return aliases
end

function M.label(kind, id)
  if kind == "item" then
    local label = rules.item_label(id)
    if label ~= id then
      return label
    end
    local list = alias_table().items and alias_table().items[id]
    return (list and list[1]) or id
  elseif kind == "local" then
    return marks.label(id)
  elseif kind == "criatura" then
    local list = alias_table().creatures and alias_table().creatures[id]
    return (list and list[1]) or id
  end
  return id
end

-- Uma pergunta `noul` por item esperado; as chaves são r1..rN.
function M.expect_questions(expect, _prompt)
  local qs = {}
  for i, e in ipairs(expect or {}) do
    local label = M.label(e.kind, e.id)
    qs["r" .. i] = M.noul(
      "O relato do jogador menciona " .. label .. "?",
      "não menciona " .. label,
      "menciona " .. label
    )
  end
  return qs
end

function M.expect_matches(answers, expect)
  for i = 1, #(expect or {}) do
    local a = answers and answers["r" .. i]
    if not a or (tonumber(a.noul) or 0) < 0.5 then
      return false, i
    end
  end
  return true, nil
end

-- Bloqueante (QA/eventos raros, como o juiz). Devolve o mapa `answers`.
function M.decide(state, questions)
  if not M.ready() then
    return nil, "viber.decision não está carregado"
  end
  local result, err = viber.decision.decide(state or {}, questions or {})
  if not result then
    return nil, err
  end
  return result.answers or result, nil
end

return M
