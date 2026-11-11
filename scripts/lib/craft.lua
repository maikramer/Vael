-- lib/craft.lua: a FORJA — a regra vive em `src/economy.rs`; aqui fica a UI.
--
-- Uma receita é `{ id, out, count, cost = { item = n, … } }`; o ouro entra no
-- custo como qualquer outro recurso. O Rust (`viber.vael.craft_status`/
-- `craft_make`) verifica o custo e o espaço na mochila ANTES de tirar o que
-- quer que seja, e devolve o que tirou se o produto não couber. A bigorna
-- (`scripts/anvil.lua`) abre o separador Forja do diário; a lista `forge` é
-- montada aqui.

local rules = viber.load("lib/rules.lua")
local actions = viber.load("lib/actions.lua")

local M = {}

-- Receitas normalizadas em Rust (ids únicos, custos positivos ordenados).
local RECIPES = viber.vael.craft_recipes()

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
  for item, n in pairs(r.cost or {}) do
    n = tonumber(n)
    if type(item) == "string" and n and n > 0 then
      out[#out + 1] = { item = item, n = math.floor(n) }
    end
  end
  table.sort(out, function(a, b) return a.item < b.item end)
  return out
end

function M.recipes()
  return RECIPES
end

-- "ok" | "missing" (falta material) | "full" (não cabe na mochila).
function M.status(r)
  if type(r) ~= "table" or type(r.id) ~= "string" or r.id == "" then
    return "missing"
  end
  return viber.vael.craft_status(r.id) or "missing"
end

function M.make(id)
  local result = viber.vael.craft_make(id)
  if type(result) ~= "table" then
    return false
  end
  if not result.ok then
    if result.reason == "missing" then
      viber.toast("Falta material para forjar isso.")
    elseif result.reason == "full" then
      viber.toast("Já não cabe mais disso na mochila.")
    else
      -- Receita desconhecida: o Lua antigo saía em silêncio.
      return false
    end
    sfx("error")
    return false
  end
  local out, count = result.output, result.count
  -- O cofre mudou diretamente em Rust: as quests de recolha têm de rever o
  -- progresso (o antigo `rules.vault_take/item_add` fazia-o em cada passo).
  rules.sync_collect()
  actions.record("forjar", "item", out)
  rules.report_craft(out, count)
  rules.add_xp(10)
  sfx("mine_hit")
  viber.toast("Forjado: " .. rules.item_label(out) .. (count > 1 and (" ×" .. count) or "") .. ".")
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
