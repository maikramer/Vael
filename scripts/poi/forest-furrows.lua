local rules = vael.load("lib/rules.lua")
local talk = vael.load("ui/say.lua")
function on_update(_dt)
  local game = vael.game()
  if not game["q.forest_witch"] then return end
  local st = vael.state()
  if not st.ready then
    vael.set_interaction("Examinar os sulcos", "e", 6)
    st.ready = true
  end
  if vael.menu_open() or talk.active() or not vael.interacted("e") then return end
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  game["campaign.seen.furrows"] = true
  talk.say("Dois sulcos paralelos. Estreitos demais para rodas. Desenhas as marcas na margem da Nota.", 6)
end
