-- Nota [F], viagem rápida [G] e as marcas que a bússola desenha.
-- O mapa [M] abre o modal; os cliques vivem em ui/map.lua.

local rules = viber.load("lib/rules.lua")
local marks = viber.load("lib/landmarks.lua")

local M = {}

local function player_xz()
  local ok, x, y, z = viber.player_position()
  if not ok then
    return nil
  end
  return x, y, z
end

local function remaining(biome)
  local n = 0
  for _, mark in ipairs(marks.LIST) do
    if mark.biome == biome and marks.available(mark, rules) and not marks.is_marked(mark.name) then
      n = n + 1
    end
  end
  return n
end

function M.sign_nearest()
  local px, py, pz = player_xz()
  if not px then
    return
  end
  local best, best_d, locked
  for _, mark in ipairs(marks.LIST) do
    if not marks.is_marked(mark.name) then
      local x, _, z = marks.position(mark.name)
      if x then
        local dx, dz = x - px, z - pz
        local d = math.sqrt(dx * dx + dz * dz)
        if d <= mark.radius and (not best_d or d < best_d) then
          if marks.available(mark, rules) then
            best, best_d = mark, d
          else
            locked = mark
          end
        end
      end
    end
  end
  if not best then
    if locked then
      local label = marks.BIOME_LABEL[locked.biome] or locked.biome
      viber.toast("A terra aqui ainda não foi cartografada — procura a atalaia de " .. label .. ".")
    end
    return
  end
  viber.game()[marks.marked_key(best.name)] = true
  viber.game().pin = best.name
  if best.survey and best.survey ~= "" then
    rules.report_visit(best.name)
  end
  local label = marks.BIOME_LABEL[best.biome] or best.biome
  local left = remaining(best.biome)
  if left == 0 then
    viber.toast("Medido e assinado: " .. best.label .. " — " .. string.upper(label) .. " ASSINADO!")
  else
    viber.toast("Medido e assinado: " .. best.label .. " (faltam " .. left .. " em " .. label .. ")")
  end
  viber.sound("quest_complete")
end

function M.rows(only_marked)
  local px, _, pz = player_xz()
  local out = {}
  for _, mark in ipairs(marks.LIST) do
    if marks.available(mark, rules) and (not only_marked or marks.is_marked(mark.name)) then
      local x, _, z = marks.position(mark.name)
      local dist = ""
      if x and px then
        dist = tostring(math.floor(math.sqrt((x - px) ^ 2 + (z - pz) ^ 2))) .. " m"
      end
      out[#out + 1] = {
        id = mark.name,
        label = mark.label,
        biome = marks.BIOME_LABEL[mark.biome] or mark.biome,
        distance = dist,
        status = marks.is_marked(mark.name) and "done" or "ready",
      }
    end
  end
  return out
end

function M.go(name)
  local x, y, z = marks.position(name)
  if not x then
    viber.toast("Esse marco ainda não está no mundo.")
    return
  end
  local ground = viber.ground_below(x, (y or 0) + 80, z)
  local py = ground or y or 0
  viber.set_pose(x, py + 0.2, z, 0, viber.player_id())
  viber.game().travel_open = false
  viber.ui.open("travel", false)
  viber.toast("A fogueira leva-te até lá.")
  viber.sound("travel")
end

function M.pin(name)
  viber.game().pin = name
  local label = name
  for _, mark in ipairs(marks.LIST) do
    if mark.name == name then
      label = mark.label
      break
    end
  end
  viber.toast("Waypoint: " .. label)
end

local function near_fire()
  local x, y, z = player_xz()
  if not x then
    return false
  end
  return marks.near_campfire(x, y, z)
end

local function quest_waypoint()
  for _, def in ipairs(rules.quest_defs()) do
    local state = rules.quest_state(def.id)
    if state == "ready" or state == "active" then
      local id = def.npc and viber.find(def.npc)
      if id then
        local ok, x, y, z = viber.entity_position(id)
        if ok then
          local prefix = state == "ready" and "Entregar" or "Missão"
          return { x = x, y = y, z = z, label = prefix .. " · " .. (def.title or def.id) }
        end
      end
    end
  end
  return nil
end

local function publish()
  local blips = {}
  local beacons = {}
  for _, mark in ipairs(marks.LIST) do
    if marks.available(mark, rules) then
      local x, y, z = marks.position(mark.name)
      if x then
        blips[#blips + 1] = { x = x, y = y, z = z, tower = mark.biome == "tower" }
        beacons[#beacons + 1] = { x = x, y = y, z = z }
      end
    end
  end
  local pin = viber.game().pin
  local waypoint
  if pin then
    local x, y, z = marks.position(pin)
    if x then
      local label = pin
      for _, mark in ipairs(marks.LIST) do
        if mark.name == pin then
          label = mark.label
          break
        end
      end
      waypoint = { x = x, y = y, z = z, label = label }
    end
  end
  if not waypoint then
    waypoint = quest_waypoint()
  end
  viber.markers({ blips = blips, beacons = beacons, waypoint = waypoint })
end

function M.update(dt)
  local st = viber.state()
  st.acc = (st.acc or 0) + dt
  if viber.menu_open() and not viber.ui.is_open("travel") and not viber.ui.is_open("region-map") then
    -- outro menu: não assinar nem viajar
  else
    if viber.input.pressed("f") and not viber.ui.is_open("travel") and not viber.ui.is_open("region-map") then
      M.sign_nearest()
    end
    if viber.input.pressed("g") then
      local open = viber.ui.is_open("travel")
      if open or near_fire() then
        viber.ui.open("travel", not open)
      end
    end
    if viber.input.pressed("m") then
      local open = viber.ui.is_open("region-map")
      if open or not viber.menu_open() then
        viber.ui.open("region-map", not open)
      end
    end
    if viber.ui.is_open("travel") and not near_fire() then
      viber.ui.open("travel", false)
    end
  end
  local go = viber.game().travel_go
  if go then
    viber.game().travel_go = nil
    M.go(go)
  end
  local pin = viber.game().pin_request
  if pin then
    viber.game().pin_request = nil
    M.pin(pin)
  end
  if st.acc >= 0.5 then
    st.acc = 0
    publish()
  end
end

return M
