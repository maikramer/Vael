-- Marcos da Nota. Os nomes são entidades do mundo; a posição vem de
-- `viber.find`. A engine não tem comportamento de marco.

local M = {}

M.CAMPFIRE_RANGE = 14

M.BIOME_LABEL = {
  tower = "atalaia",
  dark_forest = "bosque",
  desert = "deserto",
  swamp = "pântano",
  frozen_peaks = "picos",
}

-- name, biome, label, radius, survey (quest de visita; "" nas atalaias).
M.LIST = {
  { name = "atalaia-do-vale", biome = "tower", label = "Atalaia do Vale", radius = 8 },
  { name = "atalaia-do-bosque", biome = "tower", label = "Atalaia do Bosque", radius = 8 },
  { name = "atalaia-do-ermo", biome = "tower", label = "Atalaia do Ermo", radius = 8 },
  { name = "atalaia-do-pantano", biome = "tower", label = "Atalaia do Pântano", radius = 8 },
  { name = "atalaia-dos-picos", biome = "tower", label = "Atalaia dos Picos", radius = 8 },
  { name = "forest-outpost-tower", biome = "dark_forest", label = "Torre do Posto Avançado", radius = 10, survey = "forest_survey" },
  { name = "forest-crossroads-well", biome = "dark_forest", label = "Poço da Encruzilhada", radius = 10, survey = "forest_survey" },
  { name = "forest-stone-circle", biome = "dark_forest", label = "Círculo de Menires", radius = 10, survey = "forest_survey" },
  { name = "desert-arch", biome = "desert", label = "Arco do Deserto", radius = 12, survey = "desert_survey" },
  { name = "desert-caravan-wreck", biome = "desert", label = "Caravana Encalhada", radius = 12, survey = "desert_survey" },
  { name = "desert-sun-obelisk", biome = "desert", label = "Obelisco do Sol", radius = 12, survey = "desert_survey" },
  { name = "swamp-wrecked-boat", biome = "swamp", label = "Barco Naufragado", radius = 11, survey = "swamp_survey" },
  { name = "swamp-sunken-graves", biome = "swamp", label = "Covas Submersas", radius = 11, survey = "swamp_survey" },
  { name = "swamp-bone-altar", biome = "swamp", label = "Altar de Ossos", radius = 11, survey = "swamp_survey" },
  { name = "peaks-cairn-1", biome = "frozen_peaks", label = "Primeiro Mojão", radius = 9, survey = "peaks_survey" },
  { name = "peaks-cairn-2", biome = "frozen_peaks", label = "Segundo Mojão", radius = 9, survey = "peaks_survey" },
  { name = "peaks-cairn-3", biome = "frozen_peaks", label = "Terceiro Mojão", radius = 9, survey = "peaks_survey" },
}

function M.marked_key(name)
  return "mark." .. name
end

function M.is_marked(name)
  return viber.game()[M.marked_key(name)] == true
end

function M.available(mark, rules)
  if mark.biome == "tower" then
    return true
  end
  return rules.biome_revealed(mark.biome)
end

function M.position(name)
  local id = viber.find(name)
  if not id then
    return nil
  end
  local ok, x, y, z = viber.entity_position(id)
  if not ok then
    return nil
  end
  return x, y, z
end

function M.near_campfire(px, py, pz)
  for _, id in ipairs(viber.find_all("campfire") or {}) do
    local ok, x, y, z = viber.entity_position(id)
    if ok then
      local dx, dy, dz = x - px, y - py, z - pz
      if dx * dx + dy * dy + dz * dz < M.CAMPFIRE_RANGE * M.CAMPFIRE_RANGE then
        return true
      end
    end
  end
  return false
end

return M
