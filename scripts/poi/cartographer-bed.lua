local rules = vael.load("lib/rules.lua")

function on_update()
  local st = vael.state()
  if not st.ready then
    st.ready = true
    vael.set_interaction("Descansar na tua cama", "e", 3.5)
  end
  if vael.menu_open() or not vael.interacted("e") then return end
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  rules.heal_player(999)
  rules.flask_refill()
  rules.status_clear()
  vael.sound("heal")
  vael.toast("Descansaste em casa. Cantil cheio.")
end
