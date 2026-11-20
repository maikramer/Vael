-- As fitas permanecem visíveis sem áudio. Cada uma responde apenas na aproximação.
--@export clearance: string = ""
local talk = vael.load("ui/say.lua")
function on_update(_dt)
  local game = vael.game()
  if game["q.forest_witch"] ~= "active" or game.nota_witch or vael.menu_open() then return end
  local has, px, _, pz = vael.player_position()
  if not has then return end
  local x, y, z = vael.position()
  local distance = (px - x)^2 + (pz - z)^2
  local st = vael.state()
  if distance > 30^2 then st.heard = false end
  if distance < 19^2 and not st.heard then
    st.heard = true
    vael.burst("sparkle", x, y + 1.4, z, 8)
    local voice = "vessa-ribbon-" .. tostring(vael.self_id())
    vael.sound_loop(voice, "ambient/vessa_lullaby.ogg", { x = x, y = y, z = z, volume = 0.85 })
    vael.after(3.5, function() vael.sound_stop(voice) end)
    if vael.prop("clearance") == "witch" then
      vael.ring(x, z, 2.0, "#b04cff")
      talk.say("A maldição cerca a última fita. Sai do círculo antes da queda!")
      vael.after(1.1, function() vael.burst("smoke", x, y + 0.25, z, 14) end)
    else
      talk.say("A cantiga responde mais a norte. Outra fita aponta para a clareira.")
    end
  end
end
