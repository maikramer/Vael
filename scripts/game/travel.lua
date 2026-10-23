-- Nota [F], viagem rápida [G] e as marcas que a bússola desenha.
-- O mapa [M] abre o modal; os cliques vivem em ui/map.lua.

local rules = viber.load("lib/rules.lua")
local marks = viber.load("lib/landmarks.lua")

local M = {}

-- Último waypoint publicado (o mapa [M] desenha-o sobre o hillshade).
M.last_waypoint = nil

-- O HUD esconde a seta do waypoint a menos de 4 m do destino
-- (`hud/waypoint.rs`, ARRIVED_M). Chegar é o fim natural de um pin: o
-- destino sai do save no mesmo instante em que a seta se cala.
local PIN_ARRIVED_M = 4.0

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
        -- `radius` é conteúdo editável: ausente/NaN fazia comparação com nil
        -- e a varredura do [F] rebentava em todos os frames. 0 = não assina.
        local radius = tonumber(mark.radius) or 0
        if d <= radius then
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
      -- `biome` é conteúdo editável; nil não pode rebentar a concatenação.
      local label = marks.BIOME_LABEL[locked.biome] or locked.biome or "um bioma por cartografar"
      viber.toast("A terra aqui ainda não foi cartografada — procura a atalaia de " .. label .. ".")
    end
    return
  end
  viber.game()[marks.marked_key(best.name)] = true
  -- Assinar NÃO escreve `game.pin`: o pin é o DESTINO escolhido no mapa (ou
  -- indicado por um NPC). Antes, medir um marco re-apontava a seta para os
  -- pés de quem o mediu e apagava o destino que estava marcado.
  --
  -- Relata a visita SEMPRE: `report_visit` já filtra por quest ativa e pelos
  -- tokens do objetivo. A guarda pelo campo `survey` perdia a visita dos dois
  -- marcos da romaria que não o têm em `data/landmarks.json`
  -- (`vael-watch` e `vael-rim-cairn`), ficando dependente do `scan_visits`.
  rules.report_visit(best.name)
  local best_label = marks.label(best.name)
  local label = marks.BIOME_LABEL[best.biome] or best.biome or "sem bioma"
  if best.biome == "tower" then
    -- Atalaia não faz parte da Nota: não há "faltam N marcos de atalaia".
    viber.toast("Atalaia cartografada: " .. best_label .. " — viaja com [G] na fogueira.")
  else
    local left = remaining(best.biome)
    if left == 0 then
      -- Sem `string.upper`: o Luau é ASCII e "pântano" saía "PâNTANO".
      viber.toast("Medido e assinado: " .. best_label .. " — " .. label .. " sem marcos por assinar!")
    elseif left == 1 then
      viber.toast("Medido e assinado: " .. best_label .. " (falta 1 marco: " .. label .. ")")
    else
      viber.toast("Medido e assinado: " .. best_label .. " (faltam " .. left .. " marcos: " .. label .. ")")
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
      -- Marco sem entidade no mundo (conteúdo editado/renomeado) não entra na
      -- lista: a linha ficava clicável e a viagem só podia falhar no toast.
      if x then
        local dist = ""
        if px then
          dist = tostring(math.floor(math.sqrt((x - px) ^ 2 + (z - pz) ^ 2))) .. " m"
        end
        out[#out + 1] = {
          id = mark.name,
          label = marks.label(mark.name),
          biome = marks.BIOME_LABEL[mark.biome] or mark.biome or "sem bioma",
          distance = dist,
          status = marks.is_marked(mark.name) and "done" or "ready",
        }
      end
    end
  end
  return out
end

