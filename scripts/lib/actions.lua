-- Registo de ações do herói em frases curtas.
--
-- É a metade "mundo -> texto" do juiz: cada ação observável vira uma frase
-- ("o herói matou lobo em Floresta Sombria"). O `judge.lua` manda essa frase
-- ao modelo, resolve as entidades e compara com a regra.
--
-- O registo vive no store PARTILHADO (`viber.game()`), não num módulo local:
-- no BMS cada script tem a sua VM/contexto, por isso um log de módulo só via
-- as ações do próprio script (o juiz não via as de `equipment.lua`/`combat.lua`
-- e o Rust da UI não conseguia escrever nenhuma). Só primitivas — é o que o
-- save guarda:
--   act.seq          próximo n (0 = log vazio)
--   act.first        primeira entrada ainda viva
--   act.<n>.v        verbo (matar|visitar|equipar|usar|comprar|vender|entoar|colher|forjar)
--   act.<n>.k        kind (criatura|local|item|…), opcional
--   act.<n>.i        id
--   act.<n>.t        segundos de `viber.time()` (uptime do PROCESSO)
--
-- O Rust da UI escreve as mesmas chaves (equipar/visitar); o leitor aceita
-- uma entrada sem `t` e carimba-a agora. O `t` é do uptime, que recomeça do
-- zero no arranque — por isso entradas com `t` no futuro são de um processo
-- anterior e saem do registo (ver `since`/`stale`).
local M = {}

local MAX = 32
local data

local function game()
  return viber.game()
end

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

local function label_for(kind, id)
  if kind == "criatura" then
    return data.creatures[id] or id
  elseif kind == "local" then
    return data.marks[id] or id
  elseif kind == "item" then
    return data.items[id] or id
  end
  return data.items[id] or data.creatures[id] or data.marks[id] or id
end

local function render(verb, kind, id, where)
  load_data()
  local w = (type(where) == "string" and where ~= "") and where or place()
  local item = data.items[id] or id
  local creature = data.creatures[id] or id
  local landmark = data.marks[id] or id
  local label = label_for(kind, id)
  if verb == "matar" then
    return "o herói matou " .. creature .. " em " .. w
  elseif verb == "visitar" then
    return "o herói visitou " .. landmark
  elseif verb == "equipar" then
    return "o herói equipou " .. item .. " em " .. w
  elseif verb == "usar" then
    return "o herói usou " .. item .. " em " .. w
  elseif verb == "comprar" then
    return "o herói comprou " .. item .. " em " .. w
  elseif verb == "vender" then
    return "o herói vendeu " .. item .. " em " .. w
  elseif verb == "entoar" then
    return "o herói entoou " .. label .. " em " .. w
  elseif verb == "colher" then
    return "o herói colheu " .. label .. " em " .. w
  end
  return "o herói agiu em " .. w
end

M.render = render

local function key(n, field)
  return "act." .. n .. "." .. field
end

-- Apaga as quatro chaves de uma entrada (primitivas; `nil` remove do store).
local function forget(g, n)
  g[key(n, "v")] = nil
  g[key(n, "k")] = nil
  g[key(n, "i")] = nil
  g[key(n, "t")] = nil
end

-- Cursor do registo. Um save editado à mão com um número fraccionário (ou
-- negativo) não pode gerar chaves "act.3.5.v": normaliza para inteiro ≥ 0.
local function bounds(g)
  local seq = math.floor(tonumber(g["act.seq"]) or 0)
  local first = math.floor(tonumber(g["act.first"]) or 0)
  if seq < 0 then
    seq = 0
  end
  if first < 0 then
    first = 0
  end
  if first > seq then
    first = seq
  end
  return seq, first
end

-- Poda por idade: no máximo MAX entradas vivas (as mais recentes).
local function prune(g, first, seq)
  while seq - first > MAX do
    forget(g, first)
    first = first + 1
  end
  return first
end

-- verb: matar|visitar|equipar|usar|comprar|vender|entoar|colher|forjar
-- kind: criatura|local|item (o tipo do id, para o fallback sem modelo)
function M.record(verb, kind, id)
  if not verb or not id or id == "" then
    return
  end
  local g = game()
  local seq, first = bounds(g)
  first = prune(g, first, seq)
  local n = seq
  g[key(n, "v")] = verb
  if type(kind) == "string" and kind ~= "" then
    g[key(n, "k")] = kind
  else
    g[key(n, "k")] = nil
  end
  g[key(n, "i")] = id
  g[key(n, "t")] = viber.time()
  g["act.seq"] = n + 1
  first = prune(g, first, n + 1)
  g["act.first"] = first
end

-- Há entradas de um processo anterior? O `viber.time()` é o uptime do
-- processo e recomeça do zero no arranque, por isso uma entrada gravada no
-- save fica com `t` no futuro. O juiz limpa o registo no arranque da sessão.
function M.stale()
  local g = game()
  local now = viber.time()
  local seq, first = bounds(g)
  for n = first, seq - 1 do
    local t = tonumber(g[key(n, "t")])
    if t and t > now then
      return true
    end
  end
  return false
end

function M.since(t0)
  t0 = tonumber(t0) or 0
  local g = game()
  local now = viber.time()
  local seq, first = bounds(g)
  first = prune(g, first, seq)
  -- As entradas de outro processo formam um prefixo do registo (as novas
  -- começam sempre depois da última gravada). Saem já: não contam para esta
  -- janela nem ficam a crescer no save até à poda dos 32.
  while first < seq do
    local t = tonumber(g[key(first, "t")])
    if not t then
      -- Entrada escrita por fora (Rust) sem `t`: carimba agora.
      t = now
      g[key(first, "t")] = now
    end
    if t > now then
      forget(g, first)
      first = first + 1
    else
      break
    end
  end
  local out = {}
  for n = first, seq - 1 do
    local verb = g[key(n, "v")]
    local id = g[key(n, "i")]
    if type(verb) ~= "string" or type(id) ~= "string" or id == "" then
      -- Entrada incompleta (poda/escrita a meio, save editado): sai.
      forget(g, n)
    else
      local kind = g[key(n, "k")]
      if type(kind) ~= "string" or kind == "" then
        kind = nil
      end
      local t = tonumber(g[key(n, "t")])
      if not t then
        t = now
        g[key(n, "t")] = now
      end
      if t >= t0 and t <= now then
        out[#out + 1] = {
          verb = verb,
          kind = kind,
          id = id,
          t = t,
          text = render(verb, kind, id),
        }
      end
    end
  end
  g["act.first"] = first
  return out
end

function M.all()
  return M.since(0)
end

function M.clear()
  local g = game()
  -- Apaga por prefixo em vez de varrer `first..seq`: um save corrompido com
  -- um cursor enorme (1e9) não pode prender o [E] num ciclo de mil milhões.
  local stale = {}
  for k in pairs(g) do
    if type(k) == "string" and k:sub(1, 4) == "act." then
      stale[#stale + 1] = k
    end
  end
  for _, k in ipairs(stale) do
    g[k] = nil
  end
  g["act.seq"] = 0
  g["act.first"] = 0
end

return M
