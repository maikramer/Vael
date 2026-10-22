-- Nota [F], viagem rápida [G] e as marcas que a bússola desenha.
-- O mapa [M] abre o modal; os cliques vivem em ui/map.lua.

local rules = viber.load("lib/rules.lua")
local marks = viber.load("lib/landmarks.lua")

local M = {}

-- Último waypoint publicado (o mapa [M] desenha-o sobre o hillshade).
M.last_waypoint = nil

function M.waypoint()
  return M.last_waypoint
end

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

-- Distância a 3D. A 2D deixava assinar (e ver o aviso de [F]) uma atalaia
-- a partir do chão: a entidade vive no TOPO da plataforma, ~12 m acima dos
-- pés de quem está na base.
local function mark_distance(x, y, z, px, py, pz)
  local dx = x - px
  local dy = (y or 0) - (py or 0)
  local dz = z - pz
  return math.sqrt(dx * dx + dy * dy + dz * dz)
end

function M.sign_nearest()
  local px, py, pz = player_xz()
  if not px then
    return
  end
  local best, best_d, locked, locked_d
  for _, mark in ipairs(marks.LIST) do
    if not marks.is_marked(mark.name) then
      local x, y, z = marks.position(mark.name)
      if x then
        local d = mark_distance(x, y, z, px, py, pz)
        if d <= mark.radius then
          if marks.available(mark, rules) then
            if not best_d or d < best_d then
              best, best_d = mark, d
            end
          elseif not locked_d or d < locked_d then
            -- O aviso tem de nomear o bioma do marco fechado MAIS PRÓXIMO;
            -- antes ficava com o último da LIST (podia ser outro bioma).
            locked, locked_d = mark, d
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
  if best.biome == "tower" then
    -- Atalaia não faz parte da Nota: não há "faltam N marcos de atalaia".
    viber.toast("Atalaia cartografada: " .. best.label .. " — viaja com [G] na fogueira.")
  else
    local left = remaining(best.biome)
    if left == 0 then
      -- Sem `string.upper`: o Luau é ASCII e "pântano" saía "PâNTANO".
      viber.toast("Medido e assinado: " .. best.label .. " — " .. label .. " sem marcos por assinar!")
    elseif left == 1 then
      viber.toast("Medido e assinado: " .. best.label .. " (falta 1 marco: " .. label .. ")")
    else
      viber.toast("Medido e assinado: " .. best.label .. " (faltam " .. left .. " marcos: " .. label .. ")")
    end
  end
  local has, hx, hy, hz = viber.player_position()
  if has then
    viber.sound("quest_complete", { x = hx, y = hy, z = hz })
  else
    viber.sound("quest_complete")
  end
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
  -- `ground_below` só conhece o terreno: numa atalaia (entidade no TOPO da
  -- plataforma) o teleporte caía na base e o jogador tinha de subir outra
  -- vez. Se a cota do marco está claramente acima do chão, é a plataforma.
  local py
  if y == nil then
    py = ground or 0
  elseif ground and math.abs(y - ground) <= 2.0 then
    py = ground
  else
    py = y
  end
  -- `teleport_player` assenta o character controller, mas a engine resolve o
  -- Y contra o TERRAIN (`landing_position`): numa atalaia ele descarta a cota
  -- da plataforma e larga o herói na base. Para marcos claramente elevados,
  -- o `teleport_player` limpa a inércia/settle e o `set_pose` repõe a cota
  -- exata da plataforma (o collider dela segura o herói).
  local elevated = ground ~= nil and y ~= nil and (y - ground) > 2.0
  viber.teleport_player(x, py + 0.2, z)
  if elevated then
    local pid = viber.player_id()
    if pid then
      viber.set_pose(x, py + 0.2, z, 0, pid)
    end
  end
  viber.game().travel_open = false
  viber.ui.open("travel", false)
  viber.toast("A fogueira leva-te até lá.")
  viber.sound("travel", { x = x, y = py, z = z })
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
  viber.toast("Destino: " .. label)
end

