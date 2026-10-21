-- Cliques do mapa regional [M] e da viagem [G]. As teclas estão em
-- game/travel.lua (o UiRoot pode não ser o único a correr).

local travel = viber.load("game/travel.lua")
local marks = viber.load("lib/landmarks.lua")

local last_map = ""
local last_travel = ""

local function signature(rows)
  local parts = {}
  for i, row in ipairs(rows) do
    -- Inclui label/bioma (renderizados pelo XML) e separa os campos: sem
    -- isto, uma tradução a quente nunca refrescava o mapa aberto.
    parts[i] = table.concat({
      row.id, row.label, row.biome, row.distance, row.status,
    }, "\2")
  end
  return table.concat(parts, "\1")
end

local function refresh(bind, rows, last)
  local sig = signature(rows)
  if sig == last then
    return last
  end
  viber.ui.list(bind, rows)
  return sig
end

function on_update(_dt)
  if viber.ui.is_open("region-map") then
    last_map = refresh("region-map", travel.rows(false), last_map)
  end
  if viber.ui.is_open("travel") then
    last_travel = refresh("travel", travel.rows(true), last_travel)
  end
  for _, mark in ipairs(marks.LIST) do
    if viber.ui.clicked("pin-" .. mark.name) then
      viber.game().pin_request = mark.name
    end
    if viber.ui.clicked("go-" .. mark.name) then
      viber.game().travel_go = mark.name
    end
  end
end
