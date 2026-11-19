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
    local t = math.clamp((st.t - st.taken) / 0.32, 0, 1)
    if t >= 1 then viber.despawn_self() return end
    local x, z = st.x, st.z
    if ok then x, z = x + (px-x)*t, z + (pz-z)*t end
    viber.set_pose(x, st.y + t * 0.85, z, st.t * 4)
    viber.set_scale(math.max(0.05, 1-t*t))
    return
  end
  -- Pop out of the container, settle just above the floor, then hover.
  local hover = 0.21 + 0.055 * math.sin(st.t * 3.6)
  st.y = st.floor + math.max(hover, st.height + 2.35*st.t - 4.9*st.t*st.t)
  viber.set_pose(st.x, st.y, st.z, st.t * 2.4)
  if st.t < 0.55 or not ok or (px-st.x)^2 + (pz-st.z)^2 > 0.95^2
      or math.abs(py-st.y) > 1.1 then return end
  local _, hp, max_hp = rules.player_hp()
  if hp <= 0 then return end
  local text, sound = collect(st.kind, hp, max_hp)
  if text then
    st.taken = st.t
    viber.sound(sound, {x = st.x, y = st.y, z = st.z})
    viber.burst("sparkle", st.x, st.y + 0.15, st.z, 12)
    viber.damage_number(text, {x = px, y = py + 1., z = pz, color = st.kind == "life" and "#ff879e" or "#ffd76c"})
  end
end