local function near_fire()
  local x, y, z = player_xz()
  if not x then
    return false
  end
  return marks.near_campfire(x, y, z)
end

local BOSS_KINDS = {
  witch = true, ["sand-worm"] = true, ["bog-warden"] = true, boss = true,
}

local function nearest_entry(list, px, pz)
  local best, best_d
  for _, e in ipairs(list) do
    if e.x and e.z then
      local dx, dz = e.x - px, e.z - pz
      local d = dx * dx + dz * dz
      if not best_d or d < best_d then
        best, best_d = e, d
      end
    end
  end
  return best
end

-- Recurso mais próximo do tipo pedido. `player_nearby` já traz os props
-- autorais (`resource-kind`); a colheita persistida vive em `harvested.<nome>`.
local RESOURCE_CACHE_SECS = 2.0
local resource_cache = {}

-- Recursos que NÃO usam `resource-kind`: têm script próprio e marca de
-- colhido `once:<nome>` no save (lib/once.lua). Sem esta tabela o waypoint da
-- quest do musgo procurava para sempre e caía no dador em vez do tufo.
local NAME_RESOURCES = {
  ["bog-moss"] = { prefix = "bog-moss-", key = "once:" },
}

local function nearest_named_resource(prefix, key, px, pz)
  local best, best_d
  for _, id in ipairs(viber.find_all(prefix) or {}) do
    local name = viber.entity_name(id) or ""
    if name ~= "" and not viber.game()[key .. name] then
      local ok, x, y, z = viber.entity_position(id)
      if ok then
        local dx, dz = x - px, z - pz
        local d = dx * dx + dz * dz
        if not best_d or d < best_d then
          best = { x = x, y = y, z = z, name = name }
          best_d = d
        end
      end
    end
  end
  return best
end

local function nearest_resource(kind, px, pz)
  -- `player_nearby` ordena por distância e o limite é CAPADO a 64 dentro da
  -- engine (pedir 512 não subia o teto): a 420 m os 64 mais próximos são
  -- árvores/pedras de outro tipo e o recurso pedido nunca aparecia. Anéis
  -- crescentes: o primeiro anel com o recurso certo ganha. O resultado fica
  -- 2 s em cache (prop estático, waypoint a 2 Hz) para não repetir a varredura
  -- por cada quest de colecta ativa.
  local now = viber.time()
  local cached = resource_cache[kind]
  if cached and now >= cached.t and now - cached.t < RESOURCE_CACHE_SECS then
    return cached.e
  end
  local named = NAME_RESOURCES[kind]
  if named then
    local e = nearest_named_resource(named.prefix, named.key, px, pz)
    resource_cache[kind] = { t = now, e = e }
    return e
  end
  local result
  for _, radius in ipairs({ 60, 140, 300, 420 }) do
    local best, best_d
    for _, e in ipairs(viber.player_nearby(radius, 64) or {}) do
      local props = e.props or {}
      if props["resource-kind"] == kind and e.x and e.z then
        local name = e.name or ""
        local harvested = name ~= "" and viber.game()["harvested." .. name]
        if not harvested then
          local dx, dz = e.x - px, e.z - pz
          local d = dx * dx + dz * dz
          if not best_d or d < best_d then
            best, best_d = e, d
          end
        end
      end
    end
    if best then
      result = best
      break
    end
  end
  resource_cache[kind] = { t = now, e = result }
  return result
end

