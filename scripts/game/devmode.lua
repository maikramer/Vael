-- game/devmode.lua: ferramentas de um ARRANQUE de debug
-- (`viber run --scene main --entry dev`). O sinal é o `lib/dev.lua`
-- (`skip_intro`) — num jogo normal (menu, `Novo Jogo`, save) este controller
-- não faz nada. O latch vive em `viber.state()`: atravessa trocas de cena,
-- NÃO vai para o save.
--
--   [F1] voo livre      — o herói fica invisível e anda com a câmara
--   [F2] invulnerável   — herói normal, dano bloqueado em `lib/rules.lua`
--
-- No voo: WASD move, Space/ctrl sobe/desce, Shift acelera, setas olham e
-- Tab "look at" (aponta ao alvo nomeado/criatura mais alinhado com a vista).
-- O corpo do herói é reclamado (`own_system("locomotion")`): o
-- `player_movement` cala e a pose é escrita por `set_pose`. Ao sair do voo o
-- sistema é DEVOLVIDO (`release_system`) e o herói aterra onde a câmara
-- estava — a engine resolve o chão contra a física (`teleport_player`).
--
-- Host: `world/autoload/player.xml` (`tag="always-active"`): corre em todas
-- as cenas, como as regras.

local dev = viber.load("lib/dev.lua")
local rules = viber.load("lib/rules.lua")

-- Voar: velocidade base e multiplicador do Shift. Longe o suficiente para
-- atravessar o mapa em segundos; perto o suficiente para mirar props.
local FLY_SPEED = 14.0
local FLY_BOOST = 3.0
-- Teclas de olhar (graus/s). O rato não chega ao jogo (a câmara é
-- gamepad-style no runtime), por isso as setas são o controlo de vista.
local YAW_SPEED = 100.0
local PITCH_SPEED = 70.0
local MAX_PITCH = 89.0
-- Look-at: só alvos à frente (dot com a vista) e dentro deste raio. O
-- esqueleto/equipamento do herói (que viaja com a câmara) é excluído por ID
-- (`player_nearby(4, 64)`) e por nome (`is_internal_name`).
local LOOK_AT_RADIUS = 400.0
local LOOK_AT_MIN_M = 1.0
local LOOK_AT_MIN_DOT = 0.15
-- Ponto do `nearby` à frente da câmara: o controlador invisível é o centro
-- da query. Com ele EM CIMA do herói, os nós do esqueleto + equipamento
-- enchiam o cap de 64 do `player_nearby` e nenhum prop chegava à lista.
local LOOK_AT_PROBE_M = 30.0
-- Nós internos de glTF (nome genérico, sem significado de mundo).
local LOOK_AT_IGNORED = {
  Scene = true,
  root = true,
  Mesh = true,
  ["Mesh.Material"] = true,
  Armature = true,
  sky = true,
}
-- Nomes INTERNOS que não são alvos de "look at": rig humanoide, nós de
-- glTF, chunks do terreno, controllers sempre-activos e marcadores de debug.
-- Os nomes de mundo do Vael são kebab-case (`atalaia-do-vale`).
local SKIP_PREFIXES = {
  "spine_",
  "clavicle_",
  "upperarm_",
  "lowerarm_",
  "hand_",
  "thigh_",
  "calf_",
  "foot_",
  "ball_",
  "neck_",
  "bone_",
  "RightHand",
  "LeftHand",
  "voxel chunk ",
  "chunk ",
  "game-",
  "fx:",
  "debug:",
}
local SKIP_EXACT = { Head = true, pelvis = true, Autoload = true }

