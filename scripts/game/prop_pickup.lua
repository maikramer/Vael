--@export loot: enum(life,wood,gold) = "life"
--@export floor: number = 0
--@export height: number = 0.45
local rules = viber.load("lib/rules.lua")

-- Credit on contact. Leave a capped stack on the floor for the return trip.
local function collect(kind, hp, max_hp)
  if kind == "life" and hp < max_hp then
    local healed = math.min(25, max_hp - hp)
    if viber.vael.heal_player(healed) then return "+" .. math.ceil(healed) .. " HP", "heal" end
  else
    local item = kind == "life" and "potion" or kind
    if rules.vault_add(item, 1) > 0 then
      return "+1 " .. rules.item_label(item), item == "gold" and "coin" or "loot"
    end
  end
end

function on_update(dt)
  if viber.menu_open() or viber.ui.is_open("title") then return end
  local st = viber.state()
  if not st.t then
    local exists, x, _, z = viber.entity_position()
    if not exists then return end
    st.x, st.z = x, z
    st.floor = tonumber(viber.prop("floor"))
    st.height = tonumber(viber.prop("height"))
    st.kind = viber.prop("loot")
    st.t = 0
  end
  st.t = st.t + dt
  local ok, px, py, pz = viber.player_position()
  if st.taken then
    local t = math.clamp((st.t - st.taken) / 0.34, 0, 1)
    if t >= 1 then viber.despawn_self() return end
    -- Curva acelerada com arco e espiral elegante até ao peito do herói.
    local u = t * t * (2.2 - 1.2 * t)
    local x, z = st.x, st.z
    local ty = st.y + 0.95
    if ok then
      local dx, dz = px - st.x, pz - st.z
      local swirl = math.sin(t * math.pi) * 0.28
      x = st.x + dx * u - dz * swirl * 0.35
      z = st.z + dz * u + dx * swirl * 0.35
      ty = py + 1.05
    end
    local arc = math.sin(t * math.pi) * 0.55
    local y = st.y + (ty - st.y) * u + arc
    viber.set_pose(x, y, z, st.t * 7.5)
    -- Breve pulso elástico no arranque antes de encolher no peito do herói.
    local scale = t < 0.25 and (1.0 + 0.22 * math.sin(t / 0.25 * math.pi))
        or math.max(0.05, 1.0 - ((t - 0.25) / 0.75) ^ 2)
    viber.set_scale(scale)
    return
  end
  -- Pop elástico ao sair do destrutível, pequeno quique no chão e levitação viva.
  local hover = 0.23 + 0.065 * math.sin(st.t * 3.8)
  local arc_y = st.height + 2.55 * st.t - 5.4 * st.t * st.t
  local bounce = 0
  if st.t > 0.48 and st.t < 0.78 then
    local bt = (st.t - 0.48) / 0.30
    bounce = math.sin(bt * math.pi) * 0.14
  end
  st.y = st.floor + math.max(hover + bounce, arc_y)
  -- Leve atração magnética quando o herói se aproxima (mantém o raio de coleta exato).
  if ok and st.t >= 0.55 then
    local dx, dz = px - st.x, pz - st.z
    local dist_sq = dx * dx + dz * dz
    if dist_sq > 0.95 * 0.95 and dist_sq < 2.25 * 2.25 and math.abs(py - st.y) <= 1.3 then
      local pull = math.min(1.0, dt * 3.8)
      st.x = st.x + dx * pull
      st.z = st.z + dz * pull
    end
  end
  local breath = 1.0 + 0.055 * math.sin(st.t * 5.0)
  viber.set_pose(st.x, st.y, st.z, st.t * 2.7)
  viber.set_scale(breath)
  if st.t < 0.55 or not ok or (px-st.x)^2 + (pz-st.z)^2 > 0.95^2
      or math.abs(py-st.y) > 1.1 then return end
  local _, hp, max_hp = rules.player_hp()
  if hp <= 0 then return end
  local text, sound = collect(st.kind, hp, max_hp)
  if text then
    st.taken = st.t
    viber.sound(sound, {x = st.x, y = st.y, z = st.z})
    viber.burst("sparkle", st.x, st.y + 0.15, st.z, 16)
    if viber.vael and viber.vael.collect_burst then
      viber.vael.collect_burst(st.kind, st.x, st.y + 0.12, st.z, 4)
    end
    viber.damage_number(text, {x = px, y = py + 1., z = pz, color = st.kind == "life" and "#ff879e" or "#ffd76c"})
  end
end
