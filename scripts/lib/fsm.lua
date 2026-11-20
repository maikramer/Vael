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
--   local fsm = vael.load("lib/fsm.lua").new{
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
--     on_chase = function() vael.sound("growl") end,  -- opcional: transição
--     on_attack = function() rules.apply_status("venom", 4) end, -- golpe que ACERTA
--     on_first = function() vael.log("ativo") end,    -- 1.ª vez por entidade
--     gate = function() return true end,  -- opcional: false = dorme (boss gating)
--     awake_toast = "…",              -- opcional: toast à 1.ª passagem do gate
--     -- Chefes:
--     boss = "Bruxa do Bosque",       -- nome na barra de chefe do HUD
--     poise = 0.12,                   -- fração do HP que faz vacilar
--     special = { every = 3, windup = 1.0, radius = 4.5, mult = 1.6,
--                 at = "self" | "player", clip = "roar", color = "#ff4a2a",
--                 label = "…", on_hit = function() … end },
--     -- `special` também aceita inner (centro seguro), circles/stride (corredor
--     -- fixo de círculos), echo (segunda zona no mesmo centro) e cue (pista do HUD).
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
-- `vael.game().elites[id]` e o `game/loot.lua` paga a triplicar.
--   function on_player_attack(px, pz) fsm.alert() end
--   function on_update(dt) fsm.update(dt) end
--
-- O estado é POR ENTIDADE (`vael.state()`); o objeto devolvido por `new`
-- partilha a CONFIG entre instâncias do mesmo script (top-level corre 1×).

local M = {}

local rules = vael.load("lib/rules.lua")
local clock = vael.load("lib/clock.lua")

-- Marca da sessão do MUNDO: `vael.game().enemies` não é persistido, mas o
-- `vael.game()` sobrevive ao hot-reload. A marca vive lá como TABELA (o save
-- só guarda primitivos, por isso morre no `load`) — uma constante calculada no
-- top-level deste módulo mudava a cada hot-reload de `lib/fsm.lua` e recriava
-- TODOS os registos com vida cheia a meio de uma luta.
local function session_tag()
  -- Token PRIMITIVO (string): as tabelas do `vael.game()` são partilhadas
  -- entre VMs por cópia, por isso uma tabela como token de identidade
  -- nunca voltava igual (`entry.session ~= session` em todos os frames —
  -- o registo era recriado a cada frame, a cadência do golpe voltava a 0 e
  -- as criaturas chegavam ao herói sem nunca atacar). O load substitui o
  -- estado e leva a chave: o token novo marca a linha temporal nova.
  -- Vive numa tabela partilhada (fora do save; o load limpa-a).
  local game = vael.game()
  local fsm_state = game.fsm
  if type(fsm_state) ~= "table" then
    game.fsm = {}
    fsm_state = game.fsm
  end
  local tag = fsm_state.session
  if type(tag) ~= "string" then
    tag = string.format("%.6f:%d", vael.time(), math.random(1, 1000000000))
    fsm_state.session = tag
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
local PARRY_RECOVER = 0.85
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
-- Furtividade / lanterna do herói: à noite, luz apagada não atrai inimigos;
-- lanterna acesa funciona como farol na escuridão (aumenta aggro e atrai
-- patrulhas próximas para investigar o brilho).
local LIGHT_NIGHT_OFF_AGGRO, LIGHT_NIGHT_OFF_DEAGGRO = 0.42, 0.65
local LIGHT_NIGHT_ON_AGGRO, LIGHT_NIGHT_ON_DEAGGRO = 1.45, 1.30
local LIGHT_NOCTURNAL_OFF_AGGRO, LIGHT_NOCTURNAL_OFF_DEAGGRO = 0.65, 0.80
local LIGHT_NOCTURNAL_ON_AGGRO, LIGHT_NOCTURNAL_ON_DEAGGRO = 1.30, 1.20
local LIGHT_DAY_ON_AGGRO, LIGHT_DAY_ON_DEAGGRO = 1.15, 1.10
local CROUCH_SIGHT, CROUCH_SIGHT_LIT = 0.72, 0.90
local LIGHT_INVESTIGATE_MULT = 1.40
-- Coordenação de grupo (Combat Director): só um inimigo ataca de cada vez;
-- os restantes cercam/flanqueiam num anel tático e revezam o turno.
local SQUAD_SHOUT_RADIUS = 9.0
local GROUP_SCAN_RADIUS = 18.0
local TURN_GAP = 0.42
local RESERVE_TIMEOUT = 2.4
local BID_WINDOW = 0.06
local ALLY_SEP_RADIUS = 1.65
-- Renascer: segundos até a criatura voltar ao seu covil (game/spawns.lua).
local RESPAWN_SECS = 240
-- Pós-jogo (As Quatro Notas entregue): os elites são mais comuns.
local POSTGAME_ELITE = 0.22
local CAST_COLOR = "#b04cff"
local CHARGE_COLOR = "#ff3b1f"
-- Distância mínima (m) ao herói, centro a centro; abaixo dela a criatura
-- recua a `BACKOFF_SPEED` m/s (ver o "espaço pessoal" no update).
local MIN_GAP = 0.9
local BACKOFF_SPEED = 3.0

-- A barra de chefe do HUD apaga-se este tempo após o último sinal (s).
M.BOSS_BAR_TTL = 1.5

-- Distância (m) ao herói a partir da qual a criatura só refresca o registo.
local FAR_IDLE = 90

local function dist2(ax, az, bx, bz)
  return math.sqrt((ax - bx) ^ 2 + (az - bz) ^ 2)
end

-- Estado da lanterna e postura furtiva do herói (`src/lantern.rs` / `src/player.rs`).
-- Devolve `nil` em mocks antigos sem a API da lanterna (preserva neutro 1.0).
local function player_lantern_lit()
  local f = vael.vael and vael.vael.lantern_lit
  if type(f) ~= "function" then return nil end
  return f() == true
end

local function player_crouching()
  local f = vael.vael and vael.vael.hero_crouching
  return type(f) == "function" and f() == true or false
end

-- Tabela partilhada do coordenador de grupo (`vael.game().combat_dir`).
-- Só guarda primitivos para sincronizar entre contextos BMS sem entrar no save.
local function combat_dir(g, session)
  local dir = g.combat_dir
  if type(dir) ~= "table" or dir.session ~= session then
    g.combat_dir = { session = session }
    dir = g.combat_dir
  end
  return dir
end

-- Liberta o token de ataque se pertencer a `id`. Quando `had_attacked` é true,
-- regista `last_id` e abre uma pausa curta (`TURN_GAP`) para o próximo aliado
-- entrar de forma legível.
local function release_attack_token(g, session, id, now, had_attacked)
  if not id then return end
  local dir = g.combat_dir
  if type(dir) ~= "table" or dir.session ~= session then return end
  if dir.attacker_id == id then
    dir.attacker_id = nil
    dir.attacker_until = 0
    if had_attacked then
      dir.last_id = id
      dir.cooldown_until = now + TURN_GAP
    end
  end
  if dir.bid_id == id then
    dir.bid_id = nil
    dir.bid_score = nil
    dir.bid_until = 0
  end
end

-- Verifica se existe um detentor VIVO e válido do token de ataque. Limpa tokens
-- órfãos (morte, stagger, fuga ou reserva expirada) para nunca travar o grupo.
local function active_token_holder(g, dir, session, now)
  local holder_id = dir.attacker_id
  if not holder_id then return nil end
  local enemies = g.enemies
  local holder = type(enemies) == "table" and enemies[holder_id] or nil
  local alive = holder and (holder.hp or 0) > 0 and holder.chasing == true
    and holder.session == session and holder.t and (now - holder.t) < 1.0
  if alive and (holder.attacking == true or (dir.attacker_until or 0) > now) then
    return holder_id
  end
  dir.attacker_id = nil
  dir.attacker_until = 0
  return nil
end

-- Normaliza diferença angular para [-pi, pi].
local function angle_diff(a, b)
  local d = (a - b) % (math.pi * 2)
  if d > math.pi then d = d - math.pi * 2 end
  return d
end

-- Recolhe aliados em perseguição perto do herói para coordenar turnos e
-- posições no anel de cerco. Só corre durante `chase` (1–7 inimigos típicos).
local function gather_group(g, self_id, session, now, px, pz, self_x, self_z, self_ranged, attacker_id)
  local enemies = g.enemies
  local total = 1
  local melee = {}
  local all = {}
  local hfx, hfz = 0, 1
  if vael.player_forward then
    local fx, fz = vael.player_forward()
    if fx and fz and (fx * fx + fz * fz) > 1e-4 then
      hfx, hfz = fx, fz
    end
  end
  local front_ang = math.atan2(hfz, hfx)
  if not self_ranged and attacker_id ~= self_id then
    melee[1] = {
      id = self_id or 0, x = self_x, z = self_z,
      rel = angle_diff(math.atan2(self_z - pz, self_x - px), front_ang),
      self_row = true,
    }
  end
  if type(enemies) == "table" then
    for eid, e in pairs(enemies) do
      if eid ~= self_id and type(e) == "table" and (e.hp or 0) > 0
          and e.chasing == true and e.session == session
          and e.t and (now - e.t) < 1.0 and e.x and e.z then
        if dist2(e.x, e.z, px, pz) <= GROUP_SCAN_RADIUS then
          total = total + 1
          all[#all + 1] = { id = eid, x = e.x, z = e.z, ranged = e.ranged == true, shielded = e.shielded == true }
          if not e.ranged and eid ~= attacker_id then
            melee[#melee + 1] = {
              id = eid, x = e.x, z = e.z,
              rel = angle_diff(math.atan2(e.z - pz, e.x - px), front_ang),
              shielded = e.shielded == true,
            }
          end
        end
      end
    end
  end
  -- Ordena pelo ângulo relativo à frente do herói (`[-pi, pi]`): sem saltos
  -- de ramo em ±pi e sem cruzamentos ao abrir o cerco.
  table.sort(melee, function(a, b)
    if a.rel ~= b.rel then return a.rel < b.rel end
    return (a.id or 0) < (b.id or 0)
  end)
  return total, melee, all, front_ang
end

-- Verifica se algum aliado está na linha de tiro entre `(x,z)` e `(px,pz)`.
-- Devolve o lado lateral (+1/-1) mais livre para o atirador abrir ângulo.
local function shot_lane_blocked(x, z, px, pz, allies)
  local dx, dz = px - x, pz - z
  local len = math.sqrt(dx * dx + dz * dz)
  if len < 1.5 then return 0 end
  local ux, uz = dx / len, dz / len
  local rx, rz = -uz, ux
  local blocked_side = 0
  for i = 1, #allies do
    local a = allies[i]
    local ax, az = a.x - x, a.z - z
    local along = ax * ux + az * uz
    if along > 0.9 and along < len - 0.8 then
      local side = ax * rx + az * rz
      if math.abs(side) < 0.85 then
        blocked_side = side >= 0 and -1 or 1
        break
      end
    end
  end
  return blocked_side
end

-- Separação suave entre aliados próximos para não andarem empilhados.
local function ally_separation(x, z, allies)
  local sx, sz = 0, 0
  for i = 1, #allies do
    local a = allies[i]
    local dx, dz = x - a.x, z - a.z
    local d = math.sqrt(dx * dx + dz * dz)
    if d > 1e-4 and d < ALLY_SEP_RADIUS then
      local w = (ALLY_SEP_RADIUS - d) / ALLY_SEP_RADIUS
      sx = sx + (dx / d) * w
      sz = sz + (dz / d) * w
    end
  end
  return sx, sz
end

-- Entrada no volume do golpe, em fração do frame. Coordenadas relativas
-- permitem comparar um herói em movimento com um aliado parado na trajetória.
local function charge_contact(x, y, z, dx, dy, dz, radius)
  local first, last = 0, 1
  local a = dx * dx + dz * dz
  local c = x * x + z * z - radius * radius
  if a < 1e-8 then
    if c > 0 then return nil end
  else
    local b = x * dx + z * dz
    local disc = b * b - a * c
    if disc < 0 then return nil end
    local root = math.sqrt(disc)
    first, last = math.max(first, (-b - root) / a), math.min(last, (-b + root) / a)
  end
  if math.abs(dy) < 1e-6 then
    if math.abs(y) > REACH_VERTICAL then return nil end
  else
    local lo, hi = (-REACH_VERTICAL - y) / dy, (REACH_VERTICAL - y) / dy
    first, last = math.max(first, math.min(lo, hi)), math.min(last, math.max(lo, hi))
  end
  if first <= last then return first end
end

-- Procura uma vez no arranque. Durante a corrida só lê as posições atuais
-- destes vizinhos, incluindo quem ainda estava a chegar ao corredor.
local function charge_neighbors(st, x, z)
  local ids = {}
  local self_id = vael.self_id()
  for _, h in ipairs(rules.hostiles_near((x + st.ax) / 2, (z + st.az) / 2, st.clen / 2 + 12)) do
    if h.id ~= self_id then ids[#ids + 1] = h.id end
  end
  return ids
end

local function charge_reach(radius, id)
  local contact = vael.vael.contact_distance
  local distance = id and contact and contact(vael.self_id(), id)
  return distance and math.max(radius, distance + 0.015) or radius
end

local function first_charge_obstacle(st, track, dx, dy, dz, radius)
  local enemies, now = vael.game().enemies, vael.time()
  local best, first, best_id
  for _, id in ipairs(track.ids) do
    local e = enemies[id]
    if e and (e.hp or 0) > 0 and not e.boss and not e.invuln and e.session == st.far_session
        and e.t and e.t <= now and now - e.t < 1 and e.x and e.y and e.z then
      -- A matilha pode arrancar ombro a ombro. Só esbarra em quem está à frente.
      local ahead = (e.x - track.x) * st.cdx + (e.z - track.z) * st.cdz
      if ahead > 0 then
        local t = charge_contact(track.x - e.x, track.y - e.y, track.z - e.z, dx, dy, dz, charge_reach(radius, id))
        if t and (not first or t < first or (t == first and id < best_id)) then
          best, first, best_id = e, t, id
        end
      end
    end
  end
  return best, first, best_id
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
    vael.state().aggro_until = vael.time() + 10
  end

  local function pick_target(st)
    if not cfg.steering then
      local tx, tz = vael.wander_target(cfg.radius)
      st.target = { tx, tz }
      return
    end
    -- Steering: entre vários alvos válidos, escolhe o mais alinhado com o
    -- rumo atual — menos meia-volta, menos "andar de caranguejo".
    local x, _, z = vael.position()
    local hx, hz = st.hx or 0.0, st.hz or 1.0
    local best, best_dot
    for _ = 1, 6 do
      local tx, tz = vael.wander_target(cfg.radius)
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
      local ok = pcall(vael.spawn_prototype, spec.proto, sx, sz, {
        on_spawned = function(bits)
          local game = vael.game()
          game.summoned = type(game.summoned) == "table" and game.summoned or {}
          game.summoned[bits] = true
        end,
      })
      if ok then
        vael.ring(sx, sz, 1.2, spec.color or CAST_COLOR)
      end
    end
  end

  -- Teleporta a PRÓPRIA criatura (enterrar do Verme, salto da Bruxa). O Y vem
  -- do chão por baixo do ponto.
  local function warp(x, y, z)
    local gy = vael.ground_below and vael.ground_below(x, y + 30.0, z) or y
    vael.set_position(x, gy or y, z)
  end

  -- Criaturas sem rig bípede (ficam `PuppetIncompatible` em `src/puppet.rs`):
  -- mantêm o clip `"hit"` clássico; bípedes usam o HitReaction físico 360°.
  local NON_BIPED_KINDS = {
    wolf = true,
    scorpion = true,
    slime = true,
    ["sand-worm"] = true,
  }

  -- Multiplicadores de visão (noite, lanterna do herói, agachar, chuva).
  -- À noite: luz apagada reduz fortemente a visão (não atrai inimigos);
  -- lanterna acesa atrai inimigos de mais longe.
  local function sight_multipliers()
    local rain = clock.raining() and RAIN_SIGHT or 1.0
    local lit = player_lantern_lit()
    local crouch = player_crouching()
    if lit == nil then
      local base = night_mult(NIGHT_AGGRO) * rain * (crouch and CROUCH_SIGHT or 1.0)
      return base, base, nil
    end
    if clock.is_night() then
      if cfg.nocturnal then
        if lit then
          local c = crouch and CROUCH_SIGHT_LIT or 1.0
          return NIGHT_AGGRO * LIGHT_NOCTURNAL_ON_AGGRO * rain * c,
            NIGHT_AGGRO * LIGHT_NOCTURNAL_ON_DEAGGRO * rain,
            true
        else
          local c = crouch and CROUCH_SIGHT or 1.0
          return NIGHT_AGGRO * LIGHT_NOCTURNAL_OFF_AGGRO * rain * c,
            NIGHT_AGGRO * LIGHT_NOCTURNAL_OFF_DEAGGRO * rain,
            false
        end
      elseif lit then
        local c = crouch and CROUCH_SIGHT_LIT or 1.0
        return LIGHT_NIGHT_ON_AGGRO * rain * c, LIGHT_NIGHT_ON_DEAGGRO * rain, true
      else
        local c = crouch and CROUCH_SIGHT or 1.0
        return LIGHT_NIGHT_OFF_AGGRO * rain * c, LIGHT_NIGHT_OFF_DEAGGRO * rain, false
      end
    else
      if lit then
        local c = crouch and CROUCH_SIGHT_LIT or 1.0
        return LIGHT_DAY_ON_AGGRO * rain * c, LIGHT_DAY_ON_DEAGGRO * rain, true
      else
        local c = crouch and CROUCH_SIGHT or 1.0
        return rain * c, rain, false
      end
    end
  end

  -- Tenta reservar ou manter o token de ataque do grupo (1 atacante de cada vez).
  -- Em 1v1 (`total_chasing <= 1`) concede na hora, sem atraso de leilão.
  local function try_claim_attack_token(g, session, id, st, dist, total_chasing, now, in_range)
    if not id or total_chasing <= 1 then
      if id then
        local dir = combat_dir(g, session)
        dir.attacker_id = id
        dir.attacker_until = now + RESERVE_TIMEOUT
      end
      return true
    end
    local dir = combat_dir(g, session)
    local holder = active_token_holder(g, dir, session, now)
    if holder == id then
      dir.attacker_until = now + RESERVE_TIMEOUT
      return true
    end
    if holder ~= nil then
      return false
    end
    if now < (dir.cooldown_until or 0) then
      return false
    end
    -- Pontuação tática: chefes, contra-ataque de escudo, retaliação e quem
    -- espera há mais tempo sobem; quem atacou por último cede a vez ao grupo.
    local score = 0
    if cfg.boss then score = score + 100 end
    if (st.counter_left or 0) > 0 then score = score + 45 end
    if (st.retaliate_until or 0) > now then score = score + 30 end
    score = score + math.min(60, (st.wait_t or 0) * 14)
    if in_range then score = score + 18 end
    score = score - math.min(20, dist * 1.5)
    if dir.last_id == id then score = score - 50 end

    local bid_id = dir.bid_id
    local bid_until = dir.bid_until or 0
    -- Verifica se o licitante atual ainda está vivo e a perseguir.
    if bid_id then
      local e = g.enemies and g.enemies[bid_id] or nil
      local bid_alive = e and (e.hp or 0) > 0 and e.chasing == true
        and e.session == session and e.t and (now - e.t) < 1.0
      if not bid_alive or now > bid_until + 0.35 then
        bid_id = nil
        dir.bid_id, dir.bid_score, dir.bid_until = nil, nil, 0
      end
    end
    if bid_id == nil then
      dir.bid_id = id
      dir.bid_score = score
      dir.bid_until = now + BID_WINDOW
      return false
    end
    if now < bid_until then
      if bid_id == id then
        dir.bid_score = score
      elseif score > (dir.bid_score or -1e9) then
        dir.bid_id = id
        dir.bid_score = score
      end
      return false
    end
    if bid_id == id then
      dir.attacker_id = id
      dir.attacker_until = now + RESERVE_TIMEOUT
      dir.bid_id, dir.bid_score, dir.bid_until = nil, nil, 0
      st.wait_t = 0
      return true
    end
    return false
  end

  -- Dano recebido desde o último frame → flinch / poise / aggro.
  local function react_to_damage(st, hp, max, shield_chip)
    local lost = (st.hp or hp) - hp
    st.hp = hp
    if lost <= 0 then
      return
    end
    self.alert()
    -- Golpear um inimigo em espera dá-lhe prioridade para retaliar no turno seguinte.
    st.retaliate_until = vael.time() + 4.0
    -- Um corte que ressalta no escudo tira vida, mas a guarda só abre
    -- com o remate, pelas costas ou quando outro golpe o interrompe.
    lost = lost - shield_chip
    if lost < 1e-6 then return end
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
      local directional_puppet = not NON_BIPED_KINDS[cfg.kind or ""]
        and vael.vael and vael.vael.puppet_enabled and vael.vael.puppet_enabled()
      if not directional_puppet then
        vael.play_clip("hit")
      end
    end
  end

  -- Enrage: uma vez, abaixo da fração de HP.
  local function check_enrage(st, hp, max)
    local e = cfg.enrage
    if not e or st.enraged or max <= 0 or hp / max > (e.at or 0.4) then
      return
    end
    st.enraged = true
    vael.play_clip("roar")
    vael.sound("roar")
    vael.shake(0.18)
    local x, y, z = vael.position()
    vael.burst("fire", x, y + 1.2, z, 8)
    vael.ring(x, z, 3.2, "#ff3b1f")
    if e.toast then vael.toast(e.toast) end
  end

  -- Barra de chefe: sinal com TTL (o hud.lua apaga-a quando o sinal pára —
  -- um chefe morto deixa de correr o script e a barra desvanece sozinha).
  local function boss_bar(st, hp, max)
    if not cfg.boss then return end
    vael.ui.set("boss.active", true)
    vael.ui.set("boss.name", cfg.boss)
    vael.ui.set("boss", max > 0 and hp / max or 0)
    vael.ui.set("boss.enraged", st.enraged == true)
    vael.game().boss_bar_t = vael.time()
  end

  -- O primeiro corte e a resposta do bandido comprometem-se, cada um,
  -- com o círculo mostrado ao jogador no início daquele movimento.
  local function begin_cut(st, kind, windup, recover)
    local x, _, z = vael.position()
    local reach = cfg.range + REACH_SLACK
    st.phase, st.pt, st.kind = "windup", windup, kind
    st.ax = x + (st.hx or 0) * reach * 0.45
    st.az = z + (st.hz or 1) * reach * 0.45
    st.radius = reach * 0.55
    st.color = kind == "normal" and TELEGRAPH_COLOR or "#ff8060"
    st.pulse = TELEGRAPH_PULSE
    vael.ring(st.ax, st.az, st.radius, st.color)
    vael.play_clip("attack", { blend = 0.08, duration = windup + recover })
  end

  local function bow_attack(kind)
    return kind == "shot" or kind == "paired_shot" or kind == "second_arrow"
  end

  local function active_special(st)
    local sp = special_of(st)
    if st.kind == "echo" then return sp and sp.echo end
    return sp
  end

  -- Os anéis do aviso e do impacto usam os mesmos centros. A direção do
  -- ferrão fica gravada ao armar, mesmo que o herói passe para as costas.
  local function zone_center(st, sp, index)
    local offset = index * (sp.stride or 0)
    return st.ax + (st.zone_fx or 0) * offset, st.az + (st.zone_fz or 0) * offset
  end

  local function warn_special(st, sp)
    for i = 1, sp.circles or 1 do
      local x, z = zone_center(st, sp, i)
      vael.ring(x, z, sp.radius or 4.5, sp.color or "#ff4a2a")
      if sp.inner then vael.ring(x, z, sp.inner, "#91e6cb") end
    end
    st.pulse = TELEGRAPH_PULSE
  end

  local function arm_special(st, sp, kind)
    st.phase, st.pt, st.kind = "windup", sp.windup or 1.0, kind
    warn_special(st, sp)
    vael.play_clip(sp.clip or "roar,attack", { blend = 0.08, duration = st.pt + (sp.recover or cfg.recover) })
    if sp.label then vael.toast(sp.label) end
    vael.sound(sp.sound or "roar")
  end

  -- Começa um golpe: normal ou, a cada `special.every`, o especial.
  local function begin_attack(st, px, pz)
    st.swings = (st.swings or 0) + 1
    local sp = special_of(st)
    local every = sp and math.max(1, math.floor(tonumber(sp.every) or 3)) or 0
    if sp and (st.swings % every == 0 or (sp.after_block and (st.counter_left or 0) > 0)) then
      st.counter_left = nil
      local x, _, z = vael.position()
      st.ax, st.az = x, z
      if sp.at == "player" then st.ax, st.az = px, pz end
      st.zone_fx, st.zone_fz = st.hx, st.hz
      arm_special(st, sp, "special")
    elseif cfg.double_cut and st.swings % cfg.double_cut.every == 0 then
      begin_cut(st, "double", cfg.double_cut.windup, cfg.recover)
    else
      begin_cut(st, "normal", cfg.windup, cfg.recover)
    end
  end

  -- Começa um disparo (atiradores): aviso curto, dispara no fim do windup.
  local function begin_shot(st, px, pz)
    if cfg.caster then
      -- Feiticeiro: a zona fica onde o herói ESTAVA — sair dela resolve.
      st.phase, st.pt, st.kind = "windup", cfg.caster.windup or 1.2, "cast"
      st.ax, st.az = px, pz
      st.pulse = 0
      vael.play_clip("spellcast,cast,roar,attack", { blend = 0.08, duration = st.pt + cfg.recover })
      return
    end
    if not cfg.ranged then return end
    st.volleys = (st.volleys or 0) + 1
    st.phase, st.pt, st.kind = "windup", cfg.ranged.windup or 0.7, "shot"
    if cfg.ranged.pair and st.volleys % cfg.ranged.pair.every == 0 then
      st.kind = "paired_shot"
    end
    st.pulse = 0
    vael.play_clip("shoot,attack", { blend = 0.08, duration = st.pt + cfg.recover })
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
    st.charge_track, st.lane_blocked = nil, false
    st.cdx, st.cdz = dx, dz
    st.clen = len + 3.0
    st.ax, st.az = x + dx * st.clen, z + dz * st.clen
    st.pulse = 0
    st.hx, st.hz = dx, dz
    vael.play_clip("roar,attack", { blend = 0.08, duration = st.pt })
    vael.sound("roar")
    return true
  end

  -- Vira o corpo para o herói e guarda ESSA direção como rumo. O rumo do
  -- registo era só o da última marcha: uma criatura parada a atacar (ou o
  -- arqueiro na banda de tiro) ficava com `entry.fx/fz` a apontar para onde
  -- andou e o backstab do melee disparava com ela de frente para o herói.
  local function face_hero(st, px, pz, x, z)
    vael.face_player()
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
    vael.move_towards(ax, az, speed)
  end

  -- Movimento tático de espera no anel de cerco enquanto outro aliado ataca:
  -- espalha os inimigos à volta do herói, contorna pelo perímetro (nunca
  -- atravessa o herói em linha reta) e alterna entre vigiar e rondar flancos.
  local function hold_combat_ring(st, dt, px, pz, x, z, dist, melee_group, allies, ch, front_ang)
    local n = #melee_group
    local self_id = vael.self_id() or 0
    local slot_i = 1
    for i = 1, n do
      if melee_group[i].self_row or melee_group[i].id == self_id then
        slot_i = i
        break
      end
    end
    local cur_ang = math.atan2(z - pz, x - px)
    -- Cerco progressivo dos aliados em espera: 2 abrem nos flancos (±65°);
    -- 3 abrem em leque amplo; 4+ fecham o círculo completo à volta do herói.
    local desired_ang
    if n <= 1 then
      desired_ang = cur_ang
    elseif n == 2 then
      desired_ang = front_ang + (slot_i == 1 and -1.15 or 1.15)
    elseif n == 3 then
      local frac = (slot_i - 1) / 2 - 0.5
      desired_ang = front_ang + frac * 2.5
    else
      desired_ang = front_ang + (slot_i - 1) * (math.pi * 2 / n)
    end
    -- Escudeiros puxam ligeiramente para a frente do herói (vanguarda).
    if cfg.shield then
      desired_ang = desired_ang + angle_diff(front_ang, desired_ang) * 0.35
    end
    -- Direção orgânica de ronda (alterna entre vigiar de frente e flanquear).
    st.prowl_dir = st.prowl_dir or ((slot_i % 2 == 0) and 1 or -1)
    st.prowl_timer = (st.prowl_timer or (1.1 + (slot_i % 3) * 0.35)) - dt
    if st.prowl_timer <= 0 then
      st.prowl_moving = not st.prowl_moving
      if st.prowl_moving then
        st.prowl_timer = 0.95 + (slot_i % 3) * 0.2
        st.prowl_flips = (st.prowl_flips or 0) + 1
        if st.prowl_flips % 3 == 0 then
          st.prowl_dir = -st.prowl_dir
        end
      else
        st.prowl_timer = 1.2 + (slot_i % 3) * 0.25
      end
    end
    if st.prowl_moving then
      desired_ang = desired_ang + st.prowl_dir * 0.28
    end
    -- Raio do anel tático: escudeiro segura mais perto; investidores preparam
    -- na distância de arranque; restantes mantêm ~3,6 m para dar espaço ao duelo.
    local ring_r = math.max(cfg.range + 1.8, 3.6)
    if cfg.shield then
      ring_r = math.max(cfg.range + 1.1, 3.0)
    elseif ch and (st.ct or 0) >= (ch.cooldown or 6) * 0.65 then
      ring_r = math.max((ch.min or 5) + 0.8, 5.2)
    end
    local sx, sz = ally_separation(x, z, allies)
    -- Contorno pelo arco: se o alvo estiver longe em ângulo, avança por um
    -- ponto intermédio no perímetro em vez de atravessar o herói em linha reta.
    local dang = angle_diff(desired_ang, cur_ang)
    local step_ang = cur_ang + math.max(-0.65, math.min(0.65, dang))
    local step_r = dist < ring_r and ring_r or math.max(ring_r, dist - 1.5)
    local tx = px + math.cos(step_ang) * step_r + sx * 1.1
    local tz = pz + math.sin(step_ang) * step_r + sz * 1.1
    local slot_x = px + math.cos(desired_ang) * ring_r
    local slot_z = pz + math.sin(desired_ang) * ring_r
    local err = dist2(x, z, slot_x, slot_z)
    if dist < ring_r - 0.85 and dist < 2.9 then
      -- Herói encostou ou o inimigo acabou o seu turno: recua primeiro para o anel.
      local rx = px + math.cos(cur_ang) * (ring_r + 0.4) + sx * 0.9
      local rz = pz + math.sin(cur_ang) * (ring_r + 0.4) + sz * 0.9
      step(st, x, z, rx, rz, chase_speed(st) * 0.85)
    elseif err > 0.75 then
      local approach_mult = err > 3.0 and 0.88 or 0.68
      step(st, x, z, tx, tz, chase_speed(st) * approach_mult)
    elseif st.prowl_moving then
      local ox = px + math.cos(cur_ang + st.prowl_dir * 0.35) * ring_r + sx * 0.8
      local oz = pz + math.sin(cur_ang + st.prowl_dir * 0.35) * ring_r + sz * 0.8
      step(st, x, z, ox, oz, chase_speed(st) * 0.46)
    else
      face_hero(st, px, pz, x, z)
    end
  end

  -- Resolve o impacto do golpe em curso.
  local function land_attack(st)
    local has, px, py, pz = vael.player_position()
    if not has then return end
    local x, y, z = vael.position()
    if bow_attack(st.kind) then
      -- O projétil voa (tem tempo de viagem): sair da linha esquiva-o.
      vael.fire_projectile(cfg.ranged.projectile or "enemy-arrow")
      vael.sound("bow_shot", { x = x, y = y, z = z })
      return
    end
    if st.kind == "cast" or st.kind == "emerge" then
      -- Zona no chão (feitiço / o Verme a rebentar a areia).
      local spec = st.kind == "cast" and cfg.caster or (st.ph and st.ph.burrow) or {}
      local r = spec.radius or 2.6
      local gy = vael.ground_below and vael.ground_below(st.ax, y + 2.0, st.az) or y
      vael.burst(st.kind == "cast" and "magic" or "sand-dust", st.ax, gy + 0.3, st.az, 10)
      vael.shake(st.kind == "cast" and 0.08 or 0.2)
      vael.sound("hit", { x = st.ax, y = gy, z = st.az })
      if dist2(px, pz, st.ax, st.az) <= r and math.abs(py - gy) <= REACH_VERTICAL + 1.0 then
        if rules.damage_player(dmg(st) * (spec.mult or 1.2), x, z) and spec.on_hit then
          spec.on_hit()
        end
      end
      return
    end
    if st.kind == "special" or st.kind == "echo" then
      local sp = active_special(st)
      if not sp then return end
      local hit = false
      for i = 1, sp.circles or 1 do
        local cx, cz = zone_center(st, sp, i)
        local gy = vael.ground_below and vael.ground_below(cx, y + 2.0, cz) or y
        vael.burst(sp.fx or "ground-dust", cx, gy + 0.2, cz, 6)
        local d = dist2(px, pz, cx, cz)
        if d >= (sp.inner or 0) and d <= (sp.radius or 4.5)
            and math.abs(py - gy) <= REACH_VERTICAL + (cfg.boss and 1.0 or 0) then
          hit = true
        end
      end
      vael.shake(0.16)
      vael.sound("hit")
      if hit then
        -- `on_hit` só quando o dano ENTRA: um parry/i-frame não envenena.
        -- A origem para o flash de direção é a CRIATURA (x,z): no especial
        -- `at="player"` o centro da zona é o sítio onde o herói ESTAVA, e a
        -- seta de dano apontava para os pés dele em vez de para o chefe.
        local entered, parried = rules.damage_player(dmg(st) * (sp.mult or 1.6), x, z)
        if entered and sp.on_hit then sp.on_hit() end
        return parried
      end
    else
      if dist2(px, pz, st.ax, st.az) <= st.radius and math.abs(py - y) <= REACH_VERTICAL then
        local mult = (st.kind == "double" or st.kind == "followup") and cfg.double_cut.mult or 1
        local entered, parried = rules.damage_player(dmg(st) * mult, x, z)
        if entered and cfg.on_attack then cfg.on_attack() end
        return parried
      else
        vael.sound("whoosh")
      end
    end
  end

  -- Fase de ataque ativa (windup/recover/stagger): devolve true se consumiu
  -- o frame (a criatura não anda nem reavalia a FSM).
  local function tick_phase(st, dt, px, hero_y, pz, x, y, z)
    if not st.phase then return false end
    st.pt = (st.pt or 0) - dt
    if st.phase == "windup" then
      -- Hot-reload a meio do golpe pode tirar o especial/arco da config:
      -- aborta o golpe em curso em vez de indexar nil e matar o script.
      if (st.kind == "normal" and not st.radius)
          or ((st.kind == "double" or st.kind == "followup") and not cfg.double_cut)
          or ((st.kind == "special" or st.kind == "echo") and not active_special(st))
          or ((st.kind == "paired_shot" or st.kind == "second_arrow") and not (cfg.ranged and cfg.ranged.pair))
          or (st.kind == "shot" and not cfg.ranged)
          or (st.kind == "cast" and not cfg.caster)
          or (st.kind == "charge" and not charge_of(st))
          or (st.kind == "emerge" and not (st.ph and st.ph.burrow)) then
        if st.burrowed then
          st.burrowed = false
          vael.set_visible(true)
        end
        -- Mesma cadência do flinch: um windup abortado deixava `st.t` no
        -- valor que o disparou e o golpe seguinte saía no frame a seguir.
        st.phase, st.pt, st.kind, st.t = nil, 0, nil, 0
        return true
      end
      -- Fechar a distância durante a segunda puxada obriga o arqueiro
      -- a baixar o arco. Sair outra vez da banda não recupera a flecha.
      if st.kind == "second_arrow" then
        local dist = dist2(x, z, px, pz)
        if dist < (cfg.ranged.min or 0) or dist > cfg.ranged.range or math.abs(hero_y - y) > REACH_VERTICAL then
          st.phase, st.pt, st.t = "recover", cfg.ranged.pair.recover, 0
          return true
        end
      end
      -- Hot-reload a meio do golpe deixava `pulse` nil e o script morria.
      st.pulse = (st.pulse or 0) - dt
      if st.kind == "special" or st.kind == "echo" then
        -- Aviso pulsado da zona; a criatura fica parada a olhar o herói.
        if st.pulse <= 0 then
          warn_special(st, active_special(st))
        end
      elseif st.kind == "cast" then
        if st.pulse <= 0 then
          st.pulse = 0.5
          vael.ring(st.ax, st.az, cfg.caster.radius or 2.6, cfg.caster.color or CAST_COLOR)
        end
      elseif st.kind == "emerge" then
        if st.pulse <= 0 then
          local b = st.ph.burrow
          st.pulse = 0.4
          vael.ring(st.ax, st.az, b.radius or 4.5, b.color or "#ff8a2a")
          vael.burst("sand-dust", st.ax, (select(2, vael.position())) + 0.2, st.az, 4)
        end
      elseif st.kind == "charge" then
        -- O corredor: anéis ao longo da linha da investida.
        if st.pulse <= 0 then
          st.pulse = 0.45
          local ch = charge_of(st)
          if ch.trample_push then
            local track = st.charge_track or { ids = charge_neighbors(st, x, z) }
            track.x, track.y, track.z = x, y, z
            st.charge_track = track
            st.lane_blocked = first_charge_obstacle(st, track, st.ax - x, 0, st.az - z,
                ch.width or 1.5) ~= nil
          end
          for i = 1, 4 do
            local f = st.clen * i / 4
            vael.ring(x + st.cdx * f, z + st.cdz * f, 1.1, CHARGE_COLOR)
          end
        end
      elseif bow_attack(st.kind) then
        -- Arco a tensar: anel pequeno no atirador (lê-se "vai disparar").
        if st.pulse <= 0 then
          st.pulse = TELEGRAPH_PULSE
          vael.ring(x, z, 0.9, SHOT_COLOR)
          if st.kind == "paired_shot" then vael.ring(x, z, 1.55, SHOT_COLOR) end
        end
      else
        if st.pulse <= 0 then
          st.pulse = TELEGRAPH_PULSE
          vael.ring(st.ax, st.az, st.radius, st.color or TELEGRAPH_COLOR)
        end
      end
      if st.kind == "special" and active_special(st).stride then
        local tx, tz = zone_center(st, active_special(st), 1)
        vael.face_towards(tx, tz)
      elseif st.kind == "charge" or st.kind == "normal" or st.kind == "double" or st.kind == "followup" then
        -- Direcção TRANCADA: o corpo aponta o corredor, não segue o herói.
        vael.face_towards(st.ax, st.az)
      elseif st.kind ~= "emerge" then
        face_hero(st, px, pz, x, z)
      end
      if st.pt <= 0 then
        if st.kind == "charge" then
          local ch = charge_of(st)
          st.phase, st.pt = "charging", st.clen / math.max(1, ch.speed or 10) + 0.15
          st.charge_hit = false
          if ch.trample_push then
            local track = st.charge_track or { ids = charge_neighbors(st, x, z) }
            track.x, track.y, track.z, track.px, track.py, track.pz = x, y, z, px, hero_y, pz
            st.charge_track = track
          else
            st.charge_track = nil
          end
          return true
        end
        if st.kind == "emerge" then
          st.burrowed = false
          vael.set_visible(true)
          vael.play_clip("roar,attack")
          vael.sound("roar")
        end
        local parried = land_attack(st)
        if st.kind == "special" and not parried and active_special(st).echo then
          arm_special(st, active_special(st).echo, "echo")
          return true
        end
        if st.kind == "paired_shot" then
          st.kind, st.pt, st.pulse = "second_arrow", cfg.ranged.pair.gap, 0
          vael.play_clip("shoot,attack", { blend = 0.08, duration = st.pt })
          return true
        end
        if st.kind == "double" and not parried then
          face_hero(st, px, pz, x, z)
          begin_cut(st, "followup", cfg.double_cut.gap, cfg.double_cut.recover)
          return true
        end
        local recovery = cfg.recover
        if st.kind == "followup" then recovery = cfg.double_cut.recover end
        if st.kind == "special" or st.kind == "echo" then recovery = active_special(st).recover or recovery end
        if st.kind == "second_arrow" then recovery = cfg.ranged.pair.recover end
        st.phase, st.pt = "recover", recovery
        if parried and not cfg.boss then
          st.phase, st.pt = "stagger", math.max(recovery, PARRY_RECOVER)
          vael.play_clip("hit")
        end
        st.t = 0
      end
    elseif st.phase == "charging" then
      local ch = charge_of(st)
      if not ch then
        st.phase, st.pt, st.t = nil, 0, 0
        return true
      end
      local width = ch.width or 1.5
      local hero_reach = charge_reach(width, vael.player_id())
      local track = ch.trample_push and st.charge_track
      local hero_at, obstacle, obstacle_at, obstacle_id
      if track then
        local dx, dy, dz = x - track.x, y - track.y, z - track.z
        if not st.charge_hit then
          hero_at = charge_contact(track.x - track.px, track.y - track.py, track.z - track.pz,
              dx - (px - track.px), dy - (hero_y - track.py), dz - (pz - track.pz), hero_reach)
        end
        obstacle, obstacle_at, obstacle_id = first_charge_obstacle(st, track, dx, dy, dz, width)
        track.x, track.y, track.z, track.px, track.py, track.pz = x, y, z, px, hero_y, pz
      elseif not st.charge_hit and math.abs(hero_y - y) <= REACH_VERTICAL and dist2(px, pz, x, z) <= hero_reach then
        hero_at = 0
      end
      if hero_at and (not obstacle_at or hero_at <= obstacle_at) then
        st.charge_hit = true
        local hurt, parried = rules.damage_player(dmg(st) * (ch.mult or 1.8), x, z)
        if hurt then
          vael.push_player(st.cdx * 2.5, st.cdz * 2.5)
          vael.shake(0.3)
        end
        -- A esquiva deixa passar a investida até ao inimigo de trás.
        if hurt or parried then
          st.charge_track, obstacle = nil, nil
        end
      end
      if obstacle then
        rules.damage_enemy(obstacle_id, math.min(dmg(st), math.max(0, obstacle.hp - 1)))
        vael.entity_knockback(obstacle_id, st.cdx, st.cdz, ch.trample_push)
        vael.burst("ground-dust", obstacle.x, obstacle.y + 0.3, obstacle.z, 10)
        vael.sound("hit", { x = obstacle.x, y = obstacle.y, z = obstacle.z })
        vael.play_clip("hit")
        st.phase, st.pt, st.t, st.charge_track = "recover", ch.recover or 1.3, 0, nil
        return true
      end
      if st.pt <= 0 or dist2(st.ax, st.az, x, z) < 0.9 then
        st.charge_track = nil
        -- Fim da corrida: janela de punição LONGA (falhou = fica exposto).
        st.phase, st.pt, st.t = "recover", ch.recover or 1.3, 0
        vael.burst("ground-dust", x, (select(2, vael.position())) + 0.2, z, 6)
      else
        step(st, x, z, st.ax, st.az, ch.speed or 10)
      end
    elseif st.phase == "burrow" then
      -- Enterrado: invisível e intocável; no fim rebenta onde o herói ESTÁ.
      if st.pt <= 0 then
        local b = st.ph and st.ph.burrow
        if not b then
          st.burrowed = false
          vael.set_visible(true)
          st.phase = nil
          return true
        end
        local _, y = vael.position()
        st.ax, st.az = px, pz
        warp(px, y, pz)
        st.phase, st.pt, st.kind, st.pulse = "windup", b.windup or 1.0, "emerge", 0
        if b.label then vael.toast(b.label) end
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
        vael.damage_number("-" .. d, { color = "#ff8c47", x = x, y = y + 1.6, z = z })
        vael.burst("fire", x, y + 0.8, z, 3)
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
        vael.play_clip("hit")
        vael.ring(x, z, 1.4, "#8cd9ff")
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
      vael.set_visible(true)
    end
    local pause = ph.pause or 1.4
    st.phase, st.pt, st.kind, st.t = "stagger", pause, nil, 0
    st.bt, st.blink_t, st.ct = 0, 0, 0
    entry.invuln_until = vael.time() + pause
    vael.play_clip("roar")
    vael.sound("roar")
    vael.shake(0.22)
    vael.ring(x, z, 4.0, ph.color or "#ff3b1f")
    vael.burst("magic", x, y + 1.2, z, 12)
    if ph.toast then vael.toast(ph.toast) end
    if ph.summon then summon(ph.summon, x, z) end
  end

  function self.update(dt)
    local st = vael.state() -- POR ENTIDADE (no top-level partilhava entre instâncias)
    local id = vael.self_id()
    local g = vael.game()
    -- LOD de comportamento: longe do herói (e sem aggro) a criatura só mantém
    -- o registo fresco. A máquina inteira custa dezenas de leituras do
    -- `vael.game()` partilhado por chamada; com ~250 criaturas no mundo era
    -- ~3 ms/frame (medido 2026-11-16) para bichos que ninguém vê de perto.
    -- Só depois de uma passagem completa nesta sessão (`st.far_session`,
    -- escrito abaixo com o registo garantido); o passeio pára até o herói
    -- voltar a entrar no raio.
    if st.far_session and not st.dead and not cfg.boss
        and (st.aggro_until or 0) < vael.time() then
      local has_p, fpx, _, fpz = vael.player_position()
      if has_p then
        local fx, _, fz = vael.position()
        if dist2(fx, fz, fpx, fpz) > FAR_IDLE then
          local reg = g.enemies
          local e = id and reg and reg[id] or nil
          if e and e.session == st.far_session then
            e.t = vael.time()
            return
          end
        end
      end
    end
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
        vael.entity_despawn(id)
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
        vael.toast(cfg.awake_toast)
      end
    end
    if cfg.on_first and not st.first_done then
      st.first_done = true
      cfg.on_first()
    end
    local has, px, hero_y, pz = vael.player_position()
    -- O registo vivo não pode envelhecer durante uma pausa (menu, cinemática,
    -- LOD): `M.hostiles` só lista entradas com `t` fresco e a recriação
    -- devolvia HP cheio a um inimigo já ferido.
    local function touch_registry()
      if not id then return end
      local enemies = vael.game().enemies or {}
      local e = enemies[id]
      if e then
        e.t = vael.time()
        e.cue = nil
      end
    end
    if not has then
      touch_registry()
      return
    end
    -- Cinemática a tocar: o mundo suspende a respiração — ninguém ataca nem
    -- persegue; quem estava a armar um golpe larga-o e encara o herói.
    if vael.cinematic_playing() then
      -- Só um windup interrompido deixa a cadência "carregada" (st.t já
      -- passou do cooldown); recuperá-la evita o golpe instantâneo ao
      -- voltar da cena.
      if st.phase == "windup" then st.t = 0 end
      st.phase = nil
      release_attack_token(g, session, id, vael.time(), false)
      vael.face_player()
      touch_registry()
      return
    end
    -- Mapa, diário, loja: o golpe que já estava armado não aterra por baixo.
    if vael.menu_open() then
      if st.phase == "windup" then st.t = 0 end
      st.phase = nil
      release_attack_token(g, session, id, vael.time(), false)
      touch_registry()
      return
    end
    local x, py, z = vael.position()
    -- Herói caído (à espera do respawn): ninguém morde o cadáver — larga o
    -- golpe em curso e volta ao passeio.
    local _, hero_hp = rules.player_hp()
    if hero_hp <= 0 then
      if st.phase == "windup" then st.t = 0 end
      st.phase, st.pt, st.state, st.aggro_until = nil, 0, "wander", 0
      st.light_inv_x, st.light_inv_z, st.light_inv_until = nil, nil, nil
      release_attack_token(g, session, id, vael.time(), false)
      px, pz = x + 1e4, z + 1e4
    end

    -- Aggro-chain (`on_player_attack` do nativo): a engine deixou de emitir
    -- `AttackAlert`; o `game/combat.lua`/`game/bomb-lua.lua` publicam o ponto
    -- do golpe numa tabela do `vael.game()` e os aliados a 15 m dele passam
    -- a perseguir durante 10 s (o ataque continua a medir a distância real).
    local alert = g.attack_alert
    local aseq = type(alert) == "table" and alert.seq or nil
    -- O `seq` recomeça em 1 quando o `vael.game()` é limpo (load): o `t`
    -- distingue um alerta novo de um `seq` repetido na linha temporal nova.
    if aseq and (aseq ~= st.alert_seq or alert.t ~= st.alert_t) then
      st.alert_seq, st.alert_t = aseq, alert.t
      if alert.t and (vael.time() - alert.t) <= 0.25
          and dist2(x, z, alert.x or 1e9, alert.z or 1e9) <= ALERT_RADIUS then
        st.aggro_until = vael.time() + 10
      end
    end

    -- Chamado de grupo: quando um aliado avista o herói (ou a lanterna),
    -- companheiros próximos (≤ 9 m) ouvem o alarme e entram juntos no combate.
    local cdir_peek = g.combat_dir
    if type(cdir_peek) == "table" and cdir_peek.session == session and hero_hp > 0 then
      local sseq = cdir_peek.shout_seq
      if sseq and (sseq ~= st.shout_seq or cdir_peek.shout_t ~= st.shout_t) then
        st.shout_seq, st.shout_t = sseq, cdir_peek.shout_t
        if cdir_peek.shout_id ~= id and cdir_peek.shout_t
            and (vael.time() - cdir_peek.shout_t) <= 0.35
            and dist2(x, z, cdir_peek.shout_x or 1e9, cdir_peek.shout_z or 1e9) <= SQUAD_SHOUT_RADIUS then
          st.aggro_until = math.max(st.aggro_until or 0, vael.time() + 8)
        end
      end
    end

    g.enemies = g.enemies or {}
    local now = vael.time()
    local entry = id and g.enemies[id] or nil
    -- Save/restart deixa registos de OUTRA sessão. Uma pausa longa da MESMA
    -- sessão (menu, LOD) conserva o HP em vez de recriar a criatura cheia.
    -- Morto à espera do despawn: o abate já tirou a entrada do registo e o
    -- `entity_despawn` só chega no fim do frame. Recriar aqui ressuscitava
    -- a criatura com vida cheia (e o registo ficava com um fantasma depois
    -- do despawn — a mira suave e o portão do chefe contavam-no).
    if entry == nil and st.dead and st.dead_session == session then
      return
    end
    local stale = entry ~= nil
      and (entry.t == nil or entry.t > now + 1.0 or entry.session ~= session)
    if entry == nil or stale then
      entry = {
        kind = cfg.kind, hp = cfg.hp, max = cfg.hp, boss = cfg.boss,
        x = x, y = py, z = z, t = now, session = session,
      }
      -- Registo recriado = criatura NOVA nesta linha temporal (load, reset do
      -- registo pelas regras, spawn). O `vael.state()` sobrevive à recriação,
      -- por isso os transientes do combate anterior têm de cair aqui: um
      -- `dead` antigo tornava a criatura imortal (hp 0 sem prémio/despawn) e
      -- um chefe enraivecido voltava com vida cheia E a fúria ligada.
      st.dead, st.phase, st.pt, st.kind = nil, nil, 0, nil
      st.t, st.pulse, st.swings = 0, 0, nil
      st.volleys, st.counter_left = nil, nil
      st.enraged, st.poise_dmg = nil, 0
      st.aggro_until, st.state, st.pstate = 0, nil, nil
      st.rt, st.flee_t, st.stuck, st.last_td = nil, 0, 0, nil
      st.flee_d = nil
      -- Fases, enterro e timers dos arquétipos também são desta luta.
      st.phase_i, st.ph, st.bt, st.blink_t, st.ct = nil, nil, 0, 0, nil
      st.burn_t, st.slow, st.charge_hit = 0, false, nil
      st.wait_t, st.retaliate_until = 0, 0
      st.prowl_dir, st.prowl_timer, st.prowl_moving, st.prowl_flips = nil, nil, nil, 0
      st.light_inv_x, st.light_inv_z, st.light_inv_until, st.light_inv_pause = nil, nil, nil, 0
      st.shout_seq, st.shout_t = nil, nil
      if st.burrowed then
        st.burrowed = false
        vael.set_visible(true)
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
      if id then
        g.enemies[id] = entry
        -- O registo é PARTILHADO entre VMs (o `vael.game()` copia a
        -- tabela para o store): o resto deste frame escreve no proxy, não
        -- na cópia local.
        entry = g.enemies[id] or entry
      end
    end
    -- Registo garantido nesta sessão: o atalho "longe" acima pode usá-lo.
    st.far_session = session
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
        vael.set_scale(ELITE_SCALE)
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
    entry.t = vael.time()
    local exposed = st.phase == "recover" or st.phase == "stagger"
    local cue
    if exposed then
      cue = "opening"
    elseif st.phase == "windup" then
      if st.kind == "special" or st.kind == "echo" then
        local sp = active_special(st)
        cue = sp and (sp.cue or "leave_mark")
        exposed = sp and sp.opens_guard
      elseif st.kind == "paired_shot" or st.kind == "second_arrow" then
        cue = "close_archer"
      elseif st.kind == "cast" or st.kind == "emerge" then
        cue = "leave_mark"
      elseif st.kind == "charge" then
        cue = st.lane_blocked and "bait_charge" or "sidestep"
      elseif st.kind == "double" or st.kind == "followup" then
        cue = "double_cut"
      end
    end
    if entry.cue ~= cue then entry.cue = cue end
    entry.shield = cfg.shield
    entry.guard = cfg.shield ~= nil and not exposed
    entry.invuln = st.burrowed == true or (entry.invuln_until or 0) > entry.t
    entry.ranged = (cfg.ranged ~= nil or cfg.caster ~= nil)
    entry.shielded = cfg.shield ~= nil
    entry.attacking = st.phase == "windup" or st.phase == "charging"
    -- Sombras da noite: ao amanhecer desfazem-se (sem prémio nem cadáver).
    if cfg.fades_at_dawn and not st.dead and not clock.is_night() then
      release_attack_token(g, session, id, now, false)
      vael.burst("magic", x, py + 1.0, z, 8)
      g.enemies[id or 0] = nil
      vael.despawn_self()
      return
    end
    if entry.hp <= 0 then
      if not st.dead then
        release_attack_token(g, session, id, now, st.phase == "windup" or st.phase == "charging")
        entry.attacking = false
        entry.chasing = false
        st.dead = true
        st.dead_session = session
        if cfg.boss then
          g["boss_dead." .. cfg.kind] = true
        end
        st.phase, st.state = nil, "dead"
        vael.play_clip("death", { lock = true })
        vael.sound("enemy_death")
        if id and vael.corpse then
          vael.corpse(id)
        end
        rules.award_kill(cfg.kind, id)
        local loot = vael.load("game/loot.lua")
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
          local hx, hz = vael.home()
          g.respawns = type(g.respawns) == "table" and g.respawns or {}
          if #g.respawns < 96 then
            g.respawns[#g.respawns + 1] = {
              proto = cfg.proto, x = hx or x, z = hz or z,
              at = vael.time() + RESPAWN_SECS,
            }
          end
        end
        if id then
          local dead = entry
          vael.after(2.6, function()
            local gg = vael.game()
            -- `vael.after` é world-scoped e os bits da entidade são
            -- REUTILIZADOS: com uma troca de cena (ou um load) pelo meio, o
            -- callback podia matar a criatura nova que herdou o id e limpar
            -- o registo dela. Só age se o registo ainda for ESTE abate.
            if not gg.enemies or gg.enemies[id] ~= dead then
              return
            end
            gg.enemies[id] = nil
            vael.entity_despawn(id)
          end)
        end
      end
      return
    end
    local was_attacking = st.phase == "windup" or st.phase == "charging"
    tick_status(st, entry, dt, x, py, z)
    local shield_chip = entry.shield_chip or 0
    if st.counter_left then
      st.counter_left -= dt
      if st.counter_left <= 0 then st.counter_left = nil end
    end
    if shield_chip > 0 then
      entry.shield_chip = nil
      if cfg.special and cfg.special.after_block then st.counter_left = cfg.special.after_block end
    end
    if not promoted then
      react_to_damage(st, entry.hp, entry.max, shield_chip)
      check_enrage(st, entry.hp, entry.max)
      check_phases(st, entry, x, py, z)
    end
    if st.phase == "stagger" then
      entry.attacking = false
      release_attack_token(g, session, id, now, was_attacking)
    end

    -- Corpo físico (bevy_ext_puppet): a cambalear, caído ou a levantar-se,
    -- quem manda é o corpo — a criatura não anda nem ataca até se recompor.
    -- Se caiu durante um windup/investida, interrompe-os fisicamente.
    if id and vael.vael and vael.vael.puppet_busy and vael.vael.puppet_busy(id) then
      local pstate = vael.vael.puppet_state and vael.vael.puppet_state(id)
      if (pstate == "falling" or pstate == "down")
        and (st.phase == "windup" or st.phase == "charging")
      then
        st.phase, st.pt, st.kind, st.t, st.charge_track = nil, 0, nil, 0, nil
        entry.attacking = false
        release_attack_token(g, session, id, now, true)
      elseif pstate == "falling" or pstate == "down" then
        release_attack_token(g, session, id, now, false)
      end
      return
    end

    -- Nominais + taxa de viragem do rig (1.ª vez só; desliga auto-calibração).
    if cfg.turn_rate and not st.loco then
      st.loco = true
      vael.set_locomotion(cfg.wander, cfg.chase, cfg.turn_rate)
    end

    local phase_was_attacking = st.phase == "windup" or st.phase == "charging"
    if tick_phase(st, dt, px, hero_y, pz, x, py, z) then
      local now_attacking = st.phase == "windup" or st.phase == "charging"
      entry.attacking = now_attacking
      if phase_was_attacking and not now_attacking then
        -- Terminou golpe/investida (entrou em recover/stagger): cede a vez.
        release_attack_token(g, session, id, now, true)
      elseif now_attacking and id then
        local dir = combat_dir(g, session)
        if dir.attacker_id == id then
          dir.attacker_until = now + RESERVE_TIMEOUT
        end
      end
      -- `ok/hp/max` eram restos do antigo `vael.entity_hp()`: `ok` era nil
      -- e a barra de chefe nunca era publicada.
      if st.state == "chase" then boss_bar(st, entry.hp, entry.max) end
      return
    end
    entry.attacking = false

    local dist = dist2(px, pz, x, z)
    -- Espaço pessoal: o `move_towards` não conhece a cápsula do herói e o
    -- lunge do golpe encurta a distância — um lobo acabava DENTRO do herói
    -- (0,1 m, malhas sobrepostas; visto 2026-11-16). Abaixo do mínimo a
    -- criatura recua para fora antes de decidir o resto do frame.
    local min_gap = cfg.min_gap or MIN_GAP
    if dist < min_gap and not st.burrowed then
      local ax, az
      if dist > 1e-3 then
        ax, az = (x - px) / dist, (z - pz) / dist
      else
        ax, az = -(st.hx or 0), -(st.hz or 1)
      end
      vael.move_towards(x + ax * 2.0, z + az * 2.0, BACKOFF_SPEED)
    end
    -- A janela de aggro força só a FSM a chase (o golpe mede a distância REAL).
    local forced_aggro = (st.aggro_until or 0) > now
    local fsm_dist = forced_aggro and 0 or dist
    -- Visão depende de noite, lanterna do herói (apagada não atrai!), agachar e chuva.
    local aggro_sight, deaggro_sight, lit = sight_multipliers()
    local eff_aggro = cfg.aggro * aggro_sight
    local eff_deaggro = cfg.deaggro * deaggro_sight
    st.state = vael.next_state(st.state or "wander", fsm_dist, eff_aggro, eff_deaggro)

    if st.state ~= st.pstate or entry.chasing == nil then
      entry.chasing = st.state == "chase"
    end

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
      st.wait_t = 0
      st.light_inv_x, st.light_inv_z, st.light_inv_until, st.light_inv_pause = nil, nil, nil, 0
      -- Avistamento direto acorda aliados próximos (≤ 9 m) para coordenar em grupo.
      if not forced_aggro and hero_hp > 0 then
        local dir = combat_dir(g, session)
        dir.shout_seq = (dir.shout_seq or 0) + 1
        dir.shout_t = now
        dir.shout_id = id
        dir.shout_x = x
        dir.shout_z = z
      end
      if cfg.intro then
        local key = "cine_" .. cfg.intro
        if not g[key] then
          g[key] = true
          vael.cinematic(cfg.intro, vael.self_id())
        end
      end
      if cfg.on_chase then cfg.on_chase() end
      if st.elite then
        -- Elite a acordar: aura dourada — o jogador sabe que este é diferente.
        local _, y = vael.position()
        vael.ring(x, z, 2.2, ELITE_COLOR)
        vael.burst("sparkle", x, y + 1.2, z, 6)
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
          vael.set_visible(false)
          vael.burst("sand-dust", x, py + 0.3, z, 14)
          vael.sound("roar")
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
          vael.burst("magic", x, py + 1.0, z, 12)
          warp(px + math.cos(a) * r, py, pz + math.sin(a) * r)
          vael.burst("magic", px + math.cos(a) * r, py + 1.0, pz + math.sin(a) * r, 12)
          st.t = 0
          return
        end
      end

      local rg = cfg.ranged or cfg.caster
      local dir_now = combat_dir(g, session)
      local cur_holder = active_token_holder(g, dir_now, session, now)
      local total_chasing, melee_group, allies, front_ang = gather_group(
        g, id, session, now, px, pz, x, z, rg ~= nil, cur_holder
      )
      local in_group = total_chasing >= 2

      -- Investida: só na banda de distância certa (nem colado, nem longe).
      -- Em grupo também respeita o token para não atropelar durante outro golpe.
      local ch = charge_of(st)
      local charge_ready = false
      if ch and dist >= (ch.min or 5) and dist <= (ch.max or 12) then
        local period = ch.cooldown or 6
        st.ct = math.min(period, (st.ct or (period - (ch.opening_delay or period * 0.5))) + dt)
        if st.ct >= period then
          charge_ready = true
          if try_claim_attack_token(g, session, id, st, dist, total_chasing, now, true) then
            st.ct = 0
            if begin_charge(st, ch, px, pz, x, z) then
              entry.attacking = true
              return
            end
          end
        end
      end

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
        -- entre `min` e `range` dispara (coordenado com o grupo).
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
            release_attack_token(g, session, id, now, false)
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
          local cd_r = rg.cooldown or 2.4
          st.rt = math.min(cd_r, (st.rt or cd_r * 0.5) + dt)
          -- Se um aliado bloqueia a linha de tiro, abre lateralmente para limpar o ângulo.
          local sidestep = in_group and shot_lane_blocked(x, z, px, pz, allies) or 0
          if sidestep ~= 0 then
            local dx, dz = px - x, pz - z
            local len = math.max(1e-3, math.sqrt(dx * dx + dz * dz))
            local rx, rz = -dz / len, dx / len
            step(st, x, z, x + rx * sidestep * 2.2, z + rz * sidestep * 2.2, chase_speed(st) * 0.62)
            st.wait_t = (st.wait_t or 0) + dt
            return
          end
          face_hero(st, px, pz, x, z)
          if st.rt >= cd_r then
            if try_claim_attack_token(g, session, id, st, dist, total_chasing, now, true) then
              st.rt = 0
              begin_shot(st, px, pz)
              entry.attacking = true
            else
              st.wait_t = (st.wait_t or 0) + dt
            end
          else
            st.wait_t = (st.wait_t or 0) + dt
          end
          return
        end
      end

      -- Coordenação corpo a corpo em grupo: só quem tem o token fecha para
      -- golpear; os outros abrem cerco no anel tático e aguardam a sua vez.
      local strike_range = cfg.range + REACH_SLACK * 0.5
      local has_token = not in_group
        or try_claim_attack_token(g, session, id, st, dist, total_chasing, now, dist <= strike_range or charge_ready)
      if not has_token then
        st.wait_t = (st.wait_t or 0) + dt
        -- Carrega a cadência enquanto espera no anel para atacar rápido assim
        -- que receber a vez do grupo.
        st.t = math.min(cooldown(st), (st.t or cooldown(st) * 0.55) + dt)
        hold_combat_ring(st, dt, px, pz, x, z, dist, melee_group, allies, ch, front_ang)
        return
      end

      -- Metade da folga de alcance: as cápsulas (herói + criatura) param o
      -- passo a ~1,4 m e um alcance de 1,4–1,6 m ficava na fronteira — a
      -- criatura colava ao herói sem nunca armar o golpe. A resolução já
      -- aceita `range + REACH_SLACK`, por isso o golpe ainda aterra.
      if dist > strike_range then
        local sx, sz = 0, 0
        if in_group then sx, sz = ally_separation(x, z, allies) end
        step(st, x, z, px + sx * 0.45, pz + sz * 0.45, chase_speed(st))
        -- Ao chegar ao alcance, o 1.º golpe sai a meio da cadência (não
        -- depois de uma cadência inteira parada a olhar). Só semeia a
        -- cadência quando ela está VAZIA: reescrever 0.55 todos os frames
        -- deixava o jogador a arrastar o golpe para sempre ao oscilar no
        -- limite do alcance (a carga acumulada era deitada fora).
        if (st.t or 0) <= 0 then
          st.t = cooldown(st) * 0.55
        end
      else
        st.t = (st.t or 0) + dt
        local contact = math.max(min_gap, cfg.range * 0.7)
        if st.t >= cooldown(st) then
          st.t = 0
          face_hero(st, px, pz, x, z)
          begin_attack(st, px, pz)
          entry.attacking = true
        elseif dist > contact then
          -- Fecha o último passo enquanto prepara o ataque. A cadência
          -- continua mesmo se as cápsulas impedirem chegar mais perto.
          local gap = dist - contact
          step(st, x, z, x + (px - x) * gap / dist, z + (pz - z) * gap / dist, chase_speed(st))
        else
          face_hero(st, px, pz, x, z)
        end
      end
    else
      release_attack_token(g, session, id, now, false)
      st.wait_t = 0
      -- À noite com lanterna acesa, inimigos no halo exterior notam o clarão e
      -- investigam o último ponto iluminado. Com luz apagada, não são atraídos.
      if not cfg.boss and hero_hp > 0 then
        if clock.is_night() and lit == true and dist <= eff_aggro * LIGHT_INVESTIGATE_MULT then
          st.light_inv_x, st.light_inv_z = px, pz
          st.light_inv_until = now + 4.5
          st.light_inv_pause = 0
        elseif st.light_inv_until and now > st.light_inv_until then
          st.light_inv_x, st.light_inv_z, st.light_inv_until, st.light_inv_pause = nil, nil, nil, 0
        end
        if st.light_inv_x and st.light_inv_z then
          local idist = dist2(st.light_inv_x, st.light_inv_z, x, z)
          if idist > 1.3 then
            step(st, x, z, st.light_inv_x, st.light_inv_z, cfg.wander * 1.35)
            return
          else
            st.light_inv_pause = (st.light_inv_pause or 0) + dt
            if lit == true then
              face_hero(st, px, pz, x, z)
            end
            if st.light_inv_pause >= 0.9 then
              st.light_inv_x, st.light_inv_z, st.light_inv_until, st.light_inv_pause = nil, nil, nil, 0
            else
              return
            end
          end
        end
      end
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
