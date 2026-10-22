local rules = viber.load("lib/rules.lua")
-- watch-guard.lua: [E] cumpre com gesto + som; a 1.ª vez dá XP (portado do TS).
local once = viber.load("lib/once.lua")
local function prompt_for(done)
  return done and "Cumprimentar o guarda" or "Falar com o guarda"
end
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    st.done = once.taken()
    viber.set_interaction(prompt_for(st.done), "e", 3.5)
  end
  local taken = once.taken()
  if taken ~= st.done then
    st.done = taken
    viber.set_interaction(prompt_for(taken), "e", 3.5)
  end
  if viber.menu_open() then return end
  if viber.interacted("e") then
    viber.face_player()
    -- "salute" 1.º; rigs sem o clip caem para "yes"/"talk" (fuzzy do gesto)
    viber.gesture("salute,yes,talk")
    viber.sound("ui")
    if not st.done then
      st.done = true
      once.mark()
      viber.set_interaction(prompt_for(true), "e", 3.5)
      rules.add_xp(5)
      viber.toast("“Mantenha os olhos na estrada, viajante.” (+5 XP)")
    end
  end
end