-- O objetivo da quest, não o NPC que a dá. Kill → presa viva mais próxima;
-- visit → marco por visitar; collect → recurso mais próximo (ou guardião,
-- no caso das Notas). Devolve nil quando não há alvo conhecido.
local function objective_waypoint(def, px, pz)
  local obj = def.objective or {}
  local kind = obj.type or ""
  if kind == "kill" then
    local list = {}
    for _, h in ipairs(rules.hostiles_all()) do
      if rules.kind_matches(h.kind, obj.target or "") then
        list[#list + 1] = h
      end
    end
    local h = nearest_entry(list, px, pz)
    if h then
      return { x = h.x, y = h.y, z = h.z, label = "Caçar · " .. (def.title or def.id) }
    end
  elseif kind == "visit" then
    local best, best_d
    for token in tostring(obj.target or ""):gmatch("%S+") do
      local mark = "q." .. def.id .. ".v." .. token
      if viber.game()[mark] ~= true then
        local x, y, z = marks.position(token)
        if x then
          local dx, dz = x - px, z - pz
          local d = dx * dx + dz * dz
          if not best_d or d < best_d then
            best = { x = x, y = y, z = z, label = "Visitar · " .. marks.label(token) }
            best_d = d
          end
        end
      end
    end
    if best then
      return best
    end
  elseif kind == "collect" then
    if obj.target == "nota" then
      local list = {}
      for _, h in ipairs(rules.hostiles_all()) do
        -- Um guardião cuja Nota já está no save não tem nada para dar: a
        -- seta não pode mandar o jogador para lá (o `nota_*` sobrevive ao
        -- save e o chefe só volta a existir num mundo sem `boss_dead`).
        if BOSS_KINDS[h.kind] and not viber.game()["nota_" .. h.kind] then
          list[#list + 1] = h
        end
      end
      local h = nearest_entry(list, px, pz)
      if h then
        return { x = h.x, y = h.y, z = h.z, label = "Guardião · " .. (def.title or def.id) }
      end
    else
      local e = nearest_resource(obj.target or "", px, pz)
      if e then
        return { x = e.x, y = e.y, z = e.z, label = "Recolher · " .. (def.title or def.id) }
      end
    end
  end
  return nil
end

local function npc_waypoint(def, state)
  local id = def.npc and viber.find(def.npc)
  if not id then
    return nil
  end
  local ok, x, y, z = viber.entity_position(id)
  if not ok then
    return nil
  end
  local prefix = state == "ready" and "Entregar" or "Missão"
  return { x = x, y = y, z = z, label = prefix .. " · " .. (def.title or def.id) }
end

local function quest_waypoint()
  local px, _, pz = player_xz()
  local tracked = viber.game().tracked_quest
  local tracked_target, ready, active
  for _, def in ipairs(rules.quest_defs()) do
    local state = rules.quest_state(def.id)
    if state == "ready" or state == "active" then
      -- Pronta a entregar: a seta aponta ao DADOR, não a mais recursos.
      local target
      if state == "ready" then
        target = npc_waypoint(def, state) or objective_waypoint(def, px or 0, pz or 0)
      else
        target = objective_waypoint(def, px or 0, pz or 0) or npc_waypoint(def, state)
      end
      if target then
        if def.id == tracked then
          tracked_target = target
        end
        -- Entregar (ready) tem prioridade sobre cumprir (active), mesmo que
        -- a quest ativa apareça primeiro na lista de defs. O fallback não
        -- pode depender do `tracked_quest`: uma chave morta (quest entregue)
        -- deixava a seta sem alvo nenhum.
        if state == "ready" then
          ready = ready or target
        else
          active = active or target
        end
      end
    end
  end
  return tracked_target or ready or active
end

function M.track(id)
  local game = viber.game()
  local def = rules.quest_def(id)
  local state = rules.quest_state(id)
  if state ~= "active" and state ~= "ready" then
    viber.toast("Essa missão já não está em curso.")
    return
  end
  if game.tracked_quest == id then
    game.tracked_quest = nil
    viber.toast("Missão deixada de seguir.")
  else
    game.tracked_quest = id
    viber.toast("A seguir: " .. ((def and def.title) or id))
  end
end

local publish_sign_prompt
local sign_hint

local function publish()
  local blips = {}
  local beacons = {}
  -- O engine capa os beacons a 16 e descartava os ÚLTIMOS da lista (os cinco
  -- marcos do Alto, os mais recentes e os únicos sempre disponíveis). Torna-
  -- se explícito: 16 mais próximos, sem atalaias (as torres veem-se) e sem
  -- marcos já assinados.
  local candidates = {}
  local px, _, pz = player_xz()
  for _, mark in ipairs(marks.LIST) do
    if marks.available(mark, rules) then
      local x, y, z = marks.position(mark.name)
      if x then
        blips[#blips + 1] = { x = x, y = y, z = z, tower = mark.biome == "tower" }
        if mark.biome ~= "tower" and not marks.is_marked(mark.name) then
          local dx, dz = x - (px or x), z - (pz or z)
          candidates[#candidates + 1] = {
            d = dx * dx + dz * dz, x = x, y = y, z = z,
          }
        end
      end
    end
  end
  table.sort(candidates, function(a, b) return a.d < b.d end)
  for i = 1, math.min(#candidates, 16) do
    local c = candidates[i]
    beacons[#beacons + 1] = { x = c.x, y = c.y, z = c.z }
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
  M.last_waypoint = waypoint
  viber.markers({ blips = blips, beacons = beacons, waypoint = waypoint })
  publish_sign_prompt()
  sign_hint()
end

local sign_near, sign_locked
local sign_hinted = {}
local locked_hinted = {}

-- Varre os marcos a 2 Hz (é caro: `viber.find` por marco). O aviso é um
-- TOAST, não `viber.prompt`: os dois controllers escrevem no mesmo bind e o
-- último do frame ganhava — perto de uma árvore + marco o [F] apagava o
-- "Cortar" da colheita (ou vice-versa). O toast não disputa o prompt.
publish_sign_prompt = function()
  local px, py, pz = player_xz()
  if not px then
    sign_near, sign_locked = nil, nil
    return
  end
  local near, locked
  for _, mark in ipairs(marks.LIST) do
    if not marks.is_marked(mark.name) then
      local x, y, z = marks.position(mark.name)
      if x then
        if mark_distance(x, y, z, px, py, pz) <= mark.radius then
          if marks.available(mark, rules) then
            near = mark
            break
          end
          locked = locked or mark
        end
      end
    end
  end
  sign_near, sign_locked = near, locked
end

sign_hint = function()
  if viber.menu_open() then
    return
  end
  if sign_near then
    local mark = sign_near
    if sign_hinted[mark.name] then
      return
    end
    sign_hinted[mark.name] = true
    viber.toast("[F] Assinar " .. mark.label)
    return
  end
  local mark = sign_locked
  -- O aviso de marco fechado NÃO pode gastar o aviso de [F]: depois de
  -- sondar a atalaia, o jogador voltava ao mesmo marco e nunca via
  -- "[F] Assinar" (o nome já estava em `sign_hinted`).
  if not mark or locked_hinted[mark.name] then
    return
  end
  locked_hinted[mark.name] = true
  viber.toast("[F] Marco por cartografar — procura a atalaia do bioma.")
end

function M.update(dt)
  local st = viber.state()
  -- Novo Jogo/partida diferente: os avisos de [F] voltam a aparecer.
  local sid = tonumber(viber.game().session_id) or 0
  if st.session ~= sid then
    st.session = sid
    sign_hinted = {}
    locked_hinted = {}
    resource_cache = {}
    M.last_waypoint = nil
  end
  -- `tracked_quest` é uma chave do save: entregar (ou uma bounty repetível
  -- voltar a "por aceitar") deixava-a apontada a uma missão que já não
  -- está em curso. O HUD já a ignorava, mas a seta perdia o fallback.
  local tracked = viber.game().tracked_quest
  if tracked ~= nil then
    local state = type(tracked) == "string" and rules.quest_state(tracked) or nil
    if state ~= "active" and state ~= "ready" then
      viber.game().tracked_quest = nil
    end
  end
  st.acc = (st.acc or 0) + dt
  if viber.menu_open() and not viber.ui.is_open("travel") and not viber.ui.is_open("region-map") then
    -- outro menu: não assinar nem viajar
  else
    if viber.input.pressed("f") and not viber.ui.is_open("travel") and not viber.ui.is_open("region-map") then
      M.sign_nearest()
    end
    if viber.input.pressed("g") then
      local open = viber.ui.is_open("travel")
      -- Abrir por cima do mapa regional empilhava dois modais.
      if open or (near_fire() and not viber.menu_open()) then
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
