-- game/bomb-lua.lua: a bomba do jogo (Lua). Colado ao `<Prototype
-- id="bomb-lua">` — cada instância largada por game/abilities.lua nasce com
-- este comportamento: fuse curto, detonação em ÁREA (`rules.radial_damage`:
-- falloff + knockback) e o juice (burst/ring/shake/kick/hit-stop/número).
--
-- Nota honesta: esta bomba detona NO SÍTIO onde foi largada (sem balística).
-- A balística completa precisa de passar a velocidade inicial ao spawn
-- (follow-up: `opts` do `vael.spawn_prototype`), que a API ainda não tem.

local rules = vael.load("lib/rules.lua")
local feel
do
  local ok, mod = pcall(vael.load, "lib/feel.lua")
  if ok and type(mod) == "table" and type(mod.duck) == "function" then
    feel = mod
  end
end

local FUSE = 1.5
local RADIUS, DAMAGE, KNOCKBACK = 6.0, 90, 9

function on_update(dt)
  local st = vael.state()
  -- Load na MESMA cena: a bomba é um spawn de RUNTIME (não está no save) mas a
  -- entidade sobrevive à troca do `vael.game()`. Sem isto o rastilho da linha
  -- temporal abandonada continuava e explodia no mundo carregado. O relógio
  -- da engine é o uptime e não recua; `playtime_secs` (regras) recua no load.
  local playtime = tonumber(vael.game().playtime_secs) or 0
  if st.playtime ~= nil and playtime < st.playtime - 1e-3 then
    vael.despawn_self()
    return
  end
  st.playtime = playtime
  -- Menu/diário aberto: o rastilho congela com o resto do combate. Antes a
  -- bomba largada antes do menu explodia enquanto o jogador lia o diário.
  if vael.menu_open() then
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
      local x, y, z = vael.position()
      vael.burst("sparkle", x, y + 0.3, z, 4)
    end
  end
  if st.t < FUSE or st.boom then
    return
  end
  st.boom = true
  local x, y, z = vael.position()
  -- A explosão pega fogo a quem apanha (3 s de queimadura).
  rules.radial_damage(x, z, RADIUS, DAMAGE, KNOCKBACK, y, 3)
  -- Aggro-chain: a explosão acorda os aliados a 15 m (o nativo emitia um
  -- `AttackAlert` no centro; a engine já não tem produtor e o `lib/fsm.lua`
  -- lê esta tabela do jogo).
  local g = vael.game()
  local alert = g.attack_alert
  if type(alert) ~= "table" then
    alert = {}
    g.attack_alert = alert
    alert = g.attack_alert or alert -- tabela partilhada: escreve no proxy
  end
  alert.x, alert.z = x, z
  alert.t = vael.time()
  alert.seq = (alert.seq or 0) + 1
  vael.burst("ground-dust", x, y + 0.2, z, 6)
  vael.burst("magic", x, y + 0.6, z, 4)
  vael.ring(x, z, RADIUS)
  vael.shake(0.28)
  vael.kick(0, 0.45, 0)
  vael.hit_stop(0.07)
  vael.sound("hit", { x = x, y = y, z = z })
  if feel then
    feel.duck("music", 0.5, 0.3)
  end
  vael.despawn_self()
end