function M.go(name)
  local x, y, z, id = marks.position(name)
  if not x then
    viber.toast("Esse marco ainda não está no mundo.")
    return
  end
  -- O Y autorado é a pose FINAL do mundo estático: numa atalaia é a
  -- plataforma (bem acima do terreno); num poço é a base. O `ground_below`
  -- só conhece o terreno e serve de piso mínimo — conteúdo com uma cota
  -- absurda não manda o herói para debaixo do chão.
  local ground = viber.ground_below(x, (y or 0) + 80, z)
  local py
  if y == nil then
    py = (ground or 0) + 1.0
  else
    py = math.max(y, ground or y) + 1.0
  end
  -- A engine desce um raio de física de `py + 2` e assenta na primeira
  -- superfície sólida (plataforma, ponte, telhado, terreno). O `id` do marco
  -- fica FORA do raio: o próprio poço/torre não pode servir de chão. Sem o
  -- `set_pose` de outrora — o collider da plataforma segura o herói.
  viber.teleport_player(x, py, z, id)
  -- Chave morta de versões anteriores (ninguém a lê — o estado do painel
  -- vive na UI). Limpa-se no primeiro `go` para não ficar no save.
  viber.game().travel_open = nil
  viber.ui.open("travel", false)
  viber.toast("A fogueira leva-te até lá.")
  viber.sound("travel", { x = x, y = py, z = z })
end

-- O marco existe no CONTEÚDO? `marks.position` só vê a cena atual: dentro de
-- uma casa (onde vivem os townsfolk que oferecem "Perguntar por um lugar")
-- nenhum marco do mundo grande resolve, por isso o pedido era recusado — e
-- ainda apagava o `pin` que o jogador já tinha. A validação da ENTIDADE fica
-- para o `publish`, que corre no mundo e limpa um destino morto.
local function content_mark(name)
  if type(name) ~= "string" or name == "" then
    return nil
  end
  for _, mark in ipairs(marks.LIST) do
    if mark.name == name then
      return mark
    end
  end
  return nil
end

function M.pin(name)
  local mark = content_mark(name)
  if not mark then
    -- Nome desconhecido: não é um destino morto (não havia destino nenhum);
    -- o pin atual fica como estava.
    viber.toast("Não conheço esse lugar.")
    return
  end
  -- No mundo principal a entidade pode ter sido apagada/renomeada no editor.
  -- Nas cenas próprias (interiores) não existem marcos: aceita-se o nome do
  -- conteúdo e o destino resolve quando o herói voltar ao mundo.
  local scene = viber.scene_name and viber.scene_name() or nil
  if (scene == nil or scene == "main") and not marks.position(name) then
    if viber.game().pin == name then
      viber.game().pin = nil
    end
    viber.toast("Esse destino já não existe.")
    return
  end
  viber.game().pin = name
  viber.toast("Destino: " .. marks.label(name))
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

-- `viber.find` cai para SUBSTRING quando o nome exato não existe. Um NPC
-- renomeado no conteúdo passaria a resolver para outra entidade cujo nome o
-- contém — e a seta de "Entregar"/"Missão" apontava para o sítio errado.
local function find_exact(name)
  local id = name and viber.find(name)
  if id and viber.entity_name(id) == name then
    return id
  end
  return nil
end

