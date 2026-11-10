-- lib/fsm.lua: máquina wander↔chase partilhada dos inimigos/bosses.
-- Fatorada dos ~11 scripts que tinham o MESMO bloco copiado (steering,
-- anti-stuck, aggro-window, cadência de ataque) — a engine provê os blocos
-- (percepção, movimento com snap no terreno, `next_state`), a composição
-- vive aqui e cada criatura só passa os seus números.
--
-- COMBATE v2 (ataques LEGÍVEIS): o golpe deixou de ser dano instantâneo no
-- fim do cooldown. Agora é uma sequência com leitura e resposta:
--
--   1. WINDUP   — a criatura trava, vira-se, toca o clip `attack` e marca
--                 a zona com um anel fino (um por golpe; o especial longo
--                 repete devagar para a zona não sumir antes do impacto);
--   2. IMPACTO  — o dano só entra se o herói AINDA estiver ao alcance
--                 (`range + reach`, e |Δy| curto). Recuar, dar dash [C] ou
--                 aparar [L] no instante certo resolvem o golpe;
--   3. RECOVER  — `recover` s parada depois do golpe: a janela de punição.
--
-- Atingir a criatura faz FLINCH (clip `hit`) e cancela o windup — o herói
-- que ataca primeiro ganha a troca. Chefes têm POISE (`poise`): só
-- vacilam quando o dano acumulado passa essa fração do HP máximo. Ser
-- atingido também acorda a criatura (aggro 10 s), mesmo fora do raio.
--
-- Uso no script da criatura:
--
--   local fsm = viber.load("lib/fsm.lua").new{
--     wander = 2.2, chase = 4.6,      -- m/s
--     aggro = 11, deaggro = 16,       -- histerese da FSM (m)
--     range = 1.6,                    -- alcance do ataque (m)
--     damage = 9, cooldown = 1.2,     -- golpe
--     windup = 0.4,                   -- opcional: aviso antes do impacto (s)
--     recover = 0.35,                 -- opcional: janela de punição (s)
--     radius = 7,                     -- raio do wander (m)
--     turn_rate = 9.0,                -- opcional: pivô rápido do rig
--     steering = true,                -- opcional: alvos alinhados com o rumo
--     anti_stuck = true,              -- opcional (default true): repick após 6 s preso
--     on_chase = function() viber.sound("growl") end,  -- opcional: transição
--     on_attack = function() rules.apply_status("venom", 4) end, -- golpe que ACERTA
--     on_first = function() viber.log("ativo") end,    -- 1.ª vez por entidade
--     gate = function() return true end,  -- opcional: false = dorme (boss gating)
--     awake_toast = "…",              -- opcional: toast à 1.ª passagem do gate
--     -- Chefes:
--     boss = "Bruxa do Bosque",       -- nome na barra de chefe do HUD
--     poise = 0.12,                   -- fração do HP que faz vacilar
--     special = { every = 3, windup = 1.0, radius = 4.5, mult = 1.6,
--                 at = "self" | "player", clip = "roar", color = "#ff4a2a",
--                 label = "…", on_hit = function() … end },
--     enrage = { at = 0.4, toast = "…", speed = 1.2, cooldown = 0.7 },
--     -- Atiradores: dispara um `<ProjectileTemplate>` entre `min` e `range`
--     -- metros (com aviso de `windup` s); mais perto do que `min` recua
--     -- (a arma de perto só sai encurralado).
--     ranged = { range = 15, min = 5, cooldown = 2.4, windup = 0.7,
--                projectile = "enemy-arrow" },
--     -- ARQUÉTIPOS (opcionais, combináveis):
--     charge = { min = 5, max = 12, cooldown = 6, windup = 0.9, speed = 10,
--                mult = 1.8, recover = 1.3 },  -- investida em linha: sai do corredor
--     shield = { arc = 0.3, reduce = 0.8 },    -- bloqueia de frente; o remate parte a guarda
--     caster = { range = 13, min = 5, cooldown = 4.5, windup = 1.2, radius = 2.6,
--                mult = 1.2, color = "#b04cff", on_hit = function() … end },
--                                              -- zona no chão onde o herói ESTAVA
--     weak = { burn = 2.0 },          -- multiplicador do dano de um estado
--     nocturnal = true,               -- de noite: vê mais longe e bate mais
--     fades_at_dawn = true,           -- some ao amanhecer (sombras da noite)
--     proto = "mob-wolf",             -- `<Prototype>` para renascer (game/spawns.lua)
--     -- Chefes: FASES por limiar de vida (ordem decrescente de `at`).
--     phases = { { at = 0.66, toast = "…", summon = { proto = "mob-shade", count = 2 },
--                  special = { … }, speed = 1.1, cooldown = 0.9,
--                  burrow = { every = 9, under = 1.6, windup = 1.0, radius = 4.5, mult = 1.5 },
--                  blink = { every = 7, dist = 9 }, charge = { … } }, … },
--     elite_chance = 0.08,            -- opcional (default 0.08; chefes 0)
--     intro = "guardiao-bruxa",       -- opcional: `<Cinematic>` da 1.ª perseguição
--                                     -- (uma vez por save; âncora `self` = esta criatura)
--   }
--
-- ELITES: cada criatura não-chefe tem `elite_chance` de nascer elite —
-- maior (×1,3), 2,5× a vida e golpes ×1,4; regista-se em
-- `viber.game().elites[id]` e o `game/loot.lua` paga a triplicar.
--   function on_player_attack(px, pz) fsm.alert() end
--   function on_update(dt) fsm.update(dt) end
--
-- O estado é POR ENTIDADE (`viber.state()`); o objeto devolvido por `new`
-- partilha a CONFIG entre instâncias do mesmo script (top-level corre 1×).

local M = {}

local rules = viber.load("lib/rules.lua")
local clock = viber.load("lib/clock.lua")

-- Marca da sessão do MUNDO: `viber.game().enemies` não é persistido, mas o
-- `viber.game()` sobrevive ao hot-reload. A marca vive lá como TABELA (o save
-- só guarda primitivos, por isso morre no `load`) — uma constante calculada no
-- top-level deste módulo mudava a cada hot-reload de `lib/fsm.lua` e recriava
-- TODOS os registos com vida cheia a meio de uma luta.
local function session_tag()
  local game = viber.game()
  local tag = game.fsm_session
  if type(tag) ~= "table" then
    tag = {}
    game.fsm_session = tag
  end
  return tag
