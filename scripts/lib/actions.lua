-- Registo de ações do herói em frases curtas.
--
-- É a metade "mundo -> texto" do juiz: cada ação observável vira uma frase
-- ("o herói matou lobo em Floresta Sombria"). O `judge.lua` manda essa frase
-- ao modelo, resolve as entidades e compara com a regra.
--
-- O registo é de SESSÃO (módulo cacheado; sobrevive a trocas de cena, não a
-- um load). O juiz filtra por `t0` (quando o jogador leu a pista), por isso
-- ações antigas não contam.
local M = {}

local MAX = 32
local log = {}
local data

local function load_data()
  if data then
    return
  end
  data = { items = {}, marks = {}, creatures = {} }
  local ok_items, items = pcall(viber.load_json, "data/items.json")
  if ok_items and type(items) == "table" then
    for _, it in ipairs(items.items or {}) do
      data.items[it.id] = it.label
    end
  end
  local ok_marks, marks = pcall(viber.load_json, "data/landmarks.json")
  if ok_marks and type(marks) == "table" then
    for _, m in ipairs(marks.landmarks or {}) do
      data.marks[m.name] = m.label
    end
  end
  local ok_alias, alias = pcall(viber.load_json, "data/nlu_aliases.json")
  if ok_alias and type(alias) == "table" then
    for id, list in pairs(alias.creatures or {}) do
      data.creatures[id] = list[1] or id
    end
    for id, list in pairs(alias.items or {}) do
      if not data.items[id] then
        data.items[id] = list[1] or id
      end
    end
    for id, list in pairs(alias.places or {}) do
      if not data.marks[id] then
        data.marks[id] = list[1] or id
      end
    end
  end
end

local function place()
  local zone = ""
  if viber.ui and viber.ui.get then
    zone = viber.ui.get("zone.name") or ""
  end
  if type(zone) ~= "string" or zone == "" then
    return "o vale"
  end
  return zone
end

local function sentence(verb, kind, id)
  load_data()
  local where = place()
  local item = data.items[id] or id
  local creature = data.creatures[id] or id
  local landmark = data.marks[id] or id
  if verb == "matar" then
    return "o herói matou " .. creature .. " em " .. where
  elseif verb == "visitar" then
    return "o herói visitou " .. landmark
  elseif verb == "equipar" then
    return "o herói equipou " .. item .. " em " .. where
  elseif verb == "usar" then
    return "o herói usou " .. item .. " em " .. where
  elseif verb == "comprar" then
    return "o herói comprou " .. item .. " em " .. where
  elseif verb == "vender" then
    return "o herói vendeu " .. item .. " em " .. where
  end
  return "o herói agiu em " .. where
end

-- verb: matar|visitar|equipar|usar|comprar|vender
-- kind: criatura|local|item (o tipo do id, para o fallback sem modelo)
function M.record(verb, kind, id)
  if not verb or not id or id == "" then
    return
  end
  log[#log + 1] = {
    verb = verb,
    kind = kind,
    id = id,
    text = sentence(verb, kind, id),
    t = viber.time(),
  }
  if #log > MAX then
    table.remove(log, 1)
  end
end

function M.since(t0)
  local out = {}
  for _, e in ipairs(log) do
    if e.t >= (t0 or 0) then
      out[#out + 1] = e
    end
  end
  return out
end

function M.all()
  return log
end

function M.clear()
  log = {}
end

return M
