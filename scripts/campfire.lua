local rules = viber.load("lib/rules.lua")
-- campfire.lua: sentar/descansar na fogueira da praça (cura + toast).
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    viber.set_interaction("Descansar", "e", 2.5)
  end
  if viber.menu_open() or not viber.interacted("e") then
    return
  end
  local ok, hp, max_hp = rules.player_hp()
  if ok and hp <= 0 then
    return
  end
  local healed = math.min(25, math.max(0, (max_hp or 0) - (hp or 0)))
  if healed < 1 then
    viber.toast("O calor não tem mais o que restaurar.")
    return
  end
  rules.heal_player(healed)
  local has, px, py, pz = viber.player_position()
  if has then
    rules.set_spawn(px, py, pz)
  end
  viber.toast("O calor da fogueira restaura " .. math.floor(healed) .. " de vida.")
end
