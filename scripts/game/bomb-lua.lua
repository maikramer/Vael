-- game/bomb-lua.lua: a bomba do jogo (Lua). Colado ao `<Prototype
-- id="bomb-lua">` — cada instância largada por game/abilities.lua nasce com
-- este comportamento: fuse curto, detonação em ÁREA (`rules.radial_damage`:
-- falloff + knockback) e o juice (burst/ring/shake/kick/hit-stop/número).
--
-- Nota honesta: esta bomba detona NO SÍTIO onde foi largada (sem balística).
-- A balística completa precisa de passar a velocidade inicial ao spawn
-- (follow-up: `opts` do `viber.spawn_prototype`), que a API ainda não tem.

local rules = viber.load("lib/rules.lua")

local FUSE = 1.5
local RADIUS, DAMAGE, KNOCKBACK = 6.0, 90, 9

function on_update(dt)
  local st = viber.state()
  st.t = (st.t or 0) + dt
  -- pisca (aviso) nos últimos 0,4 s
  if st.t > FUSE - 0.4 and st.t < FUSE and not st.blink then
    st.blink = true
    local x, y, z = viber.position()
    viber.burst("sparkle", x, y + 0.3, z, 6)
  end
  if st.t < FUSE or st.boom then
    return
  end
  st.boom = true
  local x, y, z = viber.position()
  rules.radial_damage(x, z, RADIUS, DAMAGE, KNOCKBACK)
  viber.burst("ground-dust", x, y + 0.2, z, 6)
  viber.burst("magic", x, y + 0.6, z, 4)
  viber.ring(x, z, RADIUS * 0.9)
  viber.shake(0.28)
  viber.kick(0, 0.45, 0)
  viber.hit_stop(0.07)
  viber.sound("hit", { x = x, y = y, z = z })
  viber.despawn_self()
end
