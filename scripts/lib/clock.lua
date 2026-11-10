-- lib/clock.lua: a hora do mundo para as regras do jogo.
--
-- A engine publica o relógio do `<DayCycle>` no bind `clock` ("HH:MM"); não
-- há API de hora para scripts. Este módulo lê o bind e responde à única
-- pergunta que o jogo faz: é de noite? Sem relógio (mundos QA) é sempre dia.

local M = {}

M.NIGHT_FROM, M.NIGHT_TO = 20, 5 -- 20:00 → 05:00

local cache_t, cache_h = -1, nil

function M.hour()
  local now = viber.time()
  if now - cache_t < 0.5 and now >= cache_t then
    return cache_h
  end
  cache_t = now
  local text = viber.ui and viber.ui.get and viber.ui.get("clock") or ""
  local h, m = tostring(text):match("^(%d+):(%d+)")
  h, m = tonumber(h), tonumber(m)
  cache_h = (h and m) and (h + m / 60) or nil
  return cache_h
end

function M.is_night()
  local h = M.hour()
  if not h then
    return false
  end
  return h >= M.NIGHT_FROM or h < M.NIGHT_TO
end

-- Chuva 0..1 (o bind `weather.rain` do HUD). Com chuva a sério o fogo
-- apaga-se depressa e as criaturas veem pior.
M.RAIN_HEAVY = 0.3

function M.rain()
  local n = viber.ui and viber.ui.number and viber.ui.number("weather.rain") or 0
  if type(n) ~= "number" or n ~= n then
    return 0
  end
  return n
end

function M.raining()
  return M.rain() >= M.RAIN_HEAVY
end

return M
