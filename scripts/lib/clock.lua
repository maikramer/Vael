-- lib/clock.lua: a hora do mundo para as regras do jogo.
--
-- O Rust lê o `<DayCycle>`/`<Weather>` (recursos Bevy) e publica a resposta
-- em `viber.vael.clock_*`; este módulo é só o embrulho fino. Sem relógio
-- (mundos QA) `hour()` é nil e é sempre dia.

local M = {}

M.NIGHT_FROM, M.NIGHT_TO = 20, 5 -- 20:00 → 05:00
M.RAIN_HEAVY = 0.3 -- chuva 0..1: a partir daqui o fogo apaga-se depressa

function M.hour()
  local f = viber.vael and viber.vael.clock_hour
  if not f then
    return nil
  end
  local n = f()
  if type(n) ~= "number" then
    return nil
  end
  return n
end

function M.is_night()
  local f = viber.vael and viber.vael.clock_is_night
  return f and f() == true or false
end

function M.rain()
  local f = viber.vael and viber.vael.clock_rain
  local n = f and f() or 0
  if type(n) ~= "number" or n ~= n then
    return 0
  end
  return n
end

function M.raining()
  local f = viber.vael and viber.vael.clock_raining
  return f and f() == true or false
end

return M
