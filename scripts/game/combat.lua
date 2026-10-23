-- game/combat.lua: o MELEE do jogo, em Lua.
--
-- A engine só expõe primitivas — `viber.play_clip`, `hit_stop`, `shake`,
-- `damage_number`, `hit_flash` e `entity_knockback`. Os alvos vêm do
-- registro `viber.game().enemies` (cada criatura escreve kind/hp/pos) e o
-- dano é escrito nesse registro; a engine não tem Health/hostis.
-- Aqui vive a regra: passos 0→1→2→0, janela da chain, buffer curto, dano,
-- crítico, costas, combo, execução e o feedback do impacto.
--
-- Corre num controller `tag="always-active"` (`world/gameplay.xml`), por
-- isso `viber.play_clip`/`entity_*` levam o id do HERÓI — o controller não
-- tem rig.

local rules = viber.load("lib/rules.lua")

-- Janela vertical do golpe. Igual à do `lib/fsm.lua` (REACH_VERTICAL) e à
-- lista de alvos abaixo: com 2,5 m o melee acertava através de um piso/pontes
-- de 2,4 m e o herói batia em quem não o podia atingir de volta.
local VERTICAL = 2.2
local CYCLE_RANGE = 14
local MELEE_RANGE = 3.0
local FINISHER_RANGE = 3.5
local ATTACK_ARC = 0.0
local FINISHER_ARC = 0.342 -- cos(70°)
local BASE_DAMAGE = 25
local ATTACK_SPEED = 1.4
local CHAIN_MULTS = { 1.0, 1.15, 1.7 }
local CHAIN_CLIPS = { "sworda", "swordb", "swordc" }
local FALLBACK_DURATION = { sworda = 0.417, swordb = 0.5, swordc = 2.0, attack = 1.5 }
local IMPACT_FRACTION = { sworda = 0.30, swordb = 0.38, swordc = 0.42 }
local CHAIN_LINK = 0.45
local CHAIN_FLOOR = 1.1
local BUFFER_SECS = 0.3
local CRIT_CHANCE = 0.15
local CRIT_MULT = 2.0
local COMBO_WINDOW = 2.5
local COMBO_PER_HIT = 0.02
local COMBO_CAP = 0.3
local EXECUTE_FRAC = 0.15
local BOSSES = {
  witch = true, ["sand-worm"] = true, ["bog-warden"] = true, boss = true,
}

-- Nome legível na barra de alvo (o id do script é dado interno).
local TARGET_NAME = {
  wolf = "Lobo", bandit = "Bandido", ["bandit-archer"] = "Arqueiro",
  goblin = "Goblin", scorpion = "Escorpião", slime = "Gosma",
  bogling = "Bogling", shade = "Sombra", witch = "Bruxa do Bosque",
  ["sand-worm"] = "Verme das Areias", ["bog-warden"] = "Guardião do Lodo",
  boss = "Ogro dos Picos",
}

local function stats()
  return rules.player_stats()
end

local function clip_duration(name)
  if viber.clip_duration then
    local d = viber.clip_duration(name)
    if d and d > 0 then
      return d
    end
  end
  return FALLBACK_DURATION[name] or 0
end

