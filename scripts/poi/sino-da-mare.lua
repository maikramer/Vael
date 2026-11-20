-- O sino e a constelação partilham o mesmo relógio de animação.
function on_update(_dt)
  local st = vael.state()
  if not st.ready then
    vael.set_interaction("Tocar o sino da maré", "e", 4.2)
    st.ready = true
  end
  if vael.menu_open() or not vael.interacted("e") then return end
  local now = vael.time()
  if now < (st.next_ring or 0) then return end
  st.next_ring = now + 16.2
  vael.play_clip("canto", { duration = 16.0, blend = 0.02 })
  local x, y, z = vael.position()
  vael.burst("sparkle", x, y + 1.65, z, 7)
  local voice = "sino-da-mare-" .. tostring(vael.self_id())
  vael.sound_loop(voice, "world/canto_da_mare.ogg", {x=x, y=y+1.4, z=z, volume=.9})
  vael.after(16, function() vael.sound_stop(voice) end)
end
