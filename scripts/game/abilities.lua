local rules = viber.load("lib/rules.lua")
-- game/abilities.lua: as HABILIDADES do herói são do JOGO (Lua), sobre a API
-- genérica da engine (que já não tem handlers nativos para estas teclas):
--
--   [C] esquiva  → `viber.vael.dodge()` (controller Rust): W rola, A/D salto
--                  lateral, S/nada salto para trás; + grant_iframes + clip
--   [E] cura     → heal_player + número flutuante
--   [R] radial   → rules.radial_damage (falloff + knockback) + anel e shake
--   [B] bomba    → vault_take + spawn_prototype do `<Prototype id="bomb-lua">`
--                  (o script da bomba detona: ver game/bomb-lua.lua)
--
-- [J] melee, [L] guarda (precisa do componente nativo `Guarding`) e o
-- diálogo [E] continuam da engine — a posse é POR SISTEMA.
--
-- Host: `world/autoload/player.xml` (`<Entity tag="always-active">` — o
-- controller corre onde o herói andar; sem o tag o LOD de 45 m congelava-o).
-- Autoload para dash/cura/impacto funcionarem também dentro das casas.

-- Esquiva curta (1,2 s): é a resposta principal aos golpes telegrafados. A
-- cura [E] deixou de ser uma recarga grátis: gasta uma carga do CANTIL
-- (`rules.flask_*`), que só volta no descanso; a recarga de 2 s só evita o
-- duplo toque.
local CD_DASH, CD_HEAL, CD_STRIKE = 1.2, 2.0, 8.0
local DASH_DISTANCE = 5.0 -- só o recurso sem `viber.vael.dodge` (push_player)
local DASH_IFRAMES = 0.35
-- Clip de cada esquiva: o rolamento tem clip próprio; os saltos usam o
-- `jump` acelerado para caber na duração do salto.
local DODGE_CLIP = { roll = "roll", side = "jump", back = "jump" }
local HEAL_AMOUNT = 50
local STRIKE_RADIUS, STRIKE_DAMAGE, STRIKE_KNOCKBACK = 4.8, 60, 7
local BOMB_LAUNCH_DISTANCE = 2.6

local PERIOD = { dash = CD_DASH, heal = CD_HEAL, strike = CD_STRIKE }

-- A engine já não tem handlers nativos de [C]/[R]/[B] nem da guarda [L]
-- (`rules.guard` abre a janela de parry e o dano dos inimigos passa todo por
-- `rules.damage_player`): o `viber.own_system("abilities"/"bomb"/"guard")`
-- era um no-op que só avisava no arranque.

-- O dock do HUD lê `ab.<nome>` (fração em recarga) e `ab.<nome>.ready`
-- (class-bind que acende o slot) — os `cd.*` nativos ficam a zero com o
-- sistema reclamado, e sem isto o dock mostrava tudo sempre pronto.
-- (Nomes literais: o teste `test_shipped_worlds_bind_only_to_known_names`
-- lê os `viber.ui.set("…")` dos scripts para validar os binds do HUD.)
local function publish_cooldowns(cd)
  viber.ui.set("ab.dash", cd.dash / PERIOD.dash)
  viber.ui.set("ab.dash.ready", cd.dash <= 0)
  viber.ui.set("ab.heal", cd.heal / PERIOD.heal)
  viber.ui.set("ab.heal.ready", cd.heal <= 0)
  viber.ui.set("ab.strike", cd.strike / PERIOD.strike)
  viber.ui.set("ab.strike.ready", cd.strike <= 0)
end

-- O [E] é de quem estiver à frente: com um prompt [E] no ecrã (NPC,
-- baú, poço…) a tecla é da interação e a cura não sai por baixo.
local function e_is_interaction()
  return viber.ui.get("prompt.key") == "E" and viber.ui.get("prompt.label") ~= ""
end

-- Balão de diálogo/NLU aberto: as teclas de habilidade não são da conversa.
-- A hotbar e o equipamento já leem o mesmo bind `dialogue.until`; sem isto um
-- [C]/[R]/[B] premido a ler o balão disparava por baixo (a bomba era gasta).
-- O bind é um timestamp: um valor de outra sessão (no "futuro") não pode
-- trancar as habilidades.
local function dialogue_open()
  local until_t = tonumber(viber.ui.get("dialogue.until")) or 0
  local left = until_t - viber.time()
  return left > 0 and left <= 2.0
end

