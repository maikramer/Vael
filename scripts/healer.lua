local rules = vael.load("lib/rules.lua")
-- healer.lua: [E] gira o player e cura o herói; HP cheio recusa com gesto "no".
local RANGE = 3.5

-- O prompt acompanha o estado: "Ser curado" com a vida cheia (ou caído) era
-- um verbo que o NPC recusava logo a seguir.
local function label_for(hp, max)
  if hp <= 0 then
    return "Nada a curar"
  end
  if hp >= max then
    return "Já estás curado"
  end
  return "Ser curado"
end

function on_update(dt)
  local st = vael.state() -- POR ENTIDADE (no top-level era partilhado)
  local ok, hp, max = rules.player_hp()
  local label = (ok and label_for(hp, max)) or "Ser curado"
  if not st.ready then
    st.ready = true
    st.prompt = label
    vael.set_interaction(label, "e", RANGE)
  elseif st.prompt ~= label then
    st.prompt = label
    vael.set_interaction(label, "e", RANGE)
  end
  if vael.menu_open() or not vael.interacted("e") then
    return
  end
  vael.face_player()
  if ok and hp <= 0 then
    vael.gesture("no")
    vael.toast("“Já não há o que curar.”")
    return
  end
  if ok and hp >= max then
    vael.gesture("no")
    vael.toast("“Já estás curado.”")
    return
  end
  vael.gesture("yes")
  rules.flask_refill()
  rules.status_clear()
  rules.heal_player(999)
  vael.sound("heal")
  vael.toast("“Que as graças te acompanhem.” — cura completa")
end
