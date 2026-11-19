local rules = viber.load("lib/rules.lua")
local talk = viber.load("ui/say.lua")
function on_update(_dt)
  local game = viber.game()
  if not game["q.forest_witch"] then return end
  local st = viber.state()
  if not st.ready then
    viber.set_interaction("Examinar os sulcos", "e", 6)
    st.ready = true
  end
  if viber.menu_open() or talk.active() or not viber.interacted("e") then return end
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  game["campaign.seen.furrows"] = true
  talk.say("Dois sulcos paralelos. Estreitos demais para rodas. Desenhas as marcas na margem da Nota.", 6)
end
