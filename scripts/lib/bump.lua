-- lib/bump.lua: reação a esbarrão do herói (o "bump" à Assassin's Creed).
--
-- O runtime reporta os hits do character controller do herói aos scripts das
-- entidades tocadas: `on_collision_enter/stay/exit(other)` com
-- `other = {id, name?, x, y, z}`. Encostar devagar não faz nada; esbarrar a
-- andar dá tropeço + passo ao lado (clips `stumble`), sprint/dash dá queda +
-- levantar (`knockdown`/`getup`); insistir encostado desloca o NPC.
--
-- Uso no script (um só FSM por entidade, `vael.state()`):
--   local bump = vael.load("lib/bump.lua")
--   function on_collision_enter(o) bump.enter(vael.state(), o) end
--   function on_collision_stay(o, dt) bump.stay(vael.state(), o, dt) end
--   function on_collision_exit(o) bump.exit(vael.state(), o) end
--   -- no topo do on_update, ANTES do resto:
--   if bump.tick(vael.state(), vael.time()) then return end
--   bump.sample(vael.state(), vael.time())  -- mede a velocidade do herói
--
-- Os clips vivem nos GLB dos NPCs (motion3d; ver tools/make_lods). O herói
-- anda a 4,0 m/s e sprint a 6,0 (src/player.rs); a esquiva/lunge em burst
-- passa bem acima — tudo o que é >= RUN derruba.

local B = {}

local WALK = 1.5        -- m/s: abaixo disto é encostar, não esbarrar
local RUN = 5.0         -- m/s: acima disto é empurrão (queda + getup)
local TELEPORT = 25.0   -- m/s: acima disto é teleporte/corte de cena
local COOLDOWN = 1.8    -- s entre reações
local YIELD_AFTER = 0.35 -- s de contacto a andar até ceder
local YIELD_HOLD = 0.9  -- s de contacto parado/insistido até ceder
local YIELD_COOLDOWN = 0.9
local STEP = 1.3        -- m do passo ao lado
local STUMBLE_SECS = 1.15
local KNOCKDOWN_SECS = 1.6
local GETUP_SECS = 1.6

function B.hero_is(other)
  local pid = vael.player_id and vael.player_id()
  return pid ~= nil and other ~= nil and other.id == pid
end

-- Props opcionais: scripts diferentes exportam conjuntos diferentes; um
-- `vael.prop` de chave não exportada é ERRO (não nil) e matava o handler.
local function prop_num(name, default)
  local ok, value = pcall(vael.prop, name)
  if not ok then return default end
  return tonumber(value) or default
end

-- Alvo do passo: clamp ao raio de passeio do prop (0/ausente = balcão: fica).
local function home_clamped(tx, tz)
  local radius = prop_num("radius", 0)
  if radius <= 0 then
    local x, _, z = vael.position()
    return x, z
  end
  local ok, hx, hz = pcall(vael.home)
  if not ok or not hx or not hz then return tx, tz end
  local dx, dz = tx - hx, tz - hz
  local d = math.sqrt(dx * dx + dz * dz)
  if d <= radius or d < 1e-3 then return tx, tz end
  return hx + dx / d * radius, hz + dz / d * radius
end

-- Perpendicular ao vetor NPC→herói, do lado que afasta mais do herói.
local function side_step_target(ox, oz)
  local x, _, z = vael.position()
  local dx, dz = x - ox, z - oz
  local d = math.sqrt(dx * dx + dz * dz)
  if d < 1e-3 then dx, dz, d = 1, 0, 1 end
  local tx1, tz1 = home_clamped(x - dz / d * STEP, z + dx / d * STEP)
  local tx2, tz2 = home_clamped(x + dz / d * STEP, z - dx / d * STEP)
  local d1 = (tx1 - ox) ^ 2 + (tz1 - oz) ^ 2
  local d2 = (tx2 - ox) ^ 2 + (tz2 - oz) ^ 2
  if d1 >= d2 then return tx1, tz1 end
  return tx2, tz2
end

local function busy(st, now)
  return st.talking or (st.bump_until and now < st.bump_until) or now < (st.bump_cd or 0)
end

local function react_bump(st, other)
  local now = vael.time()
  if busy(st, now) then return end
  if (st.pspeed_peak or 0) < WALK then return end
  local x, _, z = vael.position()
  local ox = other and other.x or x
  local oz = other and other.z or z
  if st.pspeed_peak >= RUN then
    -- Empurrão: cai para trás (clip in-place) e dá meio passo largo.
    local dx, dz = x - ox, z - oz
    local d = math.sqrt(dx * dx + dz * dz)
    if d < 1e-3 then dx, dz, d = 1, 0, 1 end
    local tx, tz = home_clamped(x + dx / d * 0.5, z + dz / d * 0.5)
    st.bump_until = now + KNOCKDOWN_SECS
    st.bump_next = "getup"
    st.bump_target = { tx, tz }
    st.bump_speed = 0.9
    st.bump_cd = now + KNOCKDOWN_SECS + GETUP_SECS + COOLDOWN
    vael.play_clip("knockdown")
    return
  end
  local tx, tz = side_step_target(ox, oz)
  st.bump_until = now + STUMBLE_SECS
  st.bump_target = { tx, tz }
  st.bump_speed = 1.6
  st.bump_cd = now + STUMBLE_SECS + COOLDOWN
  vael.play_clip("stumble")
end

local function react_yield(st, other)
  local now = vael.time()
  if busy(st, now) then return end
  local ox = other and other.x
  local oz = other and other.z
  if not ox then return end
  local tx, tz = side_step_target(ox, oz)
  st.bump_until = now + 0.55
  st.bump_target = { tx, tz }
  st.bump_speed = 1.3
  st.bump_cd = now + 0.55 + YIELD_COOLDOWN
end

-- Velocidade do herói por amostragem (não há vael.player_velocity): pico com
-- decaimento, porque o KCC trava o herói no instante do contacto.
function B.sample(st, now)
  local has, px, _, pz = vael.player_position()
  if not has then return end
  if st.last_px and st.last_pt then
    local step = now - st.last_pt
    if step > 1e-3 and step < 0.5 then
      local speed = math.sqrt((px - st.last_px) ^ 2 + (pz - st.last_pz) ^ 2) / step
      if speed < TELEPORT then
        st.pspeed = speed
        st.pspeed_peak = math.max(speed, (st.pspeed_peak or 0) * math.exp(-step / 0.25))
      else
        st.pspeed_peak = 0
      end
    end
  end
  st.last_px, st.last_pz, st.last_pt = px, pz, now
end

function B.enter(st, other)
  if not B.hero_is(other) then return end
  react_bump(st, other)
end

function B.stay(st, other, dt)
  if not B.hero_is(other) then return end
  st.rub = (st.rub or 0) + (dt or 0)
  local moving = (st.pspeed_peak or 0) >= 0.6
  if st.rub < (moving and YIELD_AFTER or YIELD_HOLD) then return end
  st.rub = 0
  react_yield(st, other)
end

function B.exit(st, other)
  if not B.hero_is(other) then return end
  st.rub = 0
end

-- Avança a fase de reação (chamar no topo do on_update). `true` = a fase
-- domina o frame: o FSM normal do script não corre.
function B.tick(st, now)
  if st.bump_until and now < st.bump_until then
    if st.bump_target then
      vael.move_towards(st.bump_target[1], st.bump_target[2], st.bump_speed or 1.3)
    end
    return true
  elseif st.bump_until then
    st.bump_until, st.bump_target = nil, nil
    if st.bump_next == "getup" then
      st.bump_next = nil
      vael.play_clip("getup")
      st.bump_until = now + GETUP_SECS
    end
    return true
  end
  return false
end

return B
