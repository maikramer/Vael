-- lib/craft.lua: a FORJA — receitas em `data/recipes.json`, regra em Lua.
--
-- Uma receita é `{ id, out, count, cost = { item = n, … } }`; o ouro entra no
-- custo como qualquer outro recurso. `craft.make(id)` verifica o custo e o
-- espaço na mochila ANTES de tirar o que quer que seja, e devolve o que tirou
-- se o produto não couber. A bigorna (`scripts/anvil.lua`) abre o separador
-- Forja do diário; a lista `forge` é montada aqui.

local rules = viber.load("lib/rules.lua")
local actions = viber.load("lib/actions.lua")

local M = {}

local RECIPES, BY_ID = {}, {}
do
  local ok, data = pcall(viber.load_json, "data/recipes.json")
  if ok and type(data) == "table" then
    for _, r in ipairs(data.recipes or {}) do
      if type(r) == "table" and type(r.id) == "string" and r.id ~= ""
          and type(r.out) == "string" and type(r.cost) == "table" and not BY_ID[r.id] then
        BY_ID[r.id] = r
        RECIPES[#RECIPES + 1] = r
      end
    end
  else
    viber.log("forja: falha a ler data/recipes.json")
  end
end

local function sfx(name)
  local has, x, y, z = viber.player_position()
  if has then
    viber.sound(name, { x = x, y = y, z = z })
  else
    viber.sound(name)
  end
end

-- Custo ordenado (texto estável: a lista não pisca entre frames).
local function cost_list(r)
  local out = {}
  for item, n in pairs(r.cost) do
    n = tonumber(n)
    if type(item) == "string" and n and n > 0 then
      out[#out + 1] = { item = item, n = math.floor(n) }
    end
  end
  table.sort(out, function(a, b) return a.item < b.item end)
  return out
end

local function cap_of(id)
  local def = rules.item_def(id)
  local stack = def and tonumber(def.stack)
  if stack and stack > 0 then
    return math.floor(stack)
  end
  return rules.stack_max()
end

function M.recipes()
  return RECIPES
end

-- "ok" | "missing" (falta material) | "full" (não cabe na mochila).
function M.status(r)
  for _, c in ipairs(cost_list(r)) do
    if rules.vault_get(c.item) < c.n then
      return "missing"
    end
  end
  local count = math.max(1, math.floor(tonumber(r.count) or 1))
  if rules.vault_get(r.out) + count > cap_of(r.out) then
    return "full"
  end
  return "ok"
end

function M.make(id)
  local r = BY_ID[id]
  if not r then
    return false
  end
  local status = M.status(r)
  if status == "missing" then
    viber.toast("Falta material para forjar isso.")
    sfx("error")
    return false
  elseif status == "full" then
    viber.toast("Já não cabe mais disso na mochila.")
    sfx("error")
    return false
  end
  local costs = cost_list(r)
  local taken = {}
  for _, c in ipairs(costs) do
    if not rules.vault_take(c.item, c.n) then
      -- Não devia acontecer (o `status` acabou de confirmar); devolve tudo.
      for _, t in ipairs(taken) do
        rules.vault_add(t.item, t.n)
      end
      viber.toast("Falta material para forjar isso.")
      sfx("error")
      return false
    end
    taken[#taken + 1] = c
  end
  local count = math.max(1, math.floor(tonumber(r.count) or 1))
  local got = rules.item_add(r.out, count) or 0
  if got < count then
    if got > 0 then
      rules.vault_take(r.out, got)
    end
    for _, t in ipairs(taken) do
      rules.vault_add(t.item, t.n)
    end
    viber.toast("Já não cabe mais disso na mochila.")
    sfx("error")
    return false
  end
  actions.record("forjar", "item", r.out)
  rules.report_craft(r.out, count)
  rules.add_xp(10)
  sfx("mine_hit")
  viber.toast("Forjado: " .. rules.item_label(r.out) .. (count > 1 and (" ×" .. count) or "") .. ".")
  return true
end

local STATUS_TEXT = { ok = "forjar", missing = "falta material", full = "mochila cheia" }
local last_sig

function M.refresh()
  local rows, parts = {}, {}
  for i, r in ipairs(RECIPES) do
    local status = M.status(r)
    local cost = {}
    for _, c in ipairs(cost_list(r)) do
      cost[#cost + 1] = c.n .. "× " .. rules.item_label(c.item)
        .. " (" .. rules.vault_get(c.item) .. ")"
    end
    local def = rules.item_def(r.out)
    local count = math.max(1, math.floor(tonumber(r.count) or 1))
    rows[i] = {
      id = r.id,
      label = rules.item_label(r.out) .. (count > 1 and (" ×" .. count) or ""),
      cost = table.concat(cost, " · "),
      note = def and def.desc or "",
      status = status,
      status_text = STATUS_TEXT[status],
    }
    parts[i] = r.id .. "\2" .. status .. "\2" .. rows[i].cost
  end
  local sig = tostring(viber.game().lang) .. "\1" .. table.concat(parts, "\1")
  if sig == last_sig then
    return
  end
  last_sig = sig
  viber.ui.list("forge", rows)
end

return M