local function is_internal_name(name)
  if SKIP_EXACT[name] then
    return true
  end
  for _, prefix in ipairs(SKIP_PREFIXES) do
    if name:sub(1, #prefix) == prefix then
      return true
    end
  end
  return false
end

-- Estado em `viber.state()` (e não em locals): o hot-reload troca o chunk
-- e os locals voltavam a nil — a câmara ficava em modo livre sem ninguém a
-- conduzir. O estado da entidade sobrevive à recarga.
--   fly         = { pid, x, y, z, yaw, pitch } | nil (a pé)
--   god         = preferência do [F2] (o voo força por cima)
--   look_target = último alvo do Tab (para ciclar)
local function state()
  return viber.state()
end

local function toast(msg)
  viber.toast("[dev] " .. msg)
end

local function clamp(v, lo, hi)
  if v < lo then
    return lo
  elseif v > hi then
    return hi
  end
  return v
end

-- `rules.set_god` é o único sítio que decide o dano: mantém a regra num
-- módulo (o HUD, o veneno e o `damage_player` leem o mesmo sinal).
local function apply_god()
  local st = state()
  rules.set_god(st.god == true or st.fly ~= nil)
end

-- Forward da vista: a MESMA convenção do `viber.camera_pose`.
local function view_forward(fly)
  local cy = math.cos(math.rad(fly.yaw))
  local sy = math.sin(math.rad(fly.yaw))
  local cp = math.cos(math.rad(fly.pitch))
  local sp = math.sin(math.rad(fly.pitch))
  return sy * cp, sp, cy * cp
end

local function start_fly()
  local pid = viber.player_id()
  local has, px, py, pz = viber.player_position()
  if not pid or not has then
    return
  end
  -- Arranca na POSE EXATA da câmara (sem salto de enquadramento): o herói
  -- invisível passa a andar com ela.
  local cam = viber.camera_state and viber.camera_state() or nil
  local st = state()
  st.fly = {
    pid = pid,
    x = cam and cam.x or px,
    y = cam and cam.y or (py + 1.6),
    z = cam and cam.z or pz,
    yaw = cam and cam.yaw or 0.0,
    pitch = cam and cam.pitch or 0.0,
  }
  st.look_target = nil
  local fly = st.fly
  viber.own_system("locomotion")
  viber.set_visible(false, pid)
  viber.camera_free(true)
  apply_god()
  local fx, fy, fz = view_forward(fly)
  viber.camera_pose(
    fly.x,
    fly.y,
    fly.z,
    fly.x + fx * 10.0,
    fly.y + fy * 10.0,
    fly.z + fz * 10.0
  )
  toast("voo ligado — WASD move, Space/ctrl sobe/desce, Shift acelera, setas olham, Tab look-at. [F1] sai.")
end

local function stop_fly()
  local st = state()
  local fly = st.fly
  if not fly then
    return
  end
  local pid = fly.pid
  local x, y, z = fly.x, fly.y, fly.z
  st.fly = nil
  st.look_target = nil
  viber.camera_free(false)
  -- Volta a mostrar e a entregar o corpo ANTES de teleportar: o
  -- `player_movement` retoma no frame seguinte e o `teleport_player` (fila
  -- diferida) assenta no chão com `TeleportSettle`.
  viber.set_visible(true, pid)
  viber.release_system("locomotion")
  viber.teleport_player(x, y, z)
  apply_god()
  toast("voo desligado — o herói aterrou onde estavas a olhar.")
end

-- Movimento e vista do modo livre. Lê o input cru (as teclas de habilidade
-- continuam reclamadas pelo `own_system`, mas WASD/setas/Space não são do
-- `player_movement` — ele está calado).
local function update_fly(dt)
  local fly = state().fly
  if not fly then
    return
  end
  dt = math.min(dt, 0.1)
  local look_x = (viber.input.down("right") and 1 or 0)
    - (viber.input.down("left") and 1 or 0)
  local look_y = (viber.input.down("up") and 1 or 0)
    - (viber.input.down("down") and 1 or 0)
  -- Virar à DIREITA diminui o yaw: forward = (sin yaw, cos yaw) e o steering
  -- nativo também subtrai (`cam.yaw_deg -= move_x * …` em player.rs). Com
  -- `+` a seta direita virava a vista para a ESQUERDA do ecrã.
  fly.yaw = fly.yaw - look_x * YAW_SPEED * dt
  fly.pitch = clamp(fly.pitch + look_y * PITCH_SPEED * dt, -MAX_PITCH, MAX_PITCH)

  local move_x = (viber.input.down("d") and 1 or 0)
    - (viber.input.down("a") and 1 or 0)
  local move_z = (viber.input.down("w") and 1 or 0)
    - (viber.input.down("s") and 1 or 0)
  local move_y = (viber.input.down("space") and 1 or 0)
    - ((viber.input.down("lctrl") or viber.input.down("rctrl")) and 1 or 0)

  local fx, fy, fz = view_forward(fly)
  -- Direita do ecrã no plano: forward × up = (-cos yaw, 0, sin yaw) — a mão
  -- que o strafe nativo usa (`process_input` com strafe positivo). up ×
  -- forward dava o lado ESQUERDO e D/A andavam ao contrário do ecrã.
  local cy = math.cos(math.rad(fly.yaw))
  local sy = math.sin(math.rad(fly.yaw))
  local dx = fx * move_z - cy * move_x
  local dy = fy * move_z + move_y
  local dz = fz * move_z + sy * move_x
  local len = math.sqrt(dx * dx + dy * dy + dz * dz)
  if len > 1e-6 then
    dx, dy, dz = dx / len, dy / len, dz / len
  end
  local boost = (viber.input.down("lshift") or viber.input.down("rshift")) and FLY_BOOST or 1
  local speed = FLY_SPEED * boost
  fly.x = fly.x + dx * speed * dt
  fly.y = fly.y + dy * speed * dt
  fly.z = fly.z + dz * speed * dt
  -- Piso de segurança: perder-se debaixo do mundo num modo de debug é pior
  -- do que ver a câmara de baixo para cima.
  if fly.y < -50.0 then
    fly.y = -50.0
  end

  -- O herói invisível vai com a câmara: streaming/LOD, `player_nearby`,
  -- áudio e gatilhos de quest continuam a olhar para o sítio certo.
  viber.set_pose(fly.x, fly.y, fly.z, fly.yaw, fly.pid)
  local lx, ly, lz = view_forward(fly)
  viber.camera_pose(
    fly.x,
    fly.y,
    fly.z,
    fly.x + lx * 10.0,
    fly.y + ly * 10.0,
    fly.z + lz * 10.0
  )
  -- O centro do `viber.nearby` é o próprio controller (invisível): 30 m à
  -- frente da câmara NO PLANO, para os nós do herói não ocuparem o cap. O
  -- resto do jogo não usa a pose deste controller.
  viber.set_pose(
    fly.x + sy * LOOK_AT_PROBE_M,
    fly.y,
    fly.z + cy * LOOK_AT_PROBE_M,
    fly.yaw
  )
end

-- [Tab] look-at: aponta a vista ao alvo nomeado (ou criatura) mais alinhado
-- com a direção atual, até 400 m. Repetir Tab passa ao seguinte — o alvo
-- anterior fica de fora da escolha.
local function look_at()
  local st = state()
  local fly = st.fly
  if not fly then
    return
  end
  local fx, fy, fz = view_forward(fly)
  local best, best_dot
  local seen = {}
  local function consider(id, name, x, y, z, min_m)
    if seen[name] or LOOK_AT_IGNORED[name] or is_internal_name(name) then
      return
    end
    local dx, dy, dz = x - fly.x, y - fly.y, z - fly.z
    local d = math.sqrt(dx * dx + dy * dy + dz * dz)
    if d < min_m or d > LOOK_AT_RADIUS or id == st.look_target then
      return
    end
    local dot = (dx * fx + dy * fy + dz * fz) / d
    if dot < LOOK_AT_MIN_DOT then
      return
    end
    seen[name] = true
    if not best_dot or dot > best_dot then
      best_dot = dot
      best = { id = id, name = name, dx = dx, dy = dy, dz = dz, d = d }
    end
  end
  -- IDs dos nós do herói (esqueleto, arma, tocha, FX) que estão SOBRE a
  -- câmara — o herói invisível viaja nela. Não são alvos.
  local local_nodes = {}
  for _, e in ipairs(viber.player_nearby(4.0, 64) or {}) do
    local_nodes[e.id] = true
  end
  -- `viber.nearby` está centrado no controller, que o `update_fly` põe
  -- 30 m à frente: os alvos reais chegam à lista antes dos nós do herói.
  for _, e in ipairs(viber.nearby(LOOK_AT_RADIUS, 64) or {}) do
    if not local_nodes[e.id] then
      consider(e.id, e.name, e.x, e.y, e.z, LOOK_AT_MIN_M)
    end
  end
  -- Criaturas de spawner não têm nome (`nearby` não as vê) e estão
  -- legitimamente perto.
  for _, h in ipairs(viber.hostiles(LOOK_AT_RADIUS) or {}) do
    consider(h.id, h.kind or "criatura", h.x, h.y, h.z, LOOK_AT_MIN_M)
  end
  if not best then
    toast("look-at: nada novo à frente (raio " .. math.floor(LOOK_AT_RADIUS) .. " m).")
    return
  end
  st.look_target = best.id
  fly.yaw = math.deg(math.atan2(best.dx, best.dz))
  fly.pitch = math.deg(math.asin(clamp(best.dy / best.d, -1.0, 1.0)))
  toast("a olhar para " .. tostring(best.name) .. " (" .. math.floor(best.d) .. " m)")
end

function on_update(dt)
  local st = viber.state()
  if not st.dev then
    -- Reavaliado até ligar (fica ligado): no arranque directo
    -- (`--scene main --entry dev`) é verdade no 1.º frame. `viber_dev` é o
    -- atalho de QA pela bridge: o REPL não drena comandos de script (um
    -- `change_scene` de lá é um no-op), mas PARTILHA a VM — `_G.viber_dev =
    -- true` liga as ferramentas sem reiniciar a engine. Nenhum caminho
    -- normal escreve esse global (precisa de `--bridge` + REPL).
    st.dev = dev.skip_intro() == true or rawget(_G, "viber_dev") == true
    if st.dev then
      toast("[F1] voar · [F2] invulnerável")
    end
  end
  if not st.dev then
    return
  end
  -- Reaplicado todos os frames: um hot-reload de `lib/rules.lua` recria o
  -- `M.god = false` do módulo e a invulnerabilidade cairia em silêncio.
  apply_god()
  -- Troca de cena com o voo ligado (portal, título): herói e câmara são
  -- autoload e sobrevivem, mas o voo tem de acabar antes da cena nova — o
  -- `place_player` da chegada seria desfeito pelo `set_pose` do frame
  -- seguinte e a câmara livre tapava o enquadramento da cena.
  for _, ev in ipairs(viber.events()) do
    if ev.type == "scene_unloading" and st.fly then
      stop_fly()
      break
    end
  end
  -- Menus/mapa/viagem/cinemática roubam o input (o mesmo portão do jogo).
  if viber.menu_open() then
    return
  end

  if viber.input.pressed("f1") then
    if st.fly then
      stop_fly()
    else
      start_fly()
    end
    return
  end
  if viber.input.pressed("f2") then
    st.god = not (st.god == true)
    apply_god()
    toast(st.god and "invulnerável ligado." or "invulnerável desligado.")
  end

  if st.fly then
    if viber.input.pressed("tab") then
      look_at()
    end
    update_fly(dt)
  end
end