end

-- Folga de alcance no impacto (m): o golpe apanha quem estava colado e deu
-- meio passo; quem recua a sério ou dá dash sai.
local REACH_SLACK = 0.7
-- Diferença de altura máxima para o golpe (m) — sem mordidas através de
-- pisos, pontes ou encostas a pique.
local REACH_VERTICAL = 2.2
-- Cadência do anel de aviso do especial (s). Golpes curtos disparam um
-- anel no primeiro frame e não repetem (o windup acaba antes disto).
local TELEGRAPH_PULSE = 0.9
-- Cor do aviso dos golpes normais (âmbar) — o especial dos chefes é vermelho.
local TELEGRAPH_COLOR = "#ffb347"
-- Stagger (s) de um flinch normal / com windup cancelado / de chefe.
local STAGGER = 0.3
local STAGGER_INTERRUPT = 0.5
local STAGGER_BOSS = 0.7
-- Cor do aviso de disparo dos atiradores.
local SHOT_COLOR = "#ff6a3d"
-- Aggro-chain: raio (m) em que o golpe do herói acorda os aliados do alvo
-- batido. É o mesmo raio do `on_player_attack` nativo (15 m).
local ALERT_RADIUS = 15.0
-- Elites: vida ×, dano ×, escala e cor da aura.
local ELITE_HP, ELITE_DAMAGE, ELITE_SCALE = 2.5, 1.4, 1.3
local ELITE_COLOR = "#ffd35a"
-- Estados nas criaturas (`rules.enemy_status`): queimadura por segundo,
-- lentidão do gelo e o teto do atordoamento do choque.
local BURN_DPS, FROST_SLOW, SHOCK_MAX = 8, 0.5, 1.5
-- De noite as criaturas `nocturnal` veem mais longe e batem mais.
local NIGHT_AGGRO, NIGHT_DAMAGE = 1.4, 1.25
-- Chuva forte: o raio de visão encolhe (aproximar-se à chuva é mais fácil).
local RAIN_SIGHT = 0.8
-- Renascer: segundos até a criatura voltar ao seu covil (game/spawns.lua).
local RESPAWN_SECS = 240
-- Pós-jogo (As Quatro Notas entregue): os elites são mais comuns.
local POSTGAME_ELITE = 0.22
local CAST_COLOR = "#b04cff"
local CHARGE_COLOR = "#ff3b1f"
-- A barra de chefe do HUD apaga-se este tempo após o último sinal (s).
M.BOSS_BAR_TTL = 1.5

local function dist2(ax, az, bx, bz)
  return math.sqrt((ax - bx) ^ 2 + (az - bz) ^ 2)
end

