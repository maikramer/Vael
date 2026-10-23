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
  -- Load na MESMA cena: a bomba é um spawn de RUNTIME (não está no save) mas a
  -- entidade sobrevive à troca do `viber.game()`. Sem isto o rastilho da linha
  -- temporal abandonada continuava e explodia no mundo carregado. O relógio
  -- da engine é o uptime e não recua; `playtime_secs` (regras) recua no load.
  local playtime = tonumber(viber.game().playtime_secs) or 0
  if st.playtime ~= nil and playtime < st.playtime - 1e-3 then
    viber.despawn_self()
    return
  end
  st.playtime = playtime
  -- Menu/diário aberto: o rastilho congela com o resto do combate. Antes a
  -- bomba largada antes do menu explodia enquanto o jogador lia o diário.
  if viber.menu_open() then
    return
  end
  st.t = (st.t or 0) + dt
  -- Aviso nos últimos 0,5 s: pisca VÁRIAS vezes. Um único burst (e só se um
  -- frame calhasse dentro da janela) perdia-se num hitch — a bomba explodia
  -- sem aviso visível.
  if st.t > FUSE - 0.5 and st.t < FUSE then
    st.blink_t = (st.blink_t or 0) - dt
    if st.blink_t <= 0 then
      st.blink_t = 0.12
      local x, y, z = viber.position()
      viber.burst("sparkle", x, y + 0.3, z, 4)
    end
  end
  if st.t < FUSE or st.boom then
    return
  end
  st.boom = true
  local x, y, z = viber.position()
  rules.radial_damage(x, z, RADIUS, DAMAGE, KNOCKBACK)
  -- Aggro-chain: a explosão acorda os aliados a 15 m (o nativo emitia um
  -- `AttackAlert` no centro; a engine já não tem produtor e o `lib/fsm.lua`
  -- lê esta tabela do jogo).
  local g = viber.game()
  local alert = g.attack_alert
  if type(alert) ~= "table" then
    alert = {}
    g.attack_alert = alert
  end
  alert.x, alert.z = x, z
  alert.t = viber.time()
  alert.seq = (alert.seq or 0) + 1
  viber.burst("ground-dust", x, y + 0.2, z, 6)
  viber.burst("magic", x, y + 0.6, z, 4)
  viber.ring(x, z, RADIUS * 0.9)
  viber.shake(0.28)
  viber.kick(0, 0.45, 0)
  viber.hit_stop(0.07)
  viber.sound("hit", { x = x, y = y, z = z })
  viber.despawn_self()
end
