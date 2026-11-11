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

-- Com `VAEL_TRAVEL_RUST=1` o catálogo e a revelação vêm do Rust
-- (`viber.vael.travel_*`). Sem a chave — ou com a bridge em falta — tudo
-- abaixo fica como antes, a ler o JSON.
local RUST = false
local ROWS
do
  local ok, game = pcall(viber.game)
  if ok and type(game) == "table" and game.vael_travel_rust == true then
    local vael = viber.vael
    if type(vael) == "table" and type(vael.travel_landmarks) == "function" then
      local rok, rows = pcall(vael.travel_landmarks)
      if rok and type(rows) == "table" and #rows > 0 then
        RUST = true
        ROWS = rows
      else
        viber.log("landmarks: bridge Rust indisponível — fallback Lua")
      end
    end
  end
end

local function fallback_labels()
  return type(DATA.biome_labels) == "table" and DATA.biome_labels or {}
end

local function fallback_list()
  return type(DATA.landmarks) == "table" and DATA.landmarks or {}
end

-- A bridge devolve os campos já saneados; a cópia só guarda o que os
-- consumidores (`travel`, mapa, parley) leem.
local function list_from_rows(rows)
  local out = {}
  for _, row in ipairs(rows) do
    if type(row) == "table" and type(row.name) == "string" and row.name ~= "" then
      out[#out + 1] = {
        name = row.name,
        biome = type(row.biome) == "string" and row.biome or "",
        label = type(row.label) == "string" and row.label or row.name,
        radius = tonumber(row.radius) or 0,
        survey = type(row.survey) == "string" and row.survey or "",
        pilgrim = row.pilgrim == true,
      }
    end
  end
  return out
end

local function labels_from_rows(rows, fallback)
  local out = {}
  for _, row in ipairs(rows) do
    if type(row) == "table" and type(row.biome) == "string" and row.biome ~= ""
      and type(row.biome_label) == "string" and row.biome_label ~= "" then
      out[row.biome] = row.biome_label
    end
  end
  -- Biomas sem marcos continuam a ter nome no mapa.
  for biome, label in pairs(fallback) do
    if out[biome] == nil then
      out[biome] = label
    end
  end
  return out
end

M.CAMPFIRE_RANGE = tonumber(DATA.campfire_range) or 14
-- `type(...) == "table"`: um campo escalar (edição à mão) fazia o `ipairs`
-- dos consumidores (`label`, travel, mapa) rebentar a cada frame.
M.BIOME_LABEL = RUST and labels_from_rows(ROWS, fallback_labels()) or fallback_labels()
M.LIST = RUST and list_from_rows(ROWS) or fallback_list()

function M.marked_key(name)
  return "mark." .. name
end

function M.is_marked(name)
  return viber.game()[M.marked_key(name)] == true
end

-- Nome autoral de um marco. Os waypoints mostravam o id cru da entidade
-- ("Visitar · forest-outpost-tower") em vez do nome que o mapa usa.
function M.label(name)
  if RUST and type(name) == "string" and name ~= "" then
    local ok, label = pcall(viber.vael.travel_label, name)
    if ok and type(label) == "string" and label ~= "" then
      return label
    end
  end
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
  if RUST then
    local ok, revealed = pcall(viber.vael.travel_is_revealed, mark.biome)
    if ok then
      return revealed == true
    end
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