function M.new(cfg)
  cfg.wander = cfg.wander or 1.6
  cfg.chase = cfg.chase or 3.2
  cfg.aggro = cfg.aggro or 10
  cfg.deaggro = cfg.deaggro or 15
  cfg.range = cfg.range or 1.6
  cfg.damage = cfg.damage or 10
  cfg.cooldown = cfg.cooldown or 1.4
  cfg.windup = cfg.windup or 0.45
  cfg.recover = cfg.recover or 0.35
  cfg.radius = cfg.radius or 8
  cfg.kind = cfg.kind or "creature"
  cfg.hp = cfg.hp or 100
  if cfg.anti_stuck == nil then cfg.anti_stuck = true end
  if cfg.elite_chance == nil then cfg.elite_chance = cfg.boss and 0 or 0.08 end

  local self = { cfg = cfg }

  -- aggro-chain (loop 6): o herói acertou um aliado — força chase 10 s
  -- (desarma sozinho; o ataque usa a distância REAL, não a janela).
  function self.alert()
    viber.state().aggro_until = viber.time() + 10
  end

  local function pick_target(st)
    if not cfg.steering then
      local tx, tz = viber.wander_target(cfg.radius)
      st.target = { tx, tz }
      return
    end
    -- Steering: entre vários alvos válidos, escolhe o mais alinhado com o
    -- rumo atual — menos meia-volta, menos "andar de caranguejo".
    local x, _, z = viber.position()
    local hx, hz = st.hx or 0.0, st.hz or 1.0
    local best, best_dot
    for _ = 1, 6 do
      local tx, tz = viber.wander_target(cfg.radius)
      local dx, dz = tx - x, tz - z
      local len = math.sqrt(dx * dx + dz * dz)
      if len > 0.5 then
        local dot = (dx * hx + dz * hz) / len
        if not best_dot or dot > best_dot then
          best, best_dot = { tx, tz }, dot
        end
      end
    end
    st.target = best
  end

  -- Números efetivos (o enrage acelera o chefe).
  local function chase_speed(st)
    return cfg.chase * ((st.enraged and cfg.enrage and cfg.enrage.speed) or 1.0)
      * ((st.ph and st.ph.speed) or 1.0) * (st.slow and FROST_SLOW or 1.0)
  end
  local function cooldown(st)
    return cfg.cooldown * ((st.enraged and cfg.enrage and cfg.enrage.cooldown) or 1.0)
      * ((st.ph and st.ph.cooldown) or 1.0)
  end
  -- Config EFETIVA: a fase actual do chefe pode trocar o especial e ligar a
  -- investida; sem fases vale a config base.
  local function special_of(st)
    return (st.ph and st.ph.special) or cfg.special
  end
  local function charge_of(st)
    return (st.ph and st.ph.charge) or cfg.charge
  end
  local function night_mult(value)
    if cfg.nocturnal and clock.is_night() then
      return value
    end
    return 1.0
  end

  -- Invoca `count` criaturas de um `<Prototype>` num anel à volta do ponto.
  local function summon(spec, x, z)
    if type(spec) ~= "table" or not spec.proto then return end
    local count = math.max(1, math.floor(tonumber(spec.count) or 1))
    local radius = spec.radius or 3.5
    local base = math.random() * math.pi * 2
    for i = 1, count do
      local a = base + (i - 1) * (math.pi * 2 / count)
      local sx, sz = x + math.cos(a) * radius, z + math.sin(a) * radius
      -- Prototype em falta (mundo QA sem `world/gameplay.xml`) é erro de
      -- script: o chefe continua a lutar sem os reforços.
      -- Reforços não renascem: marca-os para a fila de `respawns` os ignorar.
      local ok = pcall(viber.spawn_prototype, spec.proto, sx, sz, {
        on_spawned = function(bits)
          local game = viber.game()
          game.summoned = type(game.summoned) == "table" and game.summoned or {}
          game.summoned[bits] = true
        end,
      })
      if ok then
        viber.ring(sx, sz, 1.2, spec.color or CAST_COLOR)
      end
    end
  end

  -- Teleporta a PRÓPRIA criatura (enterrar do Verme, salto da Bruxa). O Y vem
  -- do chão por baixo do ponto.
  local function warp(x, y, z)
    local gy = viber.ground_below and viber.ground_below(x, y + 30.0, z) or y
    viber.set_position(x, gy or y, z)
  end

  -- Dano recebido desde o último frame → flinch / poise / aggro.
  local function react_to_damage(st, hp, max)
    local lost = (st.hp or hp) - hp
    st.hp = hp
    if lost <= 0 then
      return
    end
    self.alert()
    local stagger
    if cfg.poise and cfg.poise > 0 then
      st.poise_dmg = (st.poise_dmg or 0) + lost
      if st.poise_dmg >= cfg.poise * max then
        st.poise_dmg = 0
        stagger = STAGGER_BOSS
      end
    else
      stagger = (st.phase == "windup") and STAGGER_INTERRUPT or STAGGER
    end
    if stagger then
      -- O flinch cancela um WINDUP em curso, mas `st.t` (cadência) ficava no
      -- valor que disparou o golpe: ao fim do stagger a criatura atacava no
      -- frame seguinte, sem cooldown. Só o windup interrompido precisa do
      -- reset — um flinch na espera não encurta nem estica a cadência.
      if st.phase == "windup" then
        st.t = 0
      end
      st.phase, st.pt = "stagger", stagger
      viber.play_clip("hit")
    end
  end

  -- Enrage: uma vez, abaixo da fração de HP.
  local function check_enrage(st, hp, max)
    local e = cfg.enrage
    if not e or st.enraged or max <= 0 or hp / max > (e.at or 0.4) then
      return
    end
    st.enraged = true
    viber.play_clip("roar")
    viber.sound("roar")
    viber.shake(0.18)
    local x, y, z = viber.position()
    viber.burst("fire", x, y + 1.2, z, 8)
    viber.ring(x, z, 3.2, "#ff3b1f")
    if e.toast then viber.toast(e.toast) end
  end

  -- Barra de chefe: sinal com TTL (o hud.lua apaga-a quando o sinal pára —
  -- um chefe morto deixa de correr o script e a barra desvanece sozinha).
  local function boss_bar(st, hp, max)
    if not cfg.boss then return end
    viber.ui.set("boss.active", true)
    viber.ui.set("boss.name", cfg.boss)
    viber.ui.set("boss", max > 0 and hp / max or 0)
    viber.ui.set("boss.enraged", st.enraged == true)
    viber.game().boss_bar_t = viber.time()
  end

  -- Começa um golpe: normal ou, a cada `special.every`, o especial.
  local function begin_attack(st, px, pz)
    st.swings = (st.swings or 0) + 1
    local sp = special_of(st)
    local every = sp and math.max(1, math.floor(tonumber(sp.every) or 3)) or 0
    if sp and st.swings % every == 0 then
      st.phase, st.pt, st.kind = "windup", sp.windup or 1.0, "special"
      local x, _, z = viber.position()
      if sp.at == "player" then
        st.ax, st.az = px, pz -- a zona fica onde o herói ESTAVA: sai dela
      else
        st.ax, st.az = x, z
      end
      viber.play_clip(sp.clip or "roar,attack")
      if sp.label then viber.toast(sp.label) end
      viber.sound("roar")
    else
      st.phase, st.pt, st.kind = "windup", cfg.windup, "normal"
      viber.play_clip("attack")
    end
    st.pulse = 0
  end

  -- Começa um disparo (atiradores): aviso curto, dispara no fim do windup.
  local function begin_shot(st, px, pz)
    if cfg.caster then
      -- Feiticeiro: a zona fica onde o herói ESTAVA — sair dela resolve.
      st.phase, st.pt, st.kind = "windup", cfg.caster.windup or 1.2, "cast"
      st.ax, st.az = px, pz
      st.pulse = 0
      viber.play_clip("cast,roar,attack")
      return
    end
    if not cfg.ranged then return end
    st.phase, st.pt, st.kind = "windup", cfg.ranged.windup or 0.7, "shot"
    st.pulse = 0
    viber.play_clip("shoot,attack")
  end

  -- Investida: trava, marca o corredor e arranca em linha recta. A direcção
  -- fica a do WINDUP — quem sai do corredor vê-a passar ao lado e ganha a
  -- janela de punição mais longa do jogo.
  local function begin_charge(st, ch, px, pz, x, z)
    local dx, dz = px - x, pz - z
    local len = math.sqrt(dx * dx + dz * dz)
    if len < 1e-3 then return false end
    dx, dz = dx / len, dz / len
    st.phase, st.pt, st.kind = "windup", ch.windup or 0.9, "charge"
    st.cdx, st.cdz = dx, dz
    st.clen = len + 3.0
    st.ax, st.az = x + dx * st.clen, z + dz * st.clen
    st.pulse = 0
    st.hx, st.hz = dx, dz
    viber.play_clip("roar,attack")
    viber.sound("roar")
    return true
  end

  -- Vira o corpo para o herói e guarda ESSA direção como rumo. O rumo do
  -- registo era só o da última marcha: uma criatura parada a atacar (ou o
  -- arqueiro na banda de tiro) ficava com `entry.fx/fz` a apontar para onde
  -- andou e o backstab do melee disparava com ela de frente para o herói.
  local function face_hero(st, px, pz, x, z)
    viber.face_player()
    local dx, dz = px - x, pz - z
    local len = math.sqrt(dx * dx + dz * dz)
    if len > 1e-3 then
      st.hx, st.hz = dx / len, dz / len
    end
  end

  -- Dano do golpe (elites batem mais).
  local function dmg(st)
    return cfg.damage * (st.elite and ELITE_DAMAGE or 1.0) * night_mult(NIGHT_DAMAGE)
  end

  -- Pedido de marcha + rumo do corpo. O rumo vem da DIREÇÃO PEDIDA (é para
  -- onde o rig se vira), nunca do deslocamento observado: depois de um
  -- stagger o knockback continua a arrastar a criatura para longe do herói
  -- e essa deriva entrava no registo como "costas" — o golpe seguinte do
  -- melee contava backstab (×2) sobre quem continuava de frente.
  local function step(st, x, z, ax, az, speed)
    local dx, dz = ax - x, az - z
    local len = math.sqrt(dx * dx + dz * dz)
    if len > 1e-3 then
      st.hx, st.hz = dx / len, dz / len
    end
    viber.move_towards(ax, az, speed)
  end

  -- Resolve o impacto do golpe em curso.
  local function land_attack(st)
    local has, px, py, pz = viber.player_position()
    if not has then return end
    local x, y, z = viber.position()
    if st.kind == "shot" then
      -- O projétil voa (tem tempo de viagem): sair da linha esquiva-o.
      viber.fire_projectile(cfg.ranged.projectile or "enemy-arrow")
      viber.sound("bow_shot", { x = x, y = y, z = z })
      return
    end
    if st.kind == "cast" or st.kind == "emerge" then
      -- Zona no chão (feitiço / o Verme a rebentar a areia).
      local spec = st.kind == "cast" and cfg.caster or (st.ph and st.ph.burrow) or {}
      local r = spec.radius or 2.6
      local gy = viber.ground_below and viber.ground_below(st.ax, y + 2.0, st.az) or y
      viber.burst(st.kind == "cast" and "magic" or "sand-dust", st.ax, gy + 0.3, st.az, 10)
      viber.shake(st.kind == "cast" and 0.08 or 0.2)
      viber.sound("hit", { x = st.ax, y = gy, z = st.az })
      if dist2(px, pz, st.ax, st.az) <= r and math.abs(py - gy) <= REACH_VERTICAL + 1.0 then
        if rules.damage_player(dmg(st) * (spec.mult or 1.2), x, z) and spec.on_hit then
          spec.on_hit()
        end
      else
        viber.damage_number("ESQUIVA", { color = "#9fd8ff", x = px, y = py + 2.0, z = pz })
      end
      return
    end
    if st.kind == "special" then
      local sp = special_of(st)
      if not sp then return end
      local r = sp.radius or 4.5
      -- A poeira nasce no CHÃO da zona (o centro pode estar acima/abaixo do
      -- chefe, ex.: maldição `at="player"` num barranco).
      local gy = viber.ground_below and viber.ground_below(st.ax, y + 2.0, st.az) or y
      viber.burst("ground-dust", st.ax, gy + 0.2, st.az, 6)
      viber.shake(0.16)
      viber.sound("hit")
      if dist2(px, pz, st.ax, st.az) <= r and math.abs(py - gy) <= REACH_VERTICAL + 1.0 then
        -- `on_hit` só quando o dano ENTRA: um parry/i-frame não envenena.
        -- A origem para o flash de direção é a CRIATURA (x,z): no especial
        -- `at="player"` o centro da zona é o sítio onde o herói ESTAVA, e a
        -- seta de dano apontava para os pés dele em vez de para o chefe.
        if rules.damage_player(dmg(st) * (sp.mult or 1.6), x, z) and sp.on_hit then
          sp.on_hit()
        end
      else
        viber.damage_number("ESQUIVA", { color = "#9fd8ff", x = px, y = py + 2.0, z = pz })
      end
    else
      if dist2(px, pz, x, z) <= cfg.range + REACH_SLACK and math.abs(py - y) <= REACH_VERTICAL then
        if rules.damage_player(dmg(st), x, z) and cfg.on_attack then
          cfg.on_attack()
        end
      else
        viber.sound("whoosh")
        viber.damage_number("ESQUIVA", { color = "#9fd8ff", x = px, y = py + 2.0, z = pz })
      end
    end
  end

  -- Fase de ataque ativa (windup/recover/stagger): devolve true se consumiu
  -- o frame (a criatura não anda nem reavalia a FSM).
  local function tick_phase(st, dt, px, pz, x, z)
    if not st.phase then return false end
    st.pt = (st.pt or 0) - dt
    if st.phase == "windup" then
      -- Hot-reload a meio do golpe pode tirar o especial/arco da config:
      -- aborta o golpe em curso em vez de indexar nil e matar o script.
      if (st.kind == "special" and not special_of(st))
          or (st.kind == "shot" and not cfg.ranged)
          or (st.kind == "cast" and not cfg.caster)
          or (st.kind == "charge" and not charge_of(st))
          or (st.kind == "emerge" and not (st.ph and st.ph.burrow)) then
        if st.burrowed then
          st.burrowed = false
          viber.set_visible(true)
        end
        -- Mesma cadência do flinch: um windup abortado deixava `st.t` no
        -- valor que o disparou e o golpe seguinte saía no frame a seguir.
        st.phase, st.pt, st.kind, st.t = nil, 0, nil, 0
        return true
      end
      -- Hot-reload a meio do golpe deixava `pulse` nil e o script morria.
      st.pulse = (st.pulse or 0) - dt
      if st.kind == "special" then
        -- Aviso pulsado da zona; a criatura fica parada a olhar o herói.
        if st.pulse <= 0 then
          local sp = special_of(st)
          st.pulse = TELEGRAPH_PULSE
          viber.ring(st.ax, st.az, sp.radius or 4.5, sp.color or "#ff4a2a")
        end
      elseif st.kind == "cast" then
        if st.pulse <= 0 then
          st.pulse = 0.5
          viber.ring(st.ax, st.az, cfg.caster.radius or 2.6, cfg.caster.color or CAST_COLOR)
        end
      elseif st.kind == "emerge" then
        if st.pulse <= 0 then
          local b = st.ph.burrow
          st.pulse = 0.4
          viber.ring(st.ax, st.az, b.radius or 4.5, b.color or "#ff8a2a")
          viber.burst("sand-dust", st.ax, (select(2, viber.position())) + 0.2, st.az, 4)
        end
      elseif st.kind == "charge" then
        -- O corredor: anéis ao longo da linha da investida.
        if st.pulse <= 0 then
          st.pulse = 0.45
          for i = 1, 4 do
            local f = st.clen * i / 4
            viber.ring(x + st.cdx * f, z + st.cdz * f, 1.1, CHARGE_COLOR)
          end
        end
      elseif st.kind == "shot" then
        -- Arco a tensar: anel pequeno no atirador (lê-se "vai disparar").
        if st.pulse <= 0 then
          st.pulse = TELEGRAPH_PULSE
          viber.ring(x, z, 0.9, SHOT_COLOR)
        end
      else
        if st.pulse <= 0 then
          st.pulse = TELEGRAPH_PULSE
          -- O anel é o aviso do alcance REAL do impacto (`range + slack`);
          -- com `range + 0.3` dava para ficar de fora do círculo e levar.
          viber.ring(x, z, cfg.range + REACH_SLACK, TELEGRAPH_COLOR)
        end
      end
      if st.kind == "charge" then
        -- Direcção TRANCADA: o corpo aponta o corredor, não segue o herói.
        viber.face_towards(st.ax, st.az)
      elseif st.kind ~= "emerge" then
        face_hero(st, px, pz, x, z)
      end
      if st.pt <= 0 then
        if st.kind == "charge" then
          local ch = charge_of(st)
          st.phase, st.pt = "charging", st.clen / math.max(1, ch.speed or 10) + 0.15
          st.charge_hit = false
          return true
        end
        if st.kind == "emerge" then
          st.burrowed = false
          viber.set_visible(true)
          viber.play_clip("roar,attack")
          viber.sound("roar")
        end
        land_attack(st)
        st.phase, st.pt = "recover", cfg.recover
        st.t = 0
      end
    elseif st.phase == "charging" then
      local ch = charge_of(st)
      if not ch then
        st.phase, st.pt, st.t = nil, 0, 0
        return true
      end
      step(st, x, z, st.ax, st.az, ch.speed or 10)
      if not st.charge_hit and dist2(px, pz, x, z) <= (ch.width or 1.5) then
        st.charge_hit = true
        if rules.damage_player(dmg(st) * (ch.mult or 1.8), x, z) then
          -- Atropelado: o herói é atirado para a frente da investida.
          viber.push_player(st.cdx * 2.5, st.cdz * 2.5)
          viber.shake(0.3)
        end
      end
      if st.pt <= 0 or dist2(st.ax, st.az, x, z) < 0.9 then
        -- Fim da corrida: janela de punição LONGA (falhou = fica exposto).
        st.phase, st.pt, st.t = "recover", ch.recover or 1.3, 0
        viber.burst("ground-dust", x, (select(2, viber.position())) + 0.2, z, 6)
      end
    elseif st.phase == "burrow" then
      -- Enterrado: invisível e intocável; no fim rebenta onde o herói ESTÁ.
      if st.pt <= 0 then
        local b = st.ph and st.ph.burrow
        if not b then
          st.burrowed = false
          viber.set_visible(true)
          st.phase = nil
          return true
        end
        local _, y = viber.position()
        st.ax, st.az = px, pz
        warp(px, y, pz)
        st.phase, st.pt, st.kind, st.pulse = "windup", b.windup or 1.0, "emerge", 0
        if b.label then viber.toast(b.label) end
      end
    elseif st.pt <= 0 then
      st.phase = nil
    end
    return true
  end

  -- Estados aplicados pelo herói (queimadura, gelo, choque).
  local function tick_status(st, entry, dt, x, y, z)
    st.slow = false
    if entry.invuln then return end
    local weak = cfg.weak or {}
    if (entry.burn or 0) > 0 then
      -- A chuva apaga o fogo ao dobro da velocidade.
      entry.burn = math.max(0, entry.burn - dt * (clock.raining() and 2 or 1))
      st.burn_t = (st.burn_t or 0) + dt
      if st.burn_t >= 1.0 then
        st.burn_t = st.burn_t - 1.0
        local d = math.floor(BURN_DPS * (weak.burn or 1.0))
        entry.hp = math.max(0, entry.hp - d)
        -- O tique da queimadura não é um golpe: sem isto cada segundo dava
        -- um flinch e a criatura ficava presa em stagger enquanto ardia.
        st.hp = entry.hp
        viber.damage_number("-" .. d, { color = "#ff8c47", x = x, y = y + 1.6, z = z })
        viber.burst("fire", x, y + 0.8, z, 3)
      end
    else
      st.burn_t = 0
    end
    if (entry.frost or 0) > 0 then
      entry.frost = math.max(0, entry.frost - dt)
      st.slow = true
    end
    if (entry.shock or 0) > 0 then
      local secs = math.min(SHOCK_MAX, entry.shock * (weak.shock or 1.0))
      entry.shock = 0
      -- Chefes só meio atordoam (têm poise); um chefe enterrado nem isso.
      if cfg.boss then secs = secs * 0.4 end
      if st.phase ~= "charging" and st.phase ~= "burrow" and not st.burrowed then
        if st.phase == "windup" then st.t = 0 end
        st.phase, st.pt = "stagger", secs
        viber.play_clip("hit")
        viber.ring(x, z, 1.4, "#8cd9ff")
      end
    end
  end

  -- Fases de chefe: ao cruzar o limiar, pausa curta (intocável), aviso e os
  -- reforços/poderes da fase nova.
  local function check_phases(st, entry, x, y, z)
    local phases = cfg.phases
    if type(phases) ~= "table" or entry.max <= 0 then return end
    local at = (st.phase_i or 0) + 1
    local ph = phases[at]
    if not ph or entry.hp / entry.max > (ph.at or 0) then return end
    st.phase_i, st.ph = at, ph
    if st.burrowed then
      st.burrowed = false
      viber.set_visible(true)
    end
    local pause = ph.pause or 1.4
    st.phase, st.pt, st.kind, st.t = "stagger", pause, nil, 0
    st.bt, st.blink_t, st.ct = 0, 0, 0
    entry.invuln_until = viber.time() + pause
    viber.play_clip("roar")
    viber.sound("roar")
    viber.shake(0.22)
    viber.ring(x, z, 4.0, ph.color or "#ff3b1f")
    viber.burst("magic", x, y + 1.2, z, 12)
    if ph.toast then viber.toast(ph.toast) end
    if ph.summon then summon(ph.summon, x, z) end
  end

  function self.update(dt)
    local st = viber.state() -- POR ENTIDADE (no top-level partilhava entre instâncias)
    local id = viber.self_id()
    local g = viber.game()
    local session = session_tag()
    -- Marca se o portão abriu NESTE frame: o registo é criado logo a seguir
    -- e o reset de "linha temporal nova" não pode apagar o `awake` que
    -- acabou de ser ganho (o aviso de despertar repetia no frame seguinte).
    local woke_now = false
    -- Chefe já derrotado: a cena recriou a entidade ao voltar de um interior.
    -- ANTES do gate (e do `awake_toast`): senão o aviso de despertar repetia
    -- a cada reentrada numa cena onde o chefe já caiu, e sem pagar XP/loot
    -- outra vez.
    if cfg.boss and g["boss_dead." .. cfg.kind] then
      if id then
        viber.entity_despawn(id)
      end
      return
    end
    if cfg.gate and not st.awake then
      -- Registo limpo (load) ou primeiro frame da entidade: o próprio ainda
      -- não está no registo, e os vizinhos também ainda não voltaram a
      -- registar-se. `alive_in_region` via a banda VAZIA e o portão abria
      -- por engano — espera um frame para o resto da cena registar-se.
      local reg = id and g.enemies and g.enemies[id] or nil
      if reg == nil and st.gate_session ~= session then
        st.gate_session = session
        return
      end
      if not cfg.gate() then
        return
      end
      st.awake = true
      woke_now = true
      if cfg.awake_toast then
        viber.toast(cfg.awake_toast)
      end
    end
    if cfg.on_first and not st.first_done then
      st.first_done = true
      cfg.on_first()
    end
    local has, px, _, pz = viber.player_position()
    -- O registo vivo não pode envelhecer durante uma pausa (menu, cinemática,
    -- LOD): `M.hostiles` só lista entradas com `t` fresco e a recriação
    -- devolvia HP cheio a um inimigo já ferido.
    local function touch_registry()
      if not id then return end
      local enemies = viber.game().enemies or {}
      local e = enemies[id]
      if e then e.t = viber.time() end
    end
    if not has then
      touch_registry()
      return
    end
    -- Cinemática a tocar: o mundo suspende a respiração — ninguém ataca nem
    -- persegue; quem estava a armar um golpe larga-o e encara o herói.
    if viber.cinematic_playing() then
      -- Só um windup interrompido deixa a cadência "carregada" (st.t já
      -- passou do cooldown); recuperá-la evita o golpe instantâneo ao
      -- voltar da cena.
      if st.phase == "windup" then st.t = 0 end
      st.phase = nil
      viber.face_player()
      touch_registry()
      return
    end
    -- Mapa, diário, loja: o golpe que já estava armado não aterra por baixo.
    if viber.menu_open() then
      if st.phase == "windup" then st.t = 0 end
      st.phase = nil
      touch_registry()
      return
    end
    local x, py, z = viber.position()
    -- Herói caído (à espera do respawn): ninguém morde o cadáver — larga o
    -- golpe em curso e volta ao passeio.
    local _, hero_hp = rules.player_hp()
    if hero_hp <= 0 then
      if st.phase == "windup" then st.t = 0 end
      st.phase, st.pt, st.state, st.aggro_until = nil, 0, "wander", 0
      px, pz = x + 1e4, z + 1e4
    end

    -- Aggro-chain (`on_player_attack` do nativo): a engine deixou de emitir
    -- `AttackAlert`; o `game/combat.lua`/`game/bomb-lua.lua` publicam o ponto
    -- do golpe numa tabela do `viber.game()` e os aliados a 15 m dele passam
    -- a perseguir durante 10 s (o ataque continua a medir a distância real).
    local alert = g.attack_alert
    local aseq = type(alert) == "table" and alert.seq or nil
    -- O `seq` recomeça em 1 quando o `viber.game()` é limpo (load): o `t`
    -- distingue um alerta novo de um `seq` repetido na linha temporal nova.
    if aseq and (aseq ~= st.alert_seq or alert.t ~= st.alert_t) then
      st.alert_seq, st.alert_t = aseq, alert.t
      if alert.t and (viber.time() - alert.t) <= 0.25
          and dist2(x, z, alert.x or 1e9, alert.z or 1e9) <= ALERT_RADIUS then
        st.aggro_until = viber.time() + 10
      end
    end

    g.enemies = g.enemies or {}
    local now = viber.time()
    local entry = id and g.enemies[id] or nil
    -- Save/restart deixa registos de OUTRA sessão. Uma pausa longa da MESMA
    -- sessão (menu, LOD) conserva o HP em vez de recriar a criatura cheia.
    local stale = entry ~= nil
      and (entry.t == nil or entry.t > now + 1.0 or entry.session ~= session)
    if entry == nil or stale then
      entry = {
        kind = cfg.kind, hp = cfg.hp, max = cfg.hp, boss = cfg.boss,
        x = x, y = py, z = z, t = now, session = session,
      }
      -- Registo recriado = criatura NOVA nesta linha temporal (load, reset do
      -- registo pelas regras, spawn). O `viber.state()` sobrevive à recriação,
      -- por isso os transientes do combate anterior têm de cair aqui: um
      -- `dead` antigo tornava a criatura imortal (hp 0 sem prémio/despawn) e
      -- um chefe enraivecido voltava com vida cheia E a fúria ligada.
      st.dead, st.phase, st.pt, st.kind = nil, nil, 0, nil
      st.t, st.pulse, st.swings = 0, 0, nil
      st.enraged, st.poise_dmg = nil, 0
      st.aggro_until, st.state, st.pstate = 0, nil, nil
      st.rt, st.flee_t, st.stuck, st.last_td = nil, 0, 0, nil
      st.flee_d = nil
      -- Fases, enterro e timers dos arquétipos também são desta luta.
      st.phase_i, st.ph, st.bt, st.blink_t, st.ct = nil, nil, 0, 0, nil
      st.burn_t, st.slow, st.charge_hit = 0, false, nil
      if st.burrowed then
        st.burrowed = false
        viber.set_visible(true)
      end
      -- O `awake`/`first_done` também são transientes de combate: um load na
      -- mesma cena sobrevive à entidade, e sem isto o portão do chefe já não
      -- era reavaliado (o ogro atacava com os guardiões ainda vivos). Não se
      -- apagam no frame em que o portão abriu (`woke_now`): o `awake` foi
      -- ganho agora e o `on_first` correu logo acima.
      if not woke_now then
        st.awake, st.first_done = nil, nil
      end
      st.hx, st.hz = nil, nil
      -- Elite já promovido não pode renascer com a vida base nem perder a
      -- marca que o loot paga a triplicar.
      if st.elite then
        entry.max = entry.max * ELITE_HP
        entry.hp = entry.max
        g.elites = g.elites or {}
        if id then g.elites[id] = true end
      end
      st.hp, st.max_hp = entry.hp, entry.max
      if id then g.enemies[id] = entry end
    end
    -- Elite: uma vez por entidade, no primeiro update.
    local promoted = false
    if st.elite == nil then
      local chance = cfg.elite_chance
      if chance > 0 and g["q.story_notas"] == "done" then
        chance = math.max(chance, POSTGAME_ELITE)
      end
      st.elite = chance > 0 and math.random() < chance
      if st.elite then
        entry.max = entry.max * ELITE_HP
        entry.hp = entry.max
        st.hp, st.max_hp = entry.hp, entry.max
        viber.set_scale(ELITE_SCALE)
        g.elites = g.elites or {}
        if id then g.elites[id] = true end
        promoted = true
      end
    end
    -- O registro é a fonte da verdade: o melee do jogador escreve `entry.hp`.
    -- `react_to_damage` compara com o `st.hp` anterior; sobrescrevê-lo aqui
    -- fazia `lost` ser sempre 0 (sem flinch, sem poise, sem aggro).
    entry.kind, entry.boss = cfg.kind, cfg.boss
    entry.x, entry.y, entry.z = x, py, z
    entry.fx, entry.fz = st.hx, st.hz
    entry.t = viber.time()
    -- Escudo: erguido salvo quando a criatura está exposta (recover/stagger).
    entry.shield = cfg.shield
    entry.guard = cfg.shield ~= nil and st.phase ~= "recover" and st.phase ~= "stagger"
    entry.invuln = st.burrowed == true or (entry.invuln_until or 0) > entry.t
    -- Sombras da noite: ao amanhecer desfazem-se (sem prémio nem cadáver).
    if cfg.fades_at_dawn and not st.dead and not clock.is_night() then
      viber.burst("magic", x, py + 1.0, z, 8)
      g.enemies[id or 0] = nil
      viber.despawn_self()
      return
    end
    if entry.hp <= 0 then
      if not st.dead then
        st.dead = true
        if cfg.boss then
          g["boss_dead." .. cfg.kind] = true
        end
        st.phase, st.state = nil, "dead"
        viber.play_clip("death", { lock = true })
        viber.sound("enemy_death")
        if id and viber.corpse then
          viber.corpse(id)
        end
        rules.award_kill(cfg.kind, id)
        local loot = viber.load("game/loot.lua")
        if loot and loot.pay then
          loot.pay(cfg.kind, id, x, py, z)
        end
        -- Renascer: a criatura volta ao covil daqui a uns minutos, longe da
        -- vista (game/spawns.lua lê esta fila; é uma TABELA, não vai no save).
        local summoned = type(g.summoned) == "table" and id and g.summoned[id]
        if summoned then
          g.summoned[id] = nil
        end
        if cfg.proto and not cfg.boss and cfg.respawn ~= false and not summoned then
          local hx, hz = viber.home()
          g.respawns = type(g.respawns) == "table" and g.respawns or {}
          if #g.respawns < 96 then
            g.respawns[#g.respawns + 1] = {
              proto = cfg.proto, x = hx or x, z = hz or z,
              at = viber.time() + RESPAWN_SECS,
            }
          end
        end
        if id then
          local dead = entry
          viber.after(2.6, function()
            local gg = viber.game()
            -- `viber.after` é world-scoped e os bits da entidade são
            -- REUTILIZADOS: com uma troca de cena (ou um load) pelo meio, o
            -- callback podia matar a criatura nova que herdou o id e limpar
            -- o registo dela. Só age se o registo ainda for ESTE abate.
            if not gg.enemies or gg.enemies[id] ~= dead then
              return
            end
            gg.enemies[id] = nil
            viber.entity_despawn(id)
          end)
        end
      end
      return
    end
    tick_status(st, entry, dt, x, py, z)
    if not promoted then
      react_to_damage(st, entry.hp, entry.max)
      check_enrage(st, entry.hp, entry.max)
      check_phases(st, entry, x, py, z)
    end

    -- Nominais + taxa de viragem do rig (1.ª vez só; desliga auto-calibração).
    if cfg.turn_rate and not st.loco then
      st.loco = true
      viber.set_locomotion(cfg.wander, cfg.chase, cfg.turn_rate)
    end

    if tick_phase(st, dt, px, pz, x, z) then
      -- `ok/hp/max` eram restos do antigo `viber.entity_hp()`: `ok` era nil
      -- e a barra de chefe nunca era publicada.
      if st.state == "chase" then boss_bar(st, entry.hp, entry.max) end
      return
    end

    local dist = dist2(px, pz, x, z)
    -- A janela de aggro força só a FSM a chase (o golpe mede a distância REAL).
    local fsm_dist = dist
    if (st.aggro_until or 0) > viber.time() then fsm_dist = 0 end
    -- De noite as criaturas nocturnas veem mais longe; à chuva todas veem pior.
    local sight = night_mult(NIGHT_AGGRO) * (clock.raining() and RAIN_SIGHT or 1.0)
    st.state = viber.next_state(st.state or "wander", fsm_dist, cfg.aggro * sight, cfg.deaggro * sight)

    -- Hook na TRANSIÇÃO wander → chase (não repete por tick).
    if st.state == "chase" and st.pstate ~= "chase" then
      -- Engajamento NOVO: a cadência do arco e a fuga recomeçam. Sem isto um
      -- alvo que saiu do raio e voltou (LOD/viagem rápida) encontrava o tiro
      -- já "carregado" pelo engajamento anterior e levava a flecha no frame.
      -- A cadência CORPO A CORPO (`st.t`) tem o mesmo problema e era a única
      -- que atravessava o engajamento: quem voltava ao alcance com `st.t`
      -- quase no cooldown levava o golpe logo no primeiro frame de contacto
      -- (o flinch do stagger e o cancel do windup zeravam, mas uma SAÍDA
      -- limpa deixava a carga). `nil`, e não 0, para o 1.º golpe sair a meio
      -- da cadência, como no primeiro avistamento.
      st.rt, st.flee_t, st.flee_d, st.t = nil, 0, nil, nil
      if cfg.intro then
        local key = "cine_" .. cfg.intro
        if not g[key] then
          g[key] = true
          viber.cinematic(cfg.intro, viber.self_id())
        end
      end
      if cfg.on_chase then cfg.on_chase() end
      if st.elite then
        -- Elite a acordar: aura dourada — o jogador sabe que este é diferente.
        local _, y = viber.position()
        viber.ring(x, z, 2.2, ELITE_COLOR)
        viber.burst("sparkle", x, y + 1.2, z, 6)
      end
    end
    st.pstate = st.state

    if st.state == "chase" then
      boss_bar(st, entry.hp, entry.max)
      -- Poderes de fase: o Verme enterra-se, a Bruxa salta de sítio.
      local burrow = st.ph and st.ph.burrow
      if burrow then
        st.bt = (st.bt or 0) + dt
        if st.bt >= (burrow.every or 9) then
          st.bt = 0
          st.burrowed = true
          st.phase, st.pt, st.kind = "burrow", burrow.under or 1.6, nil
          viber.set_visible(false)
          viber.burst("sand-dust", x, py + 0.3, z, 14)
          viber.sound("roar")
          return
        end
      end
      local blink = st.ph and st.ph.blink
      if blink then
        st.blink_t = (st.blink_t or 0) + dt
        if st.blink_t >= (blink.every or 7) then
          st.blink_t = 0
          local a = math.random() * math.pi * 2
          local r = blink.dist or 9
          viber.burst("magic", x, py + 1.0, z, 12)
          warp(px + math.cos(a) * r, py, pz + math.sin(a) * r)
          viber.burst("magic", px + math.cos(a) * r, py + 1.0, pz + math.sin(a) * r, 12)
          st.t = 0
          return
        end
      end
      -- Investida: só na banda de distância certa (nem colado, nem longe).
      local ch = charge_of(st)
      if ch and dist >= (ch.min or 5) and dist <= (ch.max or 12) then
        st.ct = (st.ct or (ch.cooldown or 6) * 0.5) + dt
        if st.ct >= (ch.cooldown or 6) then
          st.ct = 0
          if begin_charge(st, ch, px, pz, x, z) then
            return
          end
        end
      end
      local rg = cfg.ranged or cfg.caster
      -- Fora da banda de tiro o arqueiro não está encurralado: sem isto um
      -- `flee_t` antigo fazia-o lutar corpo a corpo ao voltar ao alcance.
      -- A cadência do disparo também recomeça: um alvo que sai e volta não
      -- pode encontrar a flecha já "carregada" (st.rt a meio do cooldown).
      if not rg or dist > rg.range then
        st.flee_t, st.flee_d = 0, nil
        -- `nil` (e não 0) para o 1.º tiro de um novo engajamento sair a meio
        -- da cadência, como no primeiro avistamento.
        st.rt = nil
      end
      if rg and dist <= rg.range then
        -- Atirador: abaixo de `min` recua (a arma de perto só sai encurralado);
        -- entre `min` e `range` dispara.
        if dist < (rg.min or 0) then
          st.flee_t = (st.flee_t or 0) + dt
          -- "Encurralado" = não conseguir ABRIR distância. A regra antiga
          -- (colado ao herói ≤ cfg.range) deixava o arqueiro PARADO a
          -- apanhar quando o recuo batia numa parede (sem flecha e sem
          -- adaga) para sempre. Enquanto a distância sobe 0,5 m a fuga está
          -- a resultar e o contador recomeça.
          st.flee_d = st.flee_d or dist
          if dist > st.flee_d + 0.5 then
            st.flee_d = dist
            st.flee_t = 0
          end
          if not (st.flee_t > 1.5) then
            -- Alvo espelhado pelo arqueiro. Com o herói exatamente em cima
            -- (dist≈0) o ponto era o PRÓPRIO arqueiro: `move_towards` não
            -- anda e ele ficava parado a apanhar. Recua contra o rumo.
            local ax, az = x + (x - px), z + (z - pz)
            if math.abs(ax - x) < 1e-3 and math.abs(az - z) < 1e-3 then
              ax, az = x - (st.hx or 0) * 2, z - (st.hz or 1) * 2
            end
            step(st, x, z, ax, az, chase_speed(st))
            return
          end
        else
          st.flee_t, st.flee_d = 0, nil
          face_hero(st, px, pz, x, z)
          st.rt = (st.rt or (rg.cooldown or 2.4) * 0.5) + dt
          if st.rt >= (rg.cooldown or 2.4) then
            st.rt = 0
            begin_shot(st, px, pz)
          end
          return
        end
      end
      if dist > cfg.range then
        step(st, x, z, px, pz, chase_speed(st))
        -- Ao chegar ao alcance, o 1.º golpe sai a meio da cadência (não
        -- depois de uma cadência inteira parada a olhar). Só semeia a
        -- cadência quando ela está VAZIA: reescrever 0.55 todos os frames
        -- deixava o jogador a arrastar o golpe para sempre ao oscilar no
        -- limite do alcance (a carga acumulada era deitada fora).
        if (st.t or 0) <= 0 then
          st.t = cooldown(st) * 0.55
        end
      else
        face_hero(st, px, pz, x, z)
        st.t = (st.t or 0) + dt
        if st.t >= cooldown(st) then
          st.t = 0
          begin_attack(st, px, pz)
        end
      end
    else
      if st.target == nil then pick_target(st) end
      -- Steering pode falhar as 6 amostras (todas a < 0,5 m): `best` fica
      -- nil e indexar `st.target[1]` derrubava o script (lobo/escorpião
      -- congelados com warn-once).
      if st.target == nil then return end
      local td = dist2(st.target[1], st.target[2], x, z)
      if td < 0.8 then
        st.target = nil
        st.stuck, st.last_td = 0, nil -- chegou: limpa o anti-stuck
      elseif cfg.anti_stuck then
        -- anti-stuck: td sem diminuir = preso num collider; repick após ~6 s
        if st.last_td ~= nil and td >= st.last_td then
          st.stuck = (st.stuck or 0) + dt
          if st.stuck > 6 then
            pick_target(st)
            st.stuck, st.last_td = 0, nil
            -- `pick_target` deixa `target` nil se as 6 amostras falharem.
            if st.target == nil then return end
            -- `td` era do alvo ANTIGO: comparar o alvo novo com ele sujava
            -- o anti-stuck durante um frame (e podia adiar o próximo repick).
            td = dist2(st.target[1], st.target[2], x, z)
          end
        else
          st.stuck = 0
        end
        st.last_td = td
        step(st, x, z, st.target[1], st.target[2], cfg.wander)
      else
        step(st, x, z, st.target[1], st.target[2], cfg.wander)
      end
    end
  end

  return self
end

return M
