-- O sino e a constelação partilham o mesmo relógio de animação.
function on_update(_dt)
  local st = viber.state()
  if not st.ready then
    viber.set_interaction("Tocar o sino da maré", "e", 4.2)
    st.ready = true
  end
  if viber.menu_open() or not viber.interacted("e") then return end
  local now = viber.time()
  if now < (st.next_ring or 0) then return end
  st.next_ring = now + 16.2
  viber.play_clip("canto", { duration = 16.0, blend = 0.02 })
  local x, y, z = viber.position()
  viber.burst("sparkle", x, y + 1.65, z, 7)
  local voice = "sino-da-mare-" .. tostring(viber.self_id())
  viber.sound_loop(voice, "world/canto_da_mare.ogg", {x=x, y=y+1.4, z=z, volume=.9})
  viber.after(16, function() viber.sound_stop(voice) end)
end
