-- Os três sinos acompanham os passos. O rio só canta para quem o escuta.
-- Distâncias locais ao centro do tabuleiro da Passadeira da Bruxa.
local stations = {
  { along = -9, side = .91, pitch = 1.0, height = 4.1 },
  { along = 0, side = -1.25, pitch = 2^(5/12), height = 5.6 },
  { along = 9, side = 1.59, pitch = 2^(2/12), height = 3.9 },
}

local function play_voice(st, sample, pitch, duration, x, y, z)
  -- A voz posicional aceita um ficheiro do mundo. Cada toque tem a sua cauda,
  -- mesmo quando o viajante volta ao mesmo sino antes de o bronze se calar.
  st.serial = (st.serial or 0) + 1
  local id = "carrilhao-" .. tostring(viber.self_id()) .. "-" .. tostring(st.serial)
  viber.sound_loop(id, "world/" .. sample .. ".ogg", {
    pitch = pitch, volume = .95, x = x, y = y, z = z,
  })
  viber.after(duration, function() viber.sound_stop(id) end)
end

local function river_answer(st, x, y, z)
  play_voice(st, "vessa_quarta_voz", 1, 6.2, x - 3.5, y + 1.0, z)
  st.answered = true
  st.wave = 0
  st.ripple = 0
  local game = viber.game()
  if not game.vessa_quarta_voz then
    game.vessa_quarta_voz = true
    viber.toast("Três sinos na ponte. A quarta voz veio do rio.")
  end
end

function on_update(dt)
  local st = viber.state()
  local has, px, py, pz = viber.player_position()
  if not has or viber.menu_open() then return end
  local x, y, z = viber.position()
  local along, side = pz - z, px - x
  local deck = .18*(16-along)/32 + 2.75*math.cos(along*math.pi/32)
  if math.abs(along) > 16 or math.abs(side) > 1.68 or py < y + deck - .3
      or py > y + deck + 2.7 or (viber.game().hp or 1) <= 0 then
    st.station, st.rest, st.anchor_x, st.wave = nil, 0, nil, nil
    st.answered = false
    return
  end

  if st.station and math.abs(along - stations[st.station].along) > 2.1 then
    st.station = nil
  end
  if not st.station then
    for i, bell in ipairs(stations) do
      if math.abs(along - bell.along) <= 1.35 then
        st.station = i
        local bx, by, bz = x + bell.side, y + bell.height, z + bell.along
        play_voice(st, "vessa_bronze", bell.pitch, 3.1/bell.pitch, bx, by, bz)
        viber.burst("sparkle", bx, by, bz, 9)
        break
      end
    end
  end

  if math.abs(along) <= 1.6 and not st.answered then
    if not st.anchor_x or (px-st.anchor_x)^2 + (pz-st.anchor_z)^2 > .24^2 then
      st.anchor_x, st.anchor_z, st.rest = px, pz, 0
    else
      st.rest = st.rest + math.min(dt, .2)
      if st.rest >= 2.4 then river_answer(st, x, y, z) end
    end
  else
    st.anchor_x = nil
  end

  if st.wave then
    st.wave = st.wave + dt
    while st.ripple < 8 and st.wave >= st.ripple*.35 do
      local i = st.ripple
      viber.burst("sparkle", x - 1.9 - i*.77, y + 2.5 - i*.34,
                  z + math.sin(i*.67)*1.45, 10)
      st.ripple = i + 1
    end
    if st.ripple == 8 then st.wave = nil end
  end
end
