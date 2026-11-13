--@export chord: enum(brasa,geada,tempestade,raiz,pacto) = "brasa"
--@export prompt: string = "Escutar a Pedra de Eco"
--@export range: number(0.5..10) = 3.0
--@export stone_key: string = ""

-- scripts/poi/echo-stone.lua: Pedra de Ressonância do Coro de Vael.
--
-- Ensina versos de exemplo de um acorde, concede +15 XP na primeira
-- descoberta e ativa `voice_stone_boost_until` por 60s (reduzindo pela
-- metade a recarga do canto [T]).

local rules = viber.load("lib/rules.lua")

local LESSONS = {
  brasa = {
    title = "Acorde da Brasa",
    verse = "Versos: “chama do sol”, “fogo e cinza”, “ardor de verão”.",
    color = "#ff7733",
    burst = "fire",
  },
  geada = {
    title = "Acorde da Geada",
    verse = "Versos: “escudo de gelo”, “pedra e inverno”, “neve protetora”.",
    color = "#66ccff",
    burst = "snow",
  },
  tempestade = {
    title = "Acorde da Tempestade",
    verse = "Versos: “vento e trovão”, “raio veloz”, “corte do céu”.",
    color = "#ffe066",
    burst = "sparks",
  },
  raiz = {
    title = "Acorde da Raiz",
    verse = "Versos: “água da raiz”, “cura da floresta”, “luzir o caminho”.",
    color = "#66dd88",
    burst = "leaves",
  },
  pacto = {
    title = "Pacto de Caça",
    verse = "Versos: “caçar o lobo”, “jurar à sombra”, “abater o goblin”.",
    color = "#cc66ff",
    burst = "magic",
  },
}

local function stone_flag_key(chord)
  local custom = tostring(viber.prop("stone_key") or "")
  if custom ~= "" then
    return "echo_stone." .. custom
  end
  local ename = viber.entity_name and viber.entity_name()
  if ename and ename ~= "" then
    return "echo_stone." .. ename
  end
  return "echo_stone." .. tostring(chord or "brasa")
end

function on_update(_dt)
  local st = viber.state()
  if not st.ready then
    st.ready = true
    local prompt = viber.prop("prompt")
    local range = tonumber(viber.prop("range")) or 3.0
    viber.set_interaction(prompt ~= "" and prompt or "Escutar a Pedra de Eco", "e", range)
  end

  if viber.menu_open and viber.menu_open() then
    return
  end
  local alive, hp = rules.player_hp()
  if alive and (hp or 0) <= 0 then
    return
  end

  if viber.interacted and viber.interacted("e") then
    local chord = tostring(viber.prop("chord") or "brasa")
    local info = LESSONS[chord] or LESSONS.brasa

    local has, x, y, z = false, 0, 0, 0
    if viber.entity_position then
      has, x, y, z = viber.entity_position()
    end
    if not has and viber.player_position then
      has, x, y, z = viber.player_position()
    end
    local pos = has and { x = x, y = y, z = z } or nil

    if pos and viber.sound then
      viber.sound("notification", pos)
    end
    if has and viber.burst then
      viber.burst(info.burst, x, y + 1.1, z, 20)
    end
    if has and viber.ring then
      viber.ring(x, z, 3.2, info.color)
    end

    local g = viber.game()
    local now = (viber.time and viber.time()) or 0
    g.voice_stone_boost_until = now + 60.0

    local flag = stone_flag_key(chord)
    local first = not g[flag]
    if first then
      g[flag] = true
      rules.add_xp(15, "Pedra de Eco")
      viber.toast("Pedra de Eco (" .. info.title .. "): " .. info.verse .. " Ressonância 60s (recarga [T] reduzida a metade). Prima [T]! (+15 XP)")
    else
      viber.toast("Pedra de Eco (" .. info.title .. "): " .. info.verse .. " Ressonância 60s renovada. Prima [T]!")
    end
  end
end