local function chain_mult(step)
  return CHAIN_MULTS[math.min(step + 1, #CHAIN_MULTS)] or 1.0
end

local function combo_mult(hits)
  return 1.0 + math.min(hits * COMBO_PER_HIT, COMBO_CAP)
end

-- O HUD lê `combo`/`combo.text`, mas ninguém os escrevia desde que o combo
-- passou para Lua: a coluna e a animação `shake` nunca apareciam.
local function publish_combo(st)
  local n = st.combo_hits or 0
  local text = n > 0 and (n .. " HITS") or ""
  if st.combo_text == text then
    return
  end
  st.combo_text = text
  viber.ui.set("combo", n > 0)
  viber.ui.set("combo.text", text)
end

local function show_target(kind, hp, max_hp)
  viber.ui.set("target.name", kind or "")
  if max_hp and max_hp > 0 then
    viber.ui.set("target", math.max(0, hp or 0) / max_hp)
    viber.ui.set("target.alive", (hp or 0) > 0)
  else
    -- Sem max válido a barra ficava com a fração do alvo ANTERIOR (cheia)
    -- enquanto o nome já era outro. Zera-a com o resto.
    viber.ui.set("target", 0)
    viber.ui.set("target.alive", false)
  end
end

local function start_swing(st)
  -- Dentro da janela avança 0→1→2→0; fora, recomeça no primeiro corte.
  local step = 0
  if (st.chain_timer or 0) > 0 then
    step = ((st.chain_step or 0) + 1) % #CHAIN_CLIPS
  end
  st.chain_step = step
  local clip = CHAIN_CLIPS[step + 1]
  local duration = clip_duration(clip)
  if not duration or duration <= 0 then
    clip = "attack"
    duration = clip_duration(clip)
  end
  if not duration or duration <= 0 then
    duration = 0.5
  end
  local occupy = duration / ATTACK_SPEED
  local fraction = IMPACT_FRACTION[clip] or 0.35
  local delay = math.min(0.45, math.max(0.12, duration * fraction / ATTACK_SPEED))
  st.attack_t = occupy
  st.chain_timer = math.max(occupy + CHAIN_LINK, CHAIN_FLOOR)
  -- A mira do golpe é a do PRESS (o corpo pode girar durante o windup, e um
  -- piparote no rato/A-D não pode fazer o corte aterrar atrás do herói).
  local fx, fz = viber.player_forward()
  st.swing = { t = delay, step = step, fx = fx, fz = fz }
  local pid = viber.player_id()
  if pid then
    viber.play_clip(clip, { speed = ATTACK_SPEED, id = pid })
  end
  -- O controller vive na origem: sem posição o whoosh tocava a 40+ m.
  local has, px, py, pz = viber.player_position()
  if has then
    viber.sound("whoosh", { x = px, y = py, z = pz })
  else
    viber.sound("whoosh")
  end
end

local function resolve_swing(st, sw)
  local has, px, py, pz = viber.player_position()
  if not has then
    return
  end
  -- A mira vem do PRESS (ver `start_swing`). Um `sw` de um estado antigo
  -- (hot-reload a meio do golpe) não traz `fx/fz`: cai no forward atual.
  local fx, fz = sw.fx, sw.fz
  if not fx or not fz then
    fx, fz = viber.player_forward()
  end
  local step = sw.step or 0
  local finisher = step >= 2
  local range = finisher and FINISHER_RANGE or MELEE_RANGE
  local arc = finisher and FINISHER_ARC or ATTACK_ARC
  local bonus = stats()
  local hits, crit_any, kill_any = 0, false, false
  local alert_x, alert_z
  for _, h in ipairs(rules.hostiles(range + 1.0) or {}) do
    local hp = h.hp or 0
    if hp > 0 then
      local dx, dz = (h.x or px) - px, (h.z or pz) - pz
      local dist = math.sqrt(dx * dx + dz * dz)
      if dist <= range and math.abs((h.y or py) - py) <= VERTICAL then
        local ux, uz
        if dist > 1e-3 then
          ux, uz = dx / dist, dz / dist
        else
          -- Criatura empilhada com o herói (dist≈0): não havia direção e o
          -- golpe era descartado — um inimigo em cima ficava invulnerável.
          -- Usa o forward do herói como direção do golpe.
          ux, uz = fx, fz
        end
        if (fx * ux + fz * uz) >= arc then
          local dmg = (BASE_DAMAGE + (bonus.bonus_damage or 0))
            * chain_mult(step) * combo_mult(st.combo_hits or 0)
          local backstab = false
          if h.forward_x and h.forward_z then
            backstab = (h.forward_x * -ux + h.forward_z * -uz) < -0.1
          end
          if backstab then
            dmg = dmg * 2.0
          end
          local crit = math.random() < (CRIT_CHANCE + (bonus.crit_bonus or 0))
          if crit then
            dmg = dmg * CRIT_MULT
          end
          -- Execução: alvo abaixo de 15 % cai na hora. O HP do snapshot é
          -- o do início do frame; o dano garante a morte mesmo assim.
          local execute = (h.max or 0) > 0 and hp < (h.max or 0) * EXECUTE_FRAC
          if execute then
            dmg = math.max(dmg, hp + 1)
          end
          local killed = rules.damage_enemy(h.id, dmg)
          if viber.hit_flash then
            viber.hit_flash(h.id)
          end
          viber.sound("enemy_hurt", { x = h.x, y = h.y, z = h.z })
          if viber.entity_knockback and not BOSSES[h.kind or ""] then
            local meters = crit and 0.65 or (finisher and 0.55 or 0.45)
            viber.entity_knockback(h.id, ux, uz, meters * 6.0)
          end
          viber.damage_number("-" .. math.floor(dmg), {
            color = (crit or execute) and "#ff8c47" or "#f5efe0",
            x = h.x, y = (h.y or py) + 1.8, z = h.z,
          })
          viber.burst("sparks", h.x or px, (h.y or py) + 1.1, h.z or pz,
            crit and 8 or 5)
          -- Marca de impacto no chão (decal genérico): lê-se o golpe depois
          -- de as partículas apagarem. O chão vem do terreno, não do centro
          -- do inimigo (que pode estar acima do solo).
          local gx, gz = h.x or px, h.z or pz
          local gy = viber.ground_below(gx, (h.y or py) + 2.0, gz) or (h.y or py)
          if viber.decal then
            viber.decal("/assets/particles/dirt_01.ktx2", gx, gy + 0.02, gz, {
              nx = 0, ny = 1, nz = 0,
              size = finisher and 0.85 or (crit and 0.65 or 0.5),
              life = 2.4,
              color = "#3a2f28",
            })
          end
          hits = hits + 1
          if alert_x == nil then
            alert_x, alert_z = h.x or px, h.z or pz
          end
          crit_any = crit_any or crit
          kill_any = kill_any or killed
        end
      end
    end
  end
  if hits > 0 then
    -- Aggro-chain: a engine deixou de emitir `AttackAlert` (o melee nativo
    -- foi apagado), por isso o hook `on_player_attack` dos scripts das
    -- criaturas nunca corria — a matilha não acordava com o golpe no aliado.
    -- O alerta vive numa TABELA do `viber.game()` (não viaja no save) e o
    -- `lib/fsm.lua` acorda os aliados a 15 m do primeiro alvo batido.
    if alert_x then
      local g = viber.game()
      local alert = g.attack_alert
      if type(alert) ~= "table" then
        alert = {}
        g.attack_alert = alert
      end
      alert.x, alert.z = alert_x, alert_z
      alert.t = viber.time()
      alert.seq = (alert.seq or 0) + 1
    end
    st.combo_hits = (st.combo_hits or 0) + hits
    st.combo_window = COMBO_WINDOW
    publish_combo(st)
    viber.sound("hit", { x = px, y = py, z = pz })
    viber.hit_stop(kill_any and 0.09 or (crit_any and 0.07 or 0.05))
    viber.shake(kill_any and 0.36 or (crit_any and 0.28 or (finisher and 0.26 or 0.18)))
    viber.punch(crit_any and 0.09 or 0.06, 0.02)
    if finisher then
      viber.ring(px, pz, FINISHER_RANGE)
      viber.burst("ground-dust", px, py + 0.15, pz, 8)
    end
  end
end

function on_update(dt)
  local st = viber.state()
  -- Novo Jogo / save de outra partida: nada do combate anterior (golpe no
  -- ar, chain, combo, alvo) pode atravessar a fronteira.
  -- Um LOAD na MESMA cena troca o `viber.game()` sem mudar `session_id` nem
  -- a cena, e o relógio da engine NÃO volta atrás (é o uptime): o sinal é o
  -- tempo de jogo a recuar. Sem isto, um golpe pendurado no jogo abandonado
  -- aterrava no sítio onde o herói carregou, com o combo/alvo antigos.
  local game = viber.game()
  local sid = tonumber(game.session_id) or 0
  local playtime = tonumber(game.playtime_secs) or 0
  local loaded = st.playtime ~= nil and playtime < st.playtime - 1e-3
  if st.session ~= sid or loaded then
    st.session = sid
    st.swing = nil
    st.attack_t = 0
    st.chain_timer = 0
    st.chain_step = 0
    st.buffer_queued = false
    st.buffer_t = 0
    st.combo_hits = 0
    st.combo_window = 0
    st.target = nil
    st.last_hp = nil
    publish_combo(st)
    viber.ui.set("target", 0)
    viber.ui.set("target.alive", false)
    viber.ui.set("target.name", "")
  end
  st.playtime = playtime

  -- Troca de cena (portal/interior) NÃO muda a sessão: sem isto um golpe
  -- pendente aterrava já na cena nova, no sítio onde o herói entrou (o
  -- `resolve_swing` lê a posição ATUAL), e a barra de alvo/combo do mundo
  -- anterior ficava no HUD.
  local scene = viber.scene_name and viber.scene_name() or nil
  if st.scene ~= scene then
    st.scene = scene
    st.swing = nil
    st.attack_t = 0
    st.chain_timer = 0
    st.chain_step = 0
    st.buffer_queued = false
    st.buffer_t = 0
    st.combo_hits = 0
    st.combo_window = 0
    st.target = nil
    st.last_hp = nil
    publish_combo(st)
    viber.ui.set("target", 0)
    viber.ui.set("target.alive", false)
    viber.ui.set("target.name", "")
  end

  -- Timers do melee (correm mesmo com o menu aberto, como o combo nativo).
  st.attack_t = math.max(0, (st.attack_t or 0) - dt)
  st.chain_timer = math.max(0, (st.chain_timer or 0) - dt)
  if st.chain_timer <= 0 then
    st.chain_step = 0
  end
  st.combo_window = math.max(0, (st.combo_window or 0) - dt)
  if st.combo_window <= 0 and (st.combo_hits or 0) > 0 then
    st.combo_hits = 0
    publish_combo(st)
  end
  st.buffer_t = math.max(0, (st.buffer_t or 0) - dt)
  if st.buffer_t <= 0 then
    st.buffer_queued = false
  end

  if viber.menu_open() then
    -- Um golpe pendente não pode aterrar depois de fechar o menu, muito
    -- depois da animação/posição que o originou.
    st.swing = nil
    st.buffer_queued = false
    st.buffer_t = 0
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    -- O golpe que estava no ar não pode aterrar depois do respawn, no sítio
    -- onde o herói caiu. Fecha também a janela de chain e o buffer.
    st.swing = nil
    st.attack_t = 0
    st.chain_timer = 0
    st.chain_step = 0
    st.buffer_queued = false
    st.buffer_t = 0
    st.combo_hits = 0
    st.combo_window = 0
    publish_combo(st)
    viber.ui.set("target.alive", false)
    viber.ui.set("target.name", "")
    return
  end
  -- Dano no herói zera o combo (o nativo fazia-o em `notifyPlayerDamaged`).
  if alive and st.last_hp and hp < st.last_hp then
    st.combo_hits = 0
    st.combo_window = 0
    publish_combo(st)
  end
  st.last_hp = hp

  -- ── barra de alvo (ação nativa `lock`; o V é do equipamento) ──
  local has, _, py, _ = viber.player_position()
  if has then
    local list = rules.hostiles(CYCLE_RANGE) or {}
    local ids = {}
    for _, h in ipairs(list) do
      if math.abs((h.y or py) - py) <= 2.2 then
        ids[#ids + 1] = h
      end
    end
    local lock_pressed = viber.action ~= nil and viber.action("lock")
    if lock_pressed then
      if #ids == 0 then
        -- Sem alvos, o cartão tem de sair JÁ: `st.target` nil impede o ramo
        -- de limpeza abaixo (que só corre com um alvo pendente) e o nome/vida
        -- do alvo que acabou de sair ficava preso no HUD.
        st.target = nil
        viber.ui.set("target", 0)
        viber.ui.set("target.alive", false)
        viber.ui.set("target.name", "")
      else
        local at = 1
        for i, h in ipairs(ids) do
          if h.id == st.target then
            at = i % #ids + 1
            break
          end
        end
        st.target = ids[at].id
      end
    end
    local shown
    for _, h in ipairs(ids) do
      if h.id == st.target then
        shown = h
        break
      end
    end
    if not shown then
      shown = ids[1]
      st.target = shown and shown.id or nil
    end
    if shown then
      show_target(TARGET_NAME[shown.kind or ""] or shown.kind, shown.hp, shown.max)
    elseif st.target ~= nil then
      st.target = nil
      viber.ui.set("target", 0)
      viber.ui.set("target.alive", false)
      viber.ui.set("target.name", "")
    end
  end

  -- ── melee ──
  -- Com a TOCHA na mão não há arma para atacar: o `equip.kind` é publicado
  -- por game/equipment.lua (o V troca Espada↔Tocha). Sem este portão o
  -- herói "atacava com o lampião" (swing + dano com o GLB da lanterna).
  -- O [J] é o mesmo verbo da colheita: perto de uma árvore/pedra com o
  -- prompt "J" no ecrã, o golpe de espada disparava POR CIMA do machado
  -- (dois `play_clip` no herói e dano a inimigos vizinhos). O rato continua
  -- a atacar; só o J cede o lugar à interação.
  local j_key = viber.input.pressed("j")
  local prompt_j = viber.ui.get("prompt.key") == "J"
    and viber.ui.get("prompt.label") ~= ""
  -- `viber.action("attack")` é o mapa de ações (rebindável no save) e cobre
  -- o botão do gamepad; ler só o J fixo deixava o ataque morto com o comando
  -- trocado e sem nenhum botão no pad. O J do mapa é a MESMA tecla do prompt
  -- de colheita: quando o prompt é dono do J, só rato/pad atacam (o pad não
  -- toca no J, por isso `not j_key` distingue-o).
  local action_attack = viber.action ~= nil and viber.action("attack")
  local pressed = (j_key and not prompt_j)
    or viber.input.pressed("lmb")
    or (action_attack and not j_key)
  local torch = viber.ui.get("equip.kind") == "torch"
  if torch then
    if pressed then
      viber.toast("Tocha na mão — troca com V para atacar.")
    end
    pressed = false
    -- Um golpe em BUFFER (premido com a espada) não pode sair do bolso
    -- depois da troca: o portão da tocha fechava a tecla, não o buffer.
    st.buffer_queued = false
    st.buffer_t = 0
  end
  local busy = (st.attack_t or 0) > 0 or st.swing ~= nil
  if pressed then
    if busy then
      st.buffer_queued = true
      st.buffer_t = BUFFER_SECS
    else
      start_swing(st)
    end
  elseif st.buffer_queued and not busy and not torch then
    st.buffer_queued = false
    start_swing(st)
  end
  if st.swing then
    st.swing.t = st.swing.t - dt
    if st.swing.t <= 0 then
      local sw = st.swing
      st.swing = nil
      resolve_swing(st, sw)
    end
  end
end
