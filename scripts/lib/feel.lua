-- feel.lua: peso do impacto e da fala, sem um clip novo.
--
-- Malha gerada não traz o frame do golpe. O que se lê é o que o motor
-- acrescenta por cima: parar o tempo só quando o estado muda (guarda,
-- atordoamento, morte), abanar a câmara só nesse peso, e baixar a música
-- para o som — e para a fala — caberem no mesmo instante. Na chain o
-- hit-stop fica no primeiro golpe e no remate: no meio a chain ficava
-- lenta e o shake em todos os hits enjoava. O `A11y` da engine continua
-- a poder calar stop e shake.

local M = {}

-- ganho linear do bus, segundos. Mais baixo = a música cede mais.
local DUCK = {
  graze = { 0.9, 0.08 },
  light = { 0.86, 0.1 },
  heavy = { 0.7, 0.18 },
  kill = { 0.55, 0.28 },
  hurt = { 0.64, 0.22 },
  death = { 0.48, 0.4 },
  voice = { 0.68, 2.6 },
}

function M.duck(bus, gain, secs)
  local audio = vael.audio
  if type(audio) == "table" and type(audio.duck) == "function" then
    audio.duck(bus, gain, secs)
  end
end

local function duck_named(name)
  local row = DUCK[name]
  if row then
    M.duck("music", row[1], row[2])
  end
end

-- `graze` raspão/bloqueio, `open` primeiro golpe da chain, `light` o
-- do meio, `heavy` crítico/remate/choque, `kill` morte.
function M.blow(tier)
  if tier == "open" then
    if vael.hit_stop then vael.hit_stop(0.03) end
    if vael.shake then vael.shake(0.08) end
    duck_named("light")
    return
  end
  if tier == "kill" then
    if vael.hit_stop then vael.hit_stop(0.09) end
    if vael.shake then vael.shake(0.22) end
    if vael.punch then vael.punch(0.09, 0.02) end
    duck_named("kill")
    return
  end
  if tier == "heavy" then
    if vael.hit_stop then vael.hit_stop(0.06) end
    if vael.shake then vael.shake(0.14) end
    if vael.punch then vael.punch(0.06, 0.02) end
    duck_named("heavy")
    return
  end
  if tier == "graze" then
    if vael.hit_stop then vael.hit_stop(0.02) end
    if vael.shake then vael.shake(0.04) end
    duck_named("graze")
    return
  end
  if vael.shake then vael.shake(0.05) end
  duck_named("light")
end

-- Dano no herói. `died` pesa como morte; `blocked` é raspão.
function M.hurt(died, blocked)
  if died then
    if vael.hit_stop then vael.hit_stop(0.09) end
    if vael.shake then vael.shake(0.2) end
    duck_named("death")
    return
  end
  if blocked then
    if vael.hit_stop then vael.hit_stop(0.02) end
    if vael.shake then vael.shake(0.06) end
    duck_named("graze")
    return
  end
  if vael.hit_stop then vael.hit_stop(0.05) end
  if vael.shake then vael.shake(0.12) end
  duck_named("hurt")
end

function M.parry()
  if vael.hit_stop then vael.hit_stop(0.055) end
  duck_named("heavy")
end

-- A fala passa por `vael.say`. A música cede no início da linha e volta
-- antes do balão morrer, para uma conversa longa não deixar o bus no chão.
function M.install()
  if vael.__feel_voice then
    return
  end
  local say = vael.say
  if type(say) ~= "function" then
    return
  end
  vael.__feel_voice = true
  vael.say = function(text, secs, ...)
    if type(text) == "string" and text ~= "" then
      local window = tonumber(secs)
      if not window or window ~= window or window <= 0 then
        window = 4
      end
      local hold = math.min(DUCK.voice[2], math.max(0.7, window * 0.5))
      M.duck("music", DUCK.voice[1], hold)
    end
    return say(text, secs, ...)
  end
end

return M
