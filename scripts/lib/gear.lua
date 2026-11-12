-- lib/gear.lua: EQUIPAMENTO do herói — o que os itens fazem.
--
-- Os dados vivem em `data/items.json`: `slot` (weapon | armor | charm),
-- `effects` (somados às passivas dos talentos), `weapon` (números do melee) e
-- `use` (consumível). O que está equipado vive no save como `gear.<slot>` =
-- id do item; um item que saiu da mochila (vendido, entregue) deixa de contar
-- no mesmo frame — a mochila é a fonte da verdade.
--
-- O estado e as somas vivem em `src/gear.rs` (`viber.vael.gear_*`). Este
-- módulo guarda os metadados para a UI e os embrulhos finos.
--
-- Módulo PURO: não carrega `lib/rules.lua` (é o `rules` que o carrega).

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

function M.def(id)
  return ITEMS[id]
end

-- Slot equipável (`weapon | armor | charm`), ou nil.
function M.slot_of(id)
  return viber.vael.gear_slot_of(id)
end

-- Id equipado no slot, ou nil. Um item que já não está na mochila não conta
-- (a marca órfã é limpa do save).
function M.equipped(slot)
  return viber.vael.gear_equipped(slot)
end

function M.is_equipped(id)
  return viber.vael.gear_is_equipped(id)
end

-- Equipa (ou tira, se já estava). Devolve "on" | "off" | "blocked" | nil
-- (não equipável/ausente).
function M.toggle(id)
  return viber.vael.gear_toggle(id)
end

-- Requisito de atributos do item (expressão do gauge, ex. "Strength >= 7"),
-- texto curto para o diário ("Força 7") e se está satisfeito agora.
function M.requirement(id)
  return viber.vael.gear_requirement(id)
end

function M.requirement_text(id)
  return viber.vael.gear_requirement_text(id)
end

function M.requirement_met(id)
  return viber.vael.gear_requirement_met(id) == true
end

-- Soma dos `effects` de tudo o que está equipado: { chave = total }.
function M.effects()
  return viber.vael.gear_effects()
end

-- Números do melee da arma equipada (ou da espada base).
function M.weapon()
  return viber.vael.gear_weapon()
end

return M
