-- Marcos da Nota. Os nomes são entidades do mundo; a posição vem de
-- `viber.find`. A engine não tem comportamento de marco.
--
-- A lista vive em `data/landmarks.json` para o editor poder editá-la como
-- conteúdo (o plugin `content_landmarks.lua`).

local M = {}

local DATA = (function()
  local ok, data = pcall(viber.load_json, "data/landmarks.json")
  if ok and type(data) == "table" then
    return data
  end
  viber.log("landmarks: falha a ler data/landmarks.json — sem marcos")
  return {}
end)()

M.CAMPFIRE_RANGE = tonumber(DATA.campfire_range) or 14
M.BIOME_LABEL = DATA.biome_labels or {}
M.LIST = DATA.landmarks or {}

function M.marked_key(name)
  return "mark." .. name
end

function M.is_marked(name)
  return viber.game()[M.marked_key(name)] == true
end

-- Nome autoral de um marco. Os waypoints mostravam o id cru da entidade
-- ("Visitar · forest-outpost-tower") em vez do nome que o mapa usa.
function M.label(name)
  for _, mark in ipairs(M.LIST) do
    if mark.name == name then
      -- `label` é conteúdo editável: uma entrada gravada sem nome autoral
      -- metia `nil` em toasts/UiText e o `..` rebentava. O id é o fallback.
      local label = mark.label
      if type(label) == "string" and label ~= "" then
        return label
      end
      break
    end
  end
  return name
end

function M.available(mark, rules)
  -- O Alto de Vael não é um bioma cartografável: os marcos da peregrinação
  -- estão sempre disponíveis.
  if mark.biome == "tower" or mark.pilgrim then
    return true
  end
  -- Marco editado sem bioma: concatenar em `biome.<nil>` dava erro a cada
  -- publish (o mapa/travel deixava de atualizar). Um marco sem bioma fica
  -- fechado até o conteúdo ser corrigido.
  if type(mark.biome) ~= "string" or mark.biome == "" then
    return false
  end
  return rules ~= nil and rules.biome_revealed(mark.biome) == true
end

-- Devolve `x, y, z, id` — o `id` é a entidade do marco; a viagem usa-o para
-- a excluir do raio de aterragem (um poço/torre não é chão por cima de si).
function M.position(name)
  local id = viber.find(name)
  if not id then
    return nil
  end
  -- `viber.find` cai para SUBSTRING quando não há nome exato: um marco que
  -- saia do mundo (o conteúdo é editável) resolvia para outro prop cujo nome
  -- o contém e a bússola/viagem levavam ao sítio errado. Só vale o homónimo.
  if viber.entity_name(id) ~= name then
    return nil
  end
  local ok, x, y, z = viber.entity_position(id)
  if not ok then
    return nil
  end
  return x, y, z, id
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
