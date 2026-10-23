local rules = viber.load("lib/rules.lua")
-- campfire.lua: sentar/descansar na fogueira da praça (cura + toast).
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    -- A fogueira é uma caixa de 2 m (meia-largura 1 m) e o herói tem 0,35 m
    -- de raio: com o alcance autorado de 2,5 (1,25 efetivo) o [E] só aparecia
    -- de pé EM CIMA da fogueira. 5 m = 2,5 efetivo, na borda da área.
    viber.set_interaction("Descansar", "e", 5.0)
  end
  if viber.menu_open() or not viber.interacted("e") then
    return
  end
  local ok, hp, max_hp = rules.player_hp()
  if ok and hp <= 0 then
    return
  end
  -- O checkpoint vale mesmo com a vida cheia: a marcação/save não pode ficar
  -- dependente da cura. A cura entra ANTES do autosave — o save do checkpoint
  -- levava o HP com que o jogador chegou e o descanso perdia-se ao carregar.
  local healed = math.min(25, math.max(0, (max_hp or 0) - (hp or 0)))
  if healed >= 1 then
    rules.heal_player(healed)
    -- A cura do poço confirma com `heal`; o descanso na fogueira era a única
    -- cura do mundo sem confirmação audível.
    viber.sound("heal")
  end
  local has, px, py, pz = viber.player_position()
  if has then
    rules.set_spawn(px, py, pz)
    -- Checkpoint = autosave (slot 0). O jogador pode sempre Tentar de novo.
    -- No máximo 1×/2 s: bater E à fogueira não pode escrever o save a 60 Hz.
    local now = viber.time()
    if st.save_t == nil or now < st.save_t or now - st.save_t >= 2.0 then
      st.save_t = now
      viber.save_slot(0)
    end
  end
  if healed < 1 then
    viber.toast("O calor não tem mais o que restaurar. Ponto de descanso marcado.")
    return
  end
  viber.toast("O calor da fogueira restaura " .. math.floor(healed) .. " de vida.")
end
