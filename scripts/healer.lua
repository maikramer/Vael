local rules = viber.load("lib/rules.lua")
-- healer.lua: [E] gira o player e cura o herói; HP cheio recusa com gesto "no".
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    viber.set_interaction("Ser curado", "e", 3.5)
  end
  if viber.menu_open() or not viber.interacted("e") then
    return
  end
  viber.face_player()
  local ok, hp, max = viber.player_hp()
  if ok and hp <= 0 then
    viber.gesture("no")
    viber.toast("“Já não há o que curar.”")
    return
  end
  if ok and hp >= max then
    viber.gesture("no")
    viber.toast("“Já estás curado.”")
    return
  end
  viber.gesture("yes")
  rules.heal_player(999)
  viber.sound("heal")
  viber.toast("“Que as graças te acompanhem.” — cura completa")
end
