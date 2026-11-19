-- As fitas permanecem visíveis sem áudio. Cada uma responde apenas na aproximação.
--@export clearance: string = ""
local talk = viber.load("ui/say.lua")
function on_update(_dt)
  local game = viber.game()
  if game["q.forest_witch"] ~= "active" or game.nota_witch or viber.menu_open() then return end
  local has, px, _, pz = viber.player_position()
  if not has then return end
  local x, y, z = viber.position()
  local distance = (px - x)^2 + (pz - z)^2
  local st = viber.state()
  if distance > 30^2 then st.heard = false end
  if distance < 19^2 and not st.heard then
    st.heard = true
    viber.burst("sparkle", x, y + 1.4, z, 8)
    local voice = "vessa-ribbon-" .. tostring(viber.self_id())
    viber.sound_loop(voice, "ambient/vessa_lullaby.ogg", { x = x, y = y, z = z, volume = 0.85 })
    viber.after(3.5, function() viber.sound_stop(voice) end)
    if viber.prop("clearance") == "witch" then
      viber.ring(x, z, 2.0, "#b04cff")
      talk.say("A maldição cerca a última fita. Sai do círculo antes da queda!")
      viber.after(1.1, function() viber.burst("smoke", x, y + 0.25, z, 14) end)
    else
      talk.say("A cantiga responde mais a norte. Outra fita aponta para a clareira.")
    end
  end
end
