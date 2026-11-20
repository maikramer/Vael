local rules = vael.load("lib/rules.lua")
-- well.lua: beber restaura 25 HP (o mesmo descanso da fogueira). Antes era
-- um toast único e a vida não mexia.
function on_update(dt)
  local st = vael.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    vael.set_interaction("Beber", "e", 3.0)
  end
  if vael.menu_open() or not vael.interacted("e") then
    return
  end
  local ok, hp, max_hp = rules.player_hp()
  if ok and hp <= 0 then
    return
  end
  local healed = math.min(25, math.max(0, (max_hp or 0) - (hp or 0)))
  if healed < 1 then
    vael.toast("A vida já está cheia.")
    return
  end
  rules.heal_player(healed)
  -- Sem SFX a cura do poço era a única do mundo sem confirmação audível.
  vael.sound("heal")
  vael.toast("A água fresca restaura o ânimo (+" .. math.floor(healed) .. " HP).")
end
