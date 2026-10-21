-- game/harvest.lua: a COLHEITA é do jogo, em Lua.
--
-- A engine ficou só com os DADOS que o mundo autorou (`destructible=` /
-- `<ResourceNode>`) e com primitivas genéricas: `viber.destructibles` lê o
-- que está ao alcance, `viber.prompt` publica o verbo, `viber.play_clip`
-- toca o golpe, `viber.burst`/`viber.sound` fazem o FX, e
-- `viber.entity_despawn`/`viber.topple` removem o prop. Não há [J] nativo,
-- nem contador de golpes, nem loot na engine.
--
-- Host: `world/gameplay.xml` (`tag="always-active"` — a colheita não pode
-- congelar no LOD de 45 m).

local rules = viber.load("lib/rules.lua")

local SWING_COOLDOWN = 0.4
local RANGE_DEFAULT = 3.5
local SPEED = 1.4
local IMPACT_FRACTION = 0.35
local IMPACT_MIN, IMPACT_MAX = 0.12, 0.45
local FALLBACK_DURATION = 0.5
local HARVEST_XP = 30

local LABEL = { fall = "Cortar" }
local CLIP = { fall = "chop", mine = "mine" }
local HIT_SFX = { chop = "chop_hit", mine = "mine_hit" }
local BREAK_SFX = { chop = "chop_break", mine = "mine_break" }
local HIT_BURST = { fall = "leaves", mine = "sparks" }
local BREAK_BURST = { fall = "leaves", mine = "ground-dust" }

local function tool_of(style)
  return style == "fall" and "chop" or "mine"
end

local function label_of(style)
  return LABEL[style] or "Minerar"
end

local function clip_of(style)
  return CLIP[tool_of(style)]
end

local function duration_of(clip)
  local d = viber.clip_duration and viber.clip_duration(clip)
  if d and d > 0 then
    return d
  end
  return FALLBACK_DURATION
end

local function impact_delay(clip)
  local d = duration_of(clip)
  local delay = d * IMPACT_FRACTION / SPEED
  return math.min(IMPACT_MAX, math.max(IMPACT_MIN, delay))
end

-- O prompt de outro [J] (um `ScriptInteraction` mais próximo) tem prioridade
-- sobre a colheita — lê-se o prompt que a engine publicou este frame.
local function other_j_prompt(style)
  local key = viber.ui.get("prompt.key")
  local label = viber.ui.get("prompt.label")
  return key == "J" and label ~= "" and label ~= label_of(style)
end

-- Prop mais próximo DENTRO do alcance efetivo autorado (a engine já aplica a
-- escala global de interação; `effective_range` é o número final).
local function nearest()
  local list = viber.destructibles(RANGE_DEFAULT * 2) or {}
  local best
  for _, prop in ipairs(list) do
    local range = prop.effective_range or (prop.range or RANGE_DEFAULT) * 0.5
    if (prop.distance or 0) <= range then
      if not best or prop.distance < best.distance then
        best = prop
      end
    end
  end
  return best
end

local function publish(prop)
  if prop then
    viber.prompt("J", label_of(prop.style))
  else
    viber.prompt("", "")
  end
end

local function swing(st, prop)
  local clip = clip_of(prop.style)
  st.cooldown = SWING_COOLDOWN
  st.swing = {
    t = impact_delay(clip),
    id = prop.id,
    hits = prop.hits or 3,
    style = prop.style or "burst",
    kind = prop.kind or "",
    amount = prop.amount or 0,
    popup = prop.popup or "",
    name = prop.name or "",
    x = prop.x,
    y = prop.y,
    z = prop.z,
  }
  local pid = viber.player_id()
  if pid then
    viber.play_clip(clip, { speed = SPEED, id = pid })
  end
  viber.sound(HIT_SFX[tool_of(prop.style)] or "whoosh", {
    x = prop.x, y = prop.y, z = prop.z,
  })
end

local function break_prop(st, sw)
  local style = sw.style or "burst"
  local tool = tool_of(style)
  viber.burst(BREAK_BURST[tool] or "ground-dust", sw.x, (sw.y or 0) + 0.8, sw.z, 24)
  viber.sound(BREAK_SFX[tool] or "chop_break", { x = sw.x, y = sw.y, z = sw.z })
  if sw.popup ~= "" then
    viber.damage_number(sw.popup, {
      color = "#ffd35a", x = sw.x, y = (sw.y or 0) + 1.4, z = sw.z,
    })
  end
  -- Loot e XP: regra do jogo, não da engine. `vault_add` já sincroniza os
  -- objetivos `collect` das missões.
  if sw.kind ~= "" and sw.amount > 0 then
    rules.vault_add(sw.kind, sw.amount)
  end
  rules.add_xp(HARVEST_XP)
  -- Persistência: o load volta a spawnar o prop; o controller apaga-o no
  -- arranque se o nome estiver no save (props nomeados).
  if sw.name ~= "" then
    viber.game()["harvested." .. sw.name] = true
  end
  if style == "fall" then
    viber.topple(sw.id)
  else
    viber.entity_despawn(sw.id)
  end
  st.hits[sw.id] = nil
end

local function impact(st)
  local sw = st.swing
  st.swing = nil
  if not sw then
    return
  end
  local hits = (st.hits[sw.id] or 0) + 1
  if hits >= sw.hits then
    break_prop(st, sw)
    return
  end
  st.hits[sw.id] = hits
  local tool = tool_of(sw.style)
  viber.burst(HIT_BURST[tool] or "sparks", sw.x, (sw.y or 0) + 1.0, sw.z, 7)
  if viber.hit_flash then
    viber.hit_flash(sw.id)
  end
end

-- No arranque, apaga props nomeados que o save diz estarem colhidos.
local function reap_saved()
  local game = viber.game()
  for key in pairs(game) do
    if type(key) == "string" and key:sub(1, 10) == "harvested." then
      local name = key:sub(11)
      local id = name ~= "" and viber.find(name) or nil
      if id then
        viber.entity_despawn(id)
      end
    end
  end
end

function on_update(dt)
  local st = viber.state()
  st.hits = st.hits or {}
  if not st.reaped then
    st.reaped = true
    reap_saved()
  end
  st.cooldown = math.max(0, (st.cooldown or 0) - dt)
  if st.swing then
    st.swing.t = st.swing.t - dt
    if st.swing.t <= 0 then
      impact(st)
    end
  end
  if viber.menu_open() then
    viber.prompt(nil)
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    viber.prompt(nil)
    return
  end
  local prop = nearest()
  if prop and other_j_prompt(prop.style) then
    prop = nil
  end
  publish(prop)
  local pressed = viber.input.pressed("j") or viber.input.pressed("lmb")
  if not pressed or not prop then
    return
  end
  if st.cooldown > 0 or st.swing then
    return
  end
  swing(st, prop)
end
