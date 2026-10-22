local rules = viber.load("lib/rules.lua")
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
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  local ok, hp, max = rules.player_hp()
  local label = (ok and label_for(hp, max)) or "Ser curado"
  if not st.ready then
    st.ready = true
    st.prompt = label
    viber.set_interaction(label, "e", RANGE)
  elseif st.prompt ~= label then
    st.prompt = label
    viber.set_interaction(label, "e", RANGE)
  end
  if viber.menu_open() or not viber.interacted("e") then
    return
  end
  viber.face_player()
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