function on_update(dt)
  local st = viber.state()
  st.cd = st.cd or { dash = 0, heal = 0, strike = 0 }
  -- Novo Jogo / save de outra partida: recargas limpas.
  -- Um LOAD na MESMA cena troca o `viber.game()` sem mudar `session_id` e o
  -- relógio da engine (uptime) NÃO recua — o sinal é o tempo de jogo a
  -- recuar. Sem isto as recargas da linha temporal abandonada (até 12 s de
  -- cura) seguiam para o jogo carregado.
  local game = viber.game()
  local sid = tonumber(game.session_id) or 0
  local playtime = tonumber(game.playtime_secs) or 0
  local loaded = st.playtime ~= nil and playtime < st.playtime - 1e-3
  if st.session ~= sid or loaded then
    st.session = sid
    st.cd = { dash = 0, heal = 0, strike = 0 }
  end
  st.playtime = playtime
  if game and game.voice_dash_reset then
    st.cd.dash = 0
    game.voice_dash_reset = false
  end
  -- Título aberto ou cena de menu (acampamento atrás do véu): C/E/R/B/L são
  -- da UI. O `viber.menu_open()` não cobre o título — o hotbar e o
  -- equipamento já tinham o portão (o [1] gastava uma poção por cima do
  -- título), mas as habilidades ficaram de fora: o [B] gastava uma bomba do
  -- jogo anterior e o [R] matava criaturas atrás do véu.
  if (viber.scene_name and viber.scene_name() == "menu")
      or (viber.ui and viber.ui.is_open and viber.ui.is_open("title")) then
    publish_cooldowns(st.cd)
    return
  end
  -- Diário, profiler (`profiler-win`, não "profiler"), mapa [M], viagem [G]
  -- e cinemática. `menu_open()` cobre os cinco: o resto do mundo pára
  -- (criaturas, rastilho da bomba, veneno), por isso as recargas também não
  -- podem correr por baixo do modal — abrir o mapa 12 s devolvia a cura.
  local paused = viber.menu_open()
  if not paused then
    for k, v in pairs(st.cd) do
      st.cd[k] = math.max(0, v - dt)
    end
  end
  publish_cooldowns(st.cd)
  if paused then
    return
  end
  -- Balão de conversa (grafo/NLU) aberto: as habilidades ficam com a fala.
  if dialogue_open() then
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return
  end
  local has, px, py, pz = viber.player_position()
  if not has then
    return
  end

  -- [L] guarda/parry: janela curta; o dano que chegar nela é aparado.
  if viber.input.pressed("l") then
    rules.guard()
  end

  -- [C] esquiva. O controller Rust escolhe pelo input do frame: W rola,
  -- A/D dão o salto lateral (sem rodar o corpo), S/nada o salto para trás.
  -- Recusa no ar ou a meio de outra esquiva (devolve nil) — sem gastar a
  -- recarga.
  if viber.input.pressed("c") and st.cd.dash <= 0 then
    local kind, duration
    if viber.vael and viber.vael.dodge then
      kind, duration = viber.vael.dodge()
    else
      -- Recurso: o dash antigo pelo character controller.
      local fx, fz = viber.player_forward()
      viber.push_player(fx * DASH_DISTANCE, fz * DASH_DISTANCE)
      kind, duration = "dash", 0.3
    end
    if kind then
      -- Talento "Passo de vento": `dash_cd` é a variação da recarga (−0,4).
      st.cd.dash = CD_DASH * math.max(0.3, 1 + rules.stat("dash_cd"))
      PERIOD.dash = st.cd.dash
      rules.grant_iframes(DASH_IFRAMES + rules.stat("dash_iframes"))
      -- O melee lê isto: a esquiva cancela o golpe pendente e adia o
      -- próximo até aterrar.
      -- TABELA (não viaja no save, como o `attack_alert`): um escalar com o
      -- relógio de outra sessão trancava o ataque depois de um load.
      local g = viber.game()
      if type(g.hero_dodge) ~= "table" then
        g.hero_dodge = {}
      end
      g.hero_dodge.until_t = viber.time() + (duration or 0.3)
      local clip = DODGE_CLIP[kind]
      local pid = viber.player_id()
      if clip and pid then
        local clip_len = viber.clip_duration and viber.clip_duration(clip) or 0
        local speed = 1.0
        if clip_len and clip_len > 0 and duration and duration > 0 then
          speed = math.max(0.6, math.min(3.0, clip_len / duration))
        end
        viber.play_clip(clip, { speed = speed, id = pid, blend = 0.08 })
      end
      if kind == "roll" or kind == "dash" then
        viber.fov_kick(8)
      end
      viber.sound("dash", { x = px, y = py, z = pz })
      viber.burst("ground-dust", px, py + 0.2, pz, 10)
    end
  end

  -- [E] cura (sem interação em alcance — a fronteira do nativo: o prompt
  -- [E] do HUD é a mesma pergunta "há alguém para falar/usar aqui?").
  -- Com a vida cheia não gasta a recarga; o número mostra o que CUROU (e
  -- nasce sobre o herói — sem x/y/z saía sobre a entidade controller).
  if viber.input.pressed("e") and st.cd.heal <= 0 and not e_is_interaction() then
    local _, hp, max_hp = rules.player_hp()
    -- Espírito aumenta a cura (`Stat.heal_power`, 1.0 = base).
    local heal_power = rules.stat("heal_power")
    if not heal_power or heal_power <= 0 then
      heal_power = 1.0
    end
    local healed = math.min(HEAL_AMOUNT * heal_power, math.max(0, (max_hp or 0) - (hp or 0)))
    if healed < 1 then
      viber.toast("Vida já está cheia.")
    elseif not rules.flask_use() then
      viber.toast("Cantil vazio — descansa numa fogueira.")
      viber.sound("error", { x = px, y = py, z = pz })
    else
      st.cd.heal = CD_HEAL
      rules.heal_player(healed)
      viber.sound("heal", { x = px, y = py, z = pz })
      viber.burst("sparkle", px, py + 1.0, pz, 14)
      viber.damage_number("+" .. math.floor(healed) .. " HP", { color = "#7ef29d", x = px, y = py + 2.0, z = pz })
    end
  end

  -- [R] golpe radial com juice completo (valores do nativo; o dano soma as
  -- passivas de Força como o melee — o nativo lia `stats.bonus_damage`).
  if viber.input.pressed("r") and st.cd.strike <= 0 then
    st.cd.strike = CD_STRIKE
    local bonus = rules.player_stats().bonus_damage or 0
    -- Talentos do Impacto: "Abalo" alarga e reforça; "Brasas" queima.
    local radius = STRIKE_RADIUS + rules.stat("strike_radius")
    local burn = rules.stat("strike_burn")
    local knockback = rules.stat("knockback")
    if not knockback or knockback <= 0 then
      knockback = 1.0
    end
    if burn > 0 then
      for _, h in ipairs(rules.hostiles_near(px, pz, radius)) do
        rules.enemy_status(h.id, "burn", burn)
      end
      viber.burst("fire", px, py + 0.4, pz, 10)
    end
    local hits = rules.radial_damage(px, pz, radius,
      STRIKE_DAMAGE + rules.stat("strike_damage") + bonus, STRIKE_KNOCKBACK * knockback)
    -- Aggro-chain: o slam também acorda os aliados a 15 m do impacto (o
    -- nativo emitia um `AttackAlert` no centro). A engine já não tem
    -- produtor; o `lib/fsm.lua` lê esta tabela do `viber.game()`.
    if (hits or 0) > 0 then
      local g = viber.game()
      local alert = g.attack_alert
      if type(alert) ~= "table" then
        alert = {}
        g.attack_alert = alert
      end
      alert.x, alert.z = px, pz
      alert.t = viber.time()
      alert.seq = (alert.seq or 0) + 1
    end
    viber.burst("sparks", px, py + 1.0, pz, 6)
    viber.ring(px, pz, radius, burn > 0 and "#ff8c47" or nil)
    viber.shake(0.22)
    viber.kick(0, 0.35, 0)
    viber.hit_stop(0.07)
    viber.punch(0.12, 0.04)
    viber.sound("whoosh", { x = px, y = py, z = pz })
    viber.sound("hit", { x = px, y = py, z = pz })
  end

  -- [B] bomba: larga o prototype à frente (detona com fuse — bomb-lua.lua).
  if viber.input.pressed("b") then
    if rules.item_count("bomb") > 0 then
      local fx, fz = viber.player_forward()
      if fx * fx + fz * fz < 1e-4 then
        fx, fz = 0, 1
      end
      local bx = px + fx * BOMB_LAUNCH_DISTANCE
      local bz = pz + fz * BOMB_LAUNCH_DISTANCE
      -- O `<Prototype id="bomb-lua">` só existe onde `world/gameplay.xml`
      -- está incluído. Em vez de adivinhar pela cena (bloqueava os mundos QA
      -- e dependia de o id ser "main"), tenta e DEVOLVE a bomba se o spawn
      -- rebentar ("prototype desconhecido" é erro de script).
      -- O prototype não tem física: assenta-o no chão (o `y` da mão deixava
      -- a bomba a flutuar 1,1 m durante todo o rastilho).
      local gy = viber.ground_below and viber.ground_below(bx, py + 2.0, bz)
      local by = (gy or py) + 0.25
      rules.vault_take("bomb", 1)
      local ok = pcall(viber.spawn_prototype, "bomb-lua", bx, bz, { y = by })
      if ok then
        viber.sound("bomb_drop", { x = px, y = py, z = pz })
      else
        rules.item_add("bomb", 1)
        viber.toast("Aqui dentro não há espaço para bombas.")
        viber.sound("error", { x = px, y = py, z = pz })
      end
    else
      viber.toast("Sem bombas — compra ao mercador.")
      viber.sound("error", { x = px, y = py, z = pz })
    end
  end
end
