-- game/spawns.lua: o mundo REPÕE-SE e a noite tem dentes.
--
-- 1. RENASCER — o `lib/fsm.lua` regista cada abate em `viber.game().respawns`
--    ({proto, x, z, at}); passados uns minutos, e com o herói longe da vista,
--    a criatura volta ao covil pelo `<Prototype id="mob-…">` de
--    `world/gameplay.xml`. A fila é uma TABELA: não vai no save e morre num
--    load ou numa troca de cena (a cena recria as criaturas todas).
--    A serra do Ogro é excepção: enquanto ele viver, os Picos NÃO renascem —
--    o portão dele só abre com a banda sul em silêncio.
--
-- 2. SOMBRAS DA NOITE — entre as 20h e as 5h, fora da vila, um par de
--    sombras nasce perto do herói de tempos a tempos. Desfazem-se ao
--    amanhecer (`fades_at_dawn`) e não renascem.
--
-- Host: `world/gameplay.xml` (`tag="always-active"`).

local rules = viber.load("lib/rules.lua")
local clock = viber.load("lib/clock.lua")

local TICK = 1.0
local SIGHT = 70            -- o herói tem de estar mais longe do que isto
local PER_TICK = 2          -- renascimentos por tique (sem rajadas)
local VILLAGE_RADIUS = 140  -- as sombras não entram no vale da vila
local NIGHT_EVERY = { 70, 120 }
local NIGHT_MAX_ALIVE = 4
local NIGHT_PACK = 2

local acc = 0

local function playing()
  local scene = viber.scene_name and viber.scene_name() or nil
  if scene ~= nil and scene ~= "main" then
    return false
  end
  if viber.ui and viber.ui.is_open and viber.ui.is_open("title") then
    return false
  end
  return not viber.menu_open() and not viber.cinematic_playing()
end

-- Banda sul (Picos): a mesma partição de `rules.alive_in_region(2)`.
local function in_peaks(x, z)
  if math.abs(x) <= 120 and math.abs(z) <= 120 then
    return false
  end
  return math.abs(z) > math.abs(x) and z < 0
end

local function tick_respawns(game, px, pz)
  local queue = game.respawns
  if type(queue) ~= "table" or #queue == 0 then
    return
  end
  local now = viber.time()
  local ogre_alive = game["boss_dead.boss"] ~= true
  local done = 0
  local i = 1
  while i <= #queue and done < PER_TICK do
    local r = queue[i]
    local drop, spawn = false, false
    if type(r) ~= "table" or not r.proto or not r.x or not r.z then
      drop = true
    elseif ogre_alive and in_peaks(r.x, r.z) then
      drop = true
    elseif (r.at or 0) <= now then
      local dx, dz = r.x - px, r.z - pz
      if dx * dx + dz * dz > SIGHT * SIGHT then
        drop, spawn = true, true
      end
    end
    if spawn then
      pcall(viber.spawn_prototype, r.proto, r.x, r.z)
      done = done + 1
    end
    if drop then
      table.remove(queue, i)
    else
      i = i + 1
    end
  end
end

local function night_shades_alive()
  local n = 0
  for _, h in ipairs(rules.hostiles_all()) do
    if h.kind == "shade-night" then
      n = n + 1
    end
  end
  return n
end

local function tick_night(st, px, py, pz)
  if not clock.is_night() then
    st.night_t, st.night_said = nil, false
    return
  end
  if px * px + pz * pz < VILLAGE_RADIUS * VILLAGE_RADIUS then
    return
  end
  -- 1.º par da noite sai cedo; depois, a intervalos largos.
  st.night_t = (st.night_t or 12) - TICK
  if st.night_t > 0 then
    return
  end
  st.night_t = NIGHT_EVERY[1] + math.random() * (NIGHT_EVERY[2] - NIGHT_EVERY[1])
  -- A meio de uma luta (ou com sombras a mais) a noite espera.
  if #rules.hostiles(18) >= 2 or night_shades_alive() >= NIGHT_MAX_ALIVE then
    return
  end
  local spawned = 0
  local base = math.random() * math.pi * 2
  for k = 0, 7 do
    if spawned >= NIGHT_PACK then
      break
    end
    local a = base + k * (math.pi / 4)
    local r = 16 + math.random() * 5
    local x, z = px + math.cos(a) * r, pz + math.sin(a) * r
    local wet = viber.in_water and viber.in_water(x, z)
    local gy = viber.ground_below and viber.ground_below(x, py + 30.0, z) or py
    -- Sem poços nem falésias: o ponto tem de estar perto da cota do herói.
    if not wet and gy and math.abs(gy - py) <= 6 then
      if pcall(viber.spawn_prototype, "mob-shade-night", x, z) then
        viber.burst("magic", x, gy + 1.0, z, 10)
        spawned = spawned + 1
      end
    end
  end
  if spawned > 0 and not st.night_said then
    st.night_said = true
    viber.toast("As sombras saem dos pinhais — a noite não é tua.")
    viber.sound("growl", { x = px, y = py, z = pz })
  end
end

function on_update(dt)
  local st = viber.state()
  local game = viber.game()
  for _, ev in ipairs(viber.events()) do
    if ev.type == "scene_loaded" then
      -- A cena recriou todas as criaturas: renascer por cima duplicava-as.
      game.respawns = {}
      game.summoned = {}
    end
  end
  acc = acc + dt
  if acc < TICK then
    return
  end
  acc = 0
  if not playing() then
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return
  end
  local has, px, py, pz = viber.player_position()
  if not has then
    return
  end
  tick_respawns(game, px, pz)
  tick_night(st, px, py, pz)
end
