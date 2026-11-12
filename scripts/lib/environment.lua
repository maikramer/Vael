-- lib/environment.lua: o clima do mundo para as regras e a apresentação.
--
-- O Rust lê o `bevy_elements` (`src/environment.rs`) e publica a resposta em
-- `viber.vael.env_info()`; este módulo é só o embrulho fino. Sem o ambiente
-- (mundos QA antigos / `VAEL_ELEMENTS=0`) tudo devolve 0/nil como antes.

local M = {}

local function info()
  local f = viber.vael and viber.vael.env_info
  if not f then
    return nil
  end
  local t = f()
  if type(t) ~= "table" then
    return nil
  end
  return t
end

local function number(t, key, fallback)
  local n = t and t[key]
  if type(n) ~= "number" or n ~= n then
    return fallback
  end
  return n
end

--- Chuva 0..1 JÁ com o `rain-scale` do bioma (a que o jogo consome).
function M.rain_effective()
  return number(info(), "rain_effective", 0)
end

--- Chuva crua do crate, antes da escala do bioma.
function M.rain()
  return number(info(), "rain", 0)
end

--- `true` com chuva "a sério" (o mesmo limiar do `clock.lua`).
function M.raining()
  return M.rain_effective() >= 0.3
end

--- Visibilidade meteorológica em metros.
function M.visibility()
  return number(info(), "visibility", 0)
end

--- Vento: `{ x, z, speed }` (direção unitária XZ + m/s) ou `nil`.
function M.wind()
  local t = info()
  if not t then
    return nil
  end
  return {
    x = number(t, "wind_dir_x", 1),
    z = number(t, "wind_dir_z", 0),
    speed = number(t, "wind_speed", 0),
    gustiness = number(t, "gustiness", 0),
  }
end

--- Molhado do solo 0..1 (modelo de superfície do crate).
function M.wetness()
  return number(info(), "wetness", 0)
end

--- Neve: `{ cover = 0..1, depth = m }` ou `nil`.
function M.snow()
  local t = info()
  if not t then
    return nil
  end
  return {
    cover = number(t, "snow_cover", 0),
    depth = number(t, "snow_depth", 0),
  }
end

--- Transmitância das nuvens no herói (1 = sol, 0 = sombra densa); `nil`
--- antes da primeira leitura do crate.
function M.cloud_shadow()
  local t = info()
  local n = t and t.player_cloud_shadow
  if type(n) ~= "number" or n ~= n then
    return nil
  end
  return n
end

--- Transmitância média das nuvens no mundo (1 = céu aberto).
function M.cloud_transmittance()
  return number(info(), "cloud_transmittance", 1)
end

return M
