-- Resolvedor de linguagem do NLU: transforma o texto do jogador (e as
-- entidades que o modelo extraiu) nos IDs do jogo.
--
-- O modelo devolve superfícies ("lobos", "poço", "seis pedras") e rótulos
-- ruidosos (por vezes "sombra" vem como item). Aqui normalizamos, casamos
-- contra os aliases autorais e devolvemos conjuntos de IDs por tipo:
--   { item = {wood=true}, criatura = {wolf=true}, local = {...},
--     quantidade = 6, raw = "seis pedras" }
--
-- `match_expect(res, expect)` é o que as quests usam para validar o relato.
local M = {}

local index      -- kind -> { alias_normalizado -> { id = true } }
local numbers    -- palavra_normalizada -> inteiro

local function strip(s)
  s = tostring(s or ""):lower()
  s = s:gsub("á", "a"):gsub("à", "a"):gsub("â", "a"):gsub("ã", "a"):gsub("ä", "a")
  s = s:gsub("é", "e"):gsub("è", "e"):gsub("ê", "e"):gsub("ë", "e")
  s = s:gsub("í", "i"):gsub("ì", "i"):gsub("î", "i"):gsub("ï", "i")
  s = s:gsub("ó", "o"):gsub("ò", "o"):gsub("ô", "o"):gsub("õ", "o"):gsub("ö", "o")
  s = s:gsub("ú", "u"):gsub("ù", "u"):gsub("û", "u"):gsub("ü", "u")
  s = s:gsub("ç", "c")
  -- Pontuação vira espaço: "pedra," tem de casar com o alias "pedra".
  s = s:gsub("%p", " ")
  s = s:gsub("[-_]", " ")
  s = s:gsub("%s+", " ")
  return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function singular(w)
  if #w > 4 and w:sub(-3) == "oes" then
    return w:sub(1, #w - 3) .. "ao"
  end
  if #w > 4 and w:sub(-3) == "aes" then
    return w:sub(1, #w - 3) .. "ao"
  end
  if #w > 3 and w:sub(-2) == "is" then
    return w:sub(1, #w - 2) .. "il"
  end
  if #w > 3 and w:sub(-1) == "s" then
    return w:sub(1, #w - 1)
  end
  return w
end

local function tokens(s)
  local out = {}
  for w in s:gmatch("%S+") do
    out[#out + 1] = w
  end
  return out
end

local function add(kind, id, alias)
  local a = strip(alias)
  if a == "" then
    return
  end
  index[kind][a] = index[kind][a] or {}
  index[kind][a][id] = true
  local sg = singular(a)
  if sg ~= a and sg ~= "" then
    index[kind][sg] = index[kind][sg] or {}
    index[kind][sg][id] = true
  end
end

local function build()
  if index then
    return
  end
  index = { item = {}, criatura = {}, ["local"] = {} }
  numbers = {}

  local ok_alias, alias = pcall(viber.load_json, "data/nlu_aliases.json")
  if not ok_alias or type(alias) ~= "table" then
    -- Sem aliases o resolver degrada para os labels autorais em silêncio:
    -- o relato de 1 palavra das quests deixava de funcionar sem explicação.
    viber.log("parley: falha a ler data/nlu_aliases.json — só labels autorais"
      .. (ok_alias and "" or (" (" .. tostring(alias) .. ")")))
    alias = {}
  end
  for word, value in pairs(alias.numbers or {}) do
    numbers[strip(word)] = tonumber(value)
  end
  -- `type(list) == "table"`: um alias com valor escalar (edição à mão)
  -- rebentava o `ipairs` aqui e o erro subia pelo `resolve` para sempre.
  for id, list in pairs(alias.creatures or {}) do
    if type(list) == "table" then
      add("criatura", id, id)
      for _, a in ipairs(list) do
        add("criatura", id, a)
      end
    end
  end
  for id, list in pairs(alias.items or {}) do
    if type(list) == "table" then
      add("item", id, id)
      for _, a in ipairs(list) do
        add("item", id, a)
      end
    end
  end
  for id, list in pairs(alias.places or {}) do
    if type(list) == "table" then
      add("local", id, id)
      for _, a in ipairs(list) do
        add("local", id, a)
      end
    end
  end

  -- Labels autorais: itens e marcos entram sempre, mesmo que o alias mude.
  local ok_items, items = pcall(viber.load_json, "data/items.json")
  if ok_items and type(items) == "table" then
    for _, it in ipairs(items.items or {}) do
      add("item", it.id, it.label)
    end
  end
  local ok_marks, marks = pcall(viber.load_json, "data/landmarks.json")
  if ok_marks and type(marks) == "table" then
    for _, mk in ipairs(marks.landmarks or {}) do
      add("local", mk.name, mk.name)
      add("local", mk.name, mk.label)
    end
  end
end

local function find_in(kind, text)
  local out = {}
  local norm = strip(text)
  if norm == "" or not index[kind] then
    return out
  end
  for id in pairs(index[kind][norm] or {}) do
    out[id] = true
  end
  local toks = tokens(norm)
  for a, ids in pairs(index[kind]) do
    local at = tokens(a)
    local n = #at
    if n > 0 and n <= #toks then
      for i = 1, #toks - n + 1 do
        local hit = true
        for j = 1, n do
          if toks[i + j - 1] ~= at[j] then
            hit = false
            break
          end
        end
        if hit then
          for id in pairs(ids) do
            out[id] = true
          end
          break
        end
      end
    end
  end
  return out
end

local function find_number(text)
  local norm = strip(text)
  local digits = norm:match("%d+")
  if digits then
    return tonumber(digits)
  end
  for w in norm:gmatch("%S+") do
    if numbers[w] then
      return numbers[w]
    end
  end
  return nil
end

local function merge(dst, src)
  for id in pairs(src) do
    dst[id] = true
  end
end

-- Resolve o texto cru E as entidades do modelo. O texto cru é essencial:
-- substantivos isolados ("nota", "poço") às vezes não são extraídos.
function M.resolve(text, parsed)
  build()
  local res = {
    item = {}, criatura = {}, ["local"] = {},
    quantidade = find_number(text),
    raw = strip(text),
  }
  merge(res.item, find_in("item", text))
  merge(res.criatura, find_in("criatura", text))
  merge(res["local"], find_in("local", text))

  if type(parsed) == "table" and type(parsed.entities) == "table" then
    local by_label = {
      item = "item", produto = "item",
      criatura = "criatura", animal = "criatura",
      ["local"] = "local", lugar = "local",
    }
    for _, e in ipairs(parsed.entities) do
      local kind = by_label[e.label]
      local found = false
      if kind then
        local set = find_in(kind, e.text)
        if next(set) ~= nil then
          merge(res[kind], set)
          found = true
        end
      end
      -- Rótulo ruidoso: tenta todos os tipos antes de desistir.
      if not found then
        merge(res.item, find_in("item", e.text))
        merge(res.criatura, find_in("criatura", e.text))
        merge(res["local"], find_in("local", e.text))
      end
      if not res.quantidade and (e.label == "quantidade" or e.label == "numero") then
        res.quantidade = find_number(e.text)
      end
    end
    if not res.quantidade then
      res.quantidade = find_number(text)
    end
  end
  return res
end

-- expect = { {kind="criatura", id="wolf"}, ... }. Passa se QUALQUER um casar
-- (visitas têm 3 destinos possíveis).
function M.match_expect(res, expect)
  if type(expect) ~= "table" then
    return false, nil
  end
  for _, e in ipairs(expect) do
    local set = res[e.kind]
    if type(set) == "table" and set[e.id] then
      return true, e
    end
  end
  return false, nil
end

function M.describe(res)
  local out = {}
  for _, kind in ipairs({ "criatura", "item", "local" }) do
    local ids = {}
    for id in pairs(res[kind] or {}) do
      ids[#ids + 1] = id
    end
    table.sort(ids)
    out[#out + 1] = kind .. "=" .. table.concat(ids, ",")
  end
  if res.quantidade then
    out[#out + 1] = "n=" .. tostring(res.quantidade)
  end
  return table.concat(out, " ")
end

-- Teste rápido (chamável pelo bridge: `viber.load("lib/parley.lua").self_test()`).
function M.self_test()
  build()
  local cases = {
    { "lobos", { criatura = "wolf" } },
    { "matei três lobos", { criatura = "wolf" }, 3 },
    { "pedra", { item = "stone" } },
    { "seis pedras", { item = "stone" }, 6 },
    { "lenha", { item = "wood" } },
    { "poço da encruzilhada", { ["local"] = "forest-crossroads-well" } },
    { "mojões", { ["local"] = "peaks-cairn-1" } },
    { "barca encalhada", { ["local"] = "swamp-wrecked-boat" } },
    { "sombra", { criatura = "shade" } },
    { "verme das areias", { criatura = "sand-worm" } },
    { "bruxa", { criatura = "witch" } },
    { "musgo", { item = "bog-moss" } },
    { "nota", { item = "nota" } },
  }
  local errors = {}
  for _, c in ipairs(cases) do
    local res = M.resolve(c[1], nil)
    local ok = true
    for kind, id in pairs(c[2]) do
      if not (res[kind] and res[kind][id]) then
        ok = false
        errors[#errors + 1] = c[1] .. ": falta " .. kind .. "=" .. id
      end
    end
    if c[3] and res.quantidade ~= c[3] then
      errors[#errors + 1] = c[1] .. ": quantidade " .. tostring(res.quantidade) .. " ~= " .. c[3]
    end
    if ok and c[3] == nil then
      -- ok
    end
  end
  return #errors == 0, errors
end

return M
