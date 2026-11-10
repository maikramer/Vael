-- lib/gear.lua: EQUIPAMENTO do herói — o que os itens fazem.
--
-- Os dados vivem em `data/items.json`: `slot` (weapon | armor | charm),
-- `effects` (somados às passivas dos talentos), `weapon` (números do melee) e
-- `use` (consumível). O que está equipado vive no save como `gear.<slot>` =
-- id do item; um item que saiu da mochila (vendido, entregue) deixa de contar
-- no mesmo frame — a mochila é a fonte da verdade.
--
-- Módulo PURO: não carrega `lib/rules.lua` (é o `rules` que o carrega), por
-- isso lê `viber.game()` directamente.

local M = {}

M.SLOTS = { "weapon", "armor", "charm" }
M.SLOT_LABEL = { weapon = "Arma", armor = "Armadura", charm = "Amuleto" }

-- A espada do cartógrafo: a arma de quem não equipou nenhuma.
M.DEFAULT_WEAPON = {
  id = "", label = "Espada", damage = 25, speed = 1.4, range = 3.0, grip = "espada",
}

local ITEMS = {}
do
  local ok, data = pcall(viber.load_json, "data/items.json")
  if ok and type(data) == "table" then
    for _, item in ipairs(data.items or {}) do
      if type(item) == "table" and type(item.id) == "string" and item.id ~= "" then
        ITEMS[item.id] = item
      end
    end
  end
end

local function held(id)
  local n = viber.game()["item." .. id]
  return type(n) == "number" and n == n and n > 0
end

function M.def(id)
  return ITEMS[id]
end

function M.slot_of(id)
  local def = ITEMS[id]
  local slot = def and def.slot
  if slot == "weapon" or slot == "armor" or slot == "charm" then
    return slot
  end
  return nil
end

-- Id equipado no slot, ou nil. Um item que já não está na mochila não conta
-- (e a marca é limpa, para o diário não mostrar um equipado fantasma).
function M.equipped(slot)
  local key = "gear." .. slot
  local id = viber.game()[key]
  if type(id) ~= "string" or id == "" then
    return nil
  end
  if M.slot_of(id) ~= slot or not held(id) then
    viber.game()[key] = nil
    return nil
  end
  return id
end

function M.is_equipped(id)
  local slot = M.slot_of(id)
  return slot ~= nil and M.equipped(slot) == id
end

-- Equipa (ou tira, se já estava). Devolve "on" | "off" | nil (não equipável).
function M.toggle(id)
  local slot = M.slot_of(id)
  if not slot or not held(id) then
    return nil
  end
  local key = "gear." .. slot
  if viber.game()[key] == id then
    viber.game()[key] = nil
    return "off"
  end
  viber.game()[key] = id
  return "on"
end

-- Soma dos `effects` de tudo o que está equipado: { chave = total }.
function M.effects()
  local out = {}
  for _, slot in ipairs(M.SLOTS) do
    local id = M.equipped(slot)
    local eff = id and ITEMS[id].effects
    if type(eff) == "table" then
      for key, value in pairs(eff) do
        if type(value) == "number" and value == value then
          out[key] = (out[key] or 0) + value
        end
      end
    end
  end
  return out
end

-- Números do melee da arma equipada (ou da espada base).
function M.weapon()
  local id = M.equipped("weapon")
  local def = id and ITEMS[id]
  local w = def and def.weapon
  if type(w) ~= "table" then
    return M.DEFAULT_WEAPON
  end
  local base = M.DEFAULT_WEAPON
  return {
    id = id,
    label = def.label or id,
    damage = tonumber(w.damage) or base.damage,
    speed = tonumber(w.speed) or base.speed,
    range = tonumber(w.range) or base.range,
    grip = type(w.grip) == "string" and w.grip or base.grip,
  }
end

return M