local function npc_waypoint(def, state)
  local id = find_exact(def.npc)
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
  local new, new_d
  for _, def in ipairs(rules.quest_defs()) do
    local state = rules.quest_state(def.id)
    if state == "ready" or state == "active" then
      -- Pronta a entregar: a seta aponta ao DADOR. Cair para o objetivo
      -- (os recursos já estão na mochila) escondia que o passo seguinte era
      -- voltar a falar com quem a deu.
      local target
      if state == "ready" then
        target = npc_waypoint(def, state)
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
    elseif state == "not_taken" then
      -- Sem missão em curso, a seta aponta a um dador novo (README:
      -- entregar → cumprir → missão nova). O mais próximo evita mandar o
      -- jogador ao outro canto do vale sem razão.
      local target = npc_waypoint(def, state)
      if target then
        local dx, dz = target.x - (px or 0), target.z - (pz or 0)
        local d = dx * dx + dz * dz
        if not new_d or d < new_d then
          new, new_d = target, d
        end
      end
    end
  end
  return tracked_target or ready or active or new
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
    -- Há UM waypoint: seguir uma missão é a escolha mais recente e tem de
    -- ganhar ao pin do mapa. Sem isto, "Seguir" no diário mudava o cartão do
    -- HUD mas a seta continuava a apontar para o pin antigo.
    game.pin = nil
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
  local px, py, pz = player_xz()
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
    if not x then
      -- Destino morto (marco apagado/renomeado): sai do save em vez de ficar
      -- a cair no waypoint da quest para sempre, sem se perceber porquê.
      viber.game().pin = nil
    elseif px and mark_distance(x, y, z, px, py, pz) <= PIN_ARRIVED_M then
      -- Chegámos ao destino (o mesmo raio em que a seta do HUD se cala).
      -- Sem isto o pin ficava no save e a bússola apontava para trás.
      viber.game().pin = nil
    else
      waypoint = { x = x, y = y, z = z, label = marks.label(pin) }
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
  local near, near_d, locked, locked_d
  for _, mark in ipairs(marks.LIST) do
    if not marks.is_marked(mark.name) then
      local x, y, z = marks.position(mark.name)
      if x then
        local d = mark_distance(x, y, z, px, py, pz)
        local radius = tonumber(mark.radius) or 0
        if d <= radius then
          if marks.available(mark, rules) then
            -- O MAIS PRÓXIMO, como em `sign_nearest`: o aviso tem de nomear
            -- o marco que o [F] vai mesmo assinar. Antes ficava com o
            -- primeiro da LIST e podia anunciar outro.
            if not near_d or d < near_d then
              near, near_d = mark, d
            end
          elseif not locked_d or d < locked_d then
            locked, locked_d = mark, d
          end
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
    viber.toast("[F] Assinar " .. marks.label(mark.name))
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
  -- Um LOAD repõe `viber.game()` e o `playtime_secs` do save volta atrás, mas
  -- o `session_id` gravado pode ser o mesmo: os avisos de [F] já mostrados e o
  -- recurso em cache ficavam do jogo anterior (o do save podia estar ainda
  -- por colher). O relógio da engine NÃO recomeça no load, por isso o sinal é
  -- o tempo de jogo a diminuir.
  local playtime = tonumber(viber.game().playtime_secs) or 0
  if st.playtime and playtime < st.playtime then
    sign_hinted = {}
    locked_hinted = {}
    resource_cache = {}
    M.last_waypoint = nil
  end
  st.playtime = playtime
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
  -- Herói caído: o modal de morte só abre ao fim dos 2,2 s de `dying`, por
  -- isso `menu_open` ainda é falso e o [F]/[G]/[M] continuavam a valer (dava
  -- para assinar um marco ou abrir a viagem já morto).
  local alive, hp = rules.player_hp()
  local downed = alive and (hp or 0) <= 0
  if viber.menu_open() and not viber.ui.is_open("travel") and not viber.ui.is_open("region-map") then
    -- outro menu: não assinar nem viajar
  else
    if not downed and viber.input.pressed("f") and not viber.ui.is_open("travel") and not viber.ui.is_open("region-map") then
      M.sign_nearest()
    end
    if viber.input.pressed("g") then
      local open = viber.ui.is_open("travel")
      -- Abrir por cima do mapa regional empilhava dois modais.
      if open or (not downed and near_fire() and not viber.menu_open()) then
        viber.ui.open("travel", not open)
      end
    end
    if viber.input.pressed("m") then
      local open = viber.ui.is_open("region-map")
      if open or (not downed and not viber.menu_open()) then
        viber.ui.open("region-map", not open)
      end
    end
    if viber.ui.is_open("travel") and (downed or not near_fire()) then
      viber.ui.open("travel", false)
    end
  end
  local go = viber.game().travel_go
  if go then
    viber.game().travel_go = nil
    -- Um clique no painel no frame da morte não pode teleportar o cadáver.
    if not downed then
      M.go(go)
    end
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
