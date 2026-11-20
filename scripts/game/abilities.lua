local rules = vael.load("lib/rules.lua")
local feel
do
  local ok, mod = pcall(vael.load, "lib/feel.lua")
  if ok and type(mod) == "table" and type(mod.duck) == "function" then
    feel = mod
  end
end
-- game/abilities.lua: as HABILIDADES do herói são do JOGO (Lua), sobre a API
-- genérica da engine (que já não tem handlers nativos para estas teclas):
--
--   [C] esquiva  → `vael.vael.dodge()` (controller Rust): W rola, A/D salto
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
local DASH_DISTANCE = 5.0 -- só o recurso sem `vael.vael.dodge` (push_player)
local DASH_IFRAMES = 0.35
local DODGE_BUFFER = 0.16
-- Clip de cada esquiva: o rolamento tem clip próprio; os saltos usam o
-- `jump` acelerado para caber na duração do salto.
local DODGE_CLIP = { roll = "roll", side = "jump", back = "jump" }
local HEAL_AMOUNT = 50
local STRIKE_RADIUS, STRIKE_DAMAGE, STRIKE_KNOCKBACK = 4.8, 60, 7
local ECHO_WINDOW, ECHO_RADIUS, ECHO_DAMAGE = 0.75, 2.8, 30
local ECHO_COLOR = "#74e0d0"
local BOMB_LAUNCH_DISTANCE = 2.6

local PERIOD = { dash = CD_DASH, heal = CD_HEAL, strike = CD_STRIKE }

-- A engine já não tem handlers nativos de [C]/[R]/[B] nem da guarda [L]
-- (`rules.guard` abre a janela de parry e o dano dos inimigos passa todo por
-- `rules.damage_player`): o `vael.own_system("abilities"/"bomb"/"guard")`
-- era um no-op que só avisava no arranque.

-- O dock do HUD lê `ab.<nome>` (fração em recarga) e `ab.<nome>.ready`
-- (class-bind que acende o slot) — os `cd.*` nativos ficam a zero com o
-- sistema reclamado, e sem isto o dock mostrava tudo sempre pronto.
-- (Nomes literais: o teste `test_shipped_worlds_bind_only_to_known_names`
-- lê os `vael.ui.set("…")` dos scripts para validar os binds do HUD.)
local function publish_cooldowns(cd)
  vael.ui.set("ab.dash", cd.dash / PERIOD.dash)
  vael.ui.set("ab.dash.ready", cd.dash <= 0)
  vael.ui.set("ab.heal", cd.heal / PERIOD.heal)
  vael.ui.set("ab.heal.ready", cd.heal <= 0)
  vael.ui.set("ab.strike", cd.strike / PERIOD.strike)
  vael.ui.set("ab.strike.ready", cd.strike <= 0)
end

-- O [E] é de quem estiver à frente: com um prompt [E] no ecrã (NPC,
-- baú, poço…) a tecla é da interação e a cura não sai por baixo.
local function e_is_interaction()
  return vael.ui.get("prompt.key") == "E" and vael.ui.get("prompt.label") ~= ""
end

-- Balão de diálogo/NLU aberto: as teclas de habilidade não são da conversa.
-- A hotbar e o equipamento já leem o mesmo bind `dialogue.until`; sem isto um
-- [C]/[R]/[B] premido a ler o balão disparava por baixo (a bomba era gasta).
-- O bind é um timestamp: um valor de outra sessão (no "futuro") não pode
-- trancar as habilidades.
local function dialogue_open()
  local until_t = tonumber(vael.ui.get("dialogue.until")) or 0
  local left = until_t - vael.time()
  return left > 0 and left <= 2.0
end

function on_update(dt)
  local st = vael.state()
  st.cd = st.cd or { dash = 0, heal = 0, strike = 0 }
  -- Novo Jogo / save de outra partida: recargas limpas.
  -- Um LOAD na MESMA cena troca o `vael.game()` sem mudar `session_id` e o
  -- relógio da engine (uptime) NÃO recua — o sinal é o tempo de jogo a
  -- recuar. Sem isto as recargas da linha temporal abandonada (até 12 s de
  -- cura) seguiam para o jogo carregado.
  local game = vael.game()
  local sid = tonumber(game.session_id) or 0
  local playtime = tonumber(game.playtime_secs) or 0
  local loaded = st.playtime ~= nil and playtime < st.playtime - 1e-3
  local scene = vael.scene_name and vael.scene_name()
  -- A pegada vive na tabela de sessão, que o load descarta até na mesma cena.
  local echo = game.counterbeat
  if echo and (st.scene ~= scene or st.session ~= sid or loaded) then
    game.counterbeat, echo = nil, nil
  end
  local buffered = (st.scene == scene) and math.max(0, (st.dodge_buffer or 0) - dt) or 0
  st.scene, st.dodge_buffer = scene, 0
  if st.session ~= sid or loaded then
    buffered = 0
    st.session = sid
    st.cd = { dash = 0, heal = 0, strike = 0 }
  end
  st.playtime = playtime
  if game and game.voice_dash_reset then
    st.cd.dash = 0
    game.voice_dash_reset = false
  end
  -- Título aberto ou cena de menu (acampamento atrás do véu): C/E/R/B/L são
  -- da UI. O `vael.menu_open()` não cobre o título — o hotbar e o
  -- equipamento já tinham o portão (o [1] gastava uma poção por cima do
  -- título), mas as habilidades ficaram de fora: o [B] gastava uma bomba do
  -- jogo anterior e o [R] matava criaturas atrás do véu.
  if scene == "menu"
      or (vael.ui and vael.ui.is_open and vael.ui.is_open("title")) then
    if echo then game.counterbeat = nil end
    publish_cooldowns(st.cd)
    return
  end
  -- Diário, profiler (`profiler-win`, não "profiler"), mapa [M], viagem [G]
  -- e cinemática. `menu_open()` cobre os cinco: o resto do mundo pára
  -- (criaturas, rastilho da bomba, veneno), por isso as recargas também não
  -- podem correr por baixo do modal — abrir o mapa 12 s devolvia a cura.
  local paused = vael.menu_open()
  if not paused then
    for k, v in pairs(st.cd) do
      st.cd[k] = math.max(0, v - dt)
    end
  end
  publish_cooldowns(st.cd)
  if paused then
    if echo then game.counterbeat = nil end
    return
  end
  -- Balão de conversa (grafo/NLU) aberto: as habilidades ficam com a fala.
  if dialogue_open() then
    if echo then game.counterbeat = nil end
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    if echo then game.counterbeat = nil end
    return
  end
  local has, px, py, pz = vael.player_position()
  if not has then
    return
  end

  local now = vael.time()
  if echo then
    if now > echo.release + ECHO_WINDOW or vael.ui.get("boat.grip") == "oars" then
      game.counterbeat, echo = nil, nil
    elseif now >= echo.release and not echo.rang then
      echo.rang = true
      vael.ring(echo.x, echo.z, ECHO_RADIUS, ECHO_COLOR)
    end
  end

  -- [L] guarda/parry: janela curta; o dano que chegar nela é aparado.
  if vael.input.pressed("l") then
    rules.guard()
  end

  -- [C] esquiva. O controller Rust escolhe pelo input do frame: W rola,
  -- A/D dão o salto lateral (sem rodar o corpo), S/nada o salto para trás.
  -- Recusa no ar ou a meio de outra esquiva (devolve nil) — sem gastar a
  -- recarga.
  if vael.ui.get("boat.grip") ~= "oars" then
    st.dodge_buffer = vael.input.pressed("c") and DODGE_BUFFER or buffered
  end
  if st.dodge_buffer > 0 and st.cd.dash <= 0 then
    local kind, duration
    if vael.vael and vael.vael.dodge then
      kind, duration = vael.vael.dodge()
    else
      -- Recurso: o dash antigo pelo character controller.
      local fx, fz = vael.player_forward()
      vael.push_player(fx * DASH_DISTANCE, fz * DASH_DISTANCE)
      kind, duration = "dash", 0.3
    end
    if kind then
      st.dodge_buffer = 0
      -- Talento "Passo de vento": `dash_cd` é a fração da recarga base somada pelos talentos.
      st.cd.dash = CD_DASH * math.max(0.3, 1 + rules.stat("dash_cd"))
      PERIOD.dash = st.cd.dash
      rules.grant_iframes(DASH_IFRAMES + rules.stat("dash_iframes"))
      -- O melee lê isto: a esquiva cancela o golpe pendente e adia o
      -- próximo até aterrar.
      -- TABELA (não viaja no save, como o `attack_alert`): um escalar com o
      -- relógio de outra sessão trancava o ataque depois de um load.
      local g = vael.game()
      if type(g.hero_dodge) ~= "table" then
        g.hero_dodge = {}
      end
      g.hero_dodge.until_t = now + (duration or 0.3)
      echo = nil
      if st.cd.strike <= 0 and g["skill.strength1"] == true and g["skill.agility1"] == true then
        g.counterbeat = { x = px, y = py, z = pz, release = g.hero_dodge.until_t }
        vael.burst("sparkle", px, py + 0.2, pz, 6)
      else
        g.counterbeat = nil
      end
      local clip = DODGE_CLIP[kind]
      local pid = vael.player_id()
      if clip and pid then
        local clip_len = vael.clip_duration and vael.clip_duration(clip) or 0
        local speed = 1.0
        if clip_len and clip_len > 0 and duration and duration > 0 then
          speed = math.max(0.6, math.min(3.0, clip_len / duration))
        end
        vael.play_clip(clip, { speed = speed, id = pid, blend = 0.08 })
      end
      if kind == "roll" or kind == "dash" then
        vael.fov_kick(8)
      end
      vael.sound("dash", { x = px, y = py, z = pz })
      vael.burst("ground-dust", px, py + 0.2, pz, 10)
    end
  end

  -- [E] cura (sem interação em alcance — a fronteira do nativo: o prompt
  -- [E] do HUD é a mesma pergunta "há alguém para falar/usar aqui?").
  -- Com a vida cheia não gasta a recarga; o número mostra o que CUROU (e
  -- nasce sobre o herói — sem x/y/z saía sobre a entidade controller).
  if vael.input.pressed("e") and st.cd.heal <= 0 and not e_is_interaction() then
    local _, hp, max_hp = rules.player_hp()
    -- Espírito aumenta a cura (`Stat.heal_power`, 1.0 = base).
    local heal_power = rules.stat("heal_power")
    if not heal_power or heal_power <= 0 then
      heal_power = 1.0
    end
    local healed = math.min(HEAL_AMOUNT * heal_power, math.max(0, (max_hp or 0) - (hp or 0)))
    if healed < 1 then
      vael.toast("Vida já está cheia.")
    elseif not rules.flask_use() then
      vael.toast("Cantil vazio — descansa numa fogueira.")
      vael.sound("error", { x = px, y = py, z = pz })
    else
      st.cd.heal = CD_HEAL
      rules.heal_player(healed)
      vael.sound("heal", { x = px, y = py, z = pz })
      vael.burst("sparkle", px, py + 1.0, pz, 14)
      vael.damage_number("+" .. math.floor(healed) .. " HP", { color = "#7ef29d", x = px, y = py + 2.0, z = pz })
    end
  end

  -- [R] golpe radial com juice completo (valores do nativo; o dano soma as
  -- passivas de Força como o melee — o nativo lia `stats.bonus_damage`).
  if vael.input.pressed("r") and st.cd.strike <= 0 then
    if vael.vael.spend_attack_stamina and not vael.vael.spend_attack_stamina() then
      vael.toast("Sem fôlego para atacar.")
      return
    end
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
      vael.burst("fire", px, py + 0.4, pz, 10)
    end
    -- O centro principal ganha na sobreposição, mesmo quando a onda do eco
    -- apanha um alvo na margem do Impacto.
    local struck = echo and echo.rang and {} or nil
    local hits, survivor = rules.radial_damage(px, pz, radius,
      STRIKE_DAMAGE + rules.stat("strike_damage") + bonus, STRIKE_KNOCKBACK * knockback, py, burn, struck)
    if struck then
      local count, at_origin = rules.radial_damage(echo.x, echo.z, ECHO_RADIUS, ECHO_DAMAGE, 0, echo.y, 0, struck)
      hits = hits + count
      survivor = survivor or at_origin
      vael.burst("sparkle", echo.x, echo.y + 0.4, echo.z, 18)
      vael.sound("notification", { x = echo.x, y = echo.y, z = echo.z })
      if count > 0 and not game["discovery.counterbeat"] then
        game["discovery.counterbeat"] = true
        vael.toast("Contratempo descoberto. A tua pegada respondeu ao Impacto.")
      end
    end
    game.counterbeat = nil
    if survivor then vael.game().strike_focus = { id = survivor, t = vael.time() } end
    -- Aggro-chain: o slam também acorda os aliados a 15 m do impacto (o
    -- nativo emitia um `AttackAlert` no centro). A engine já não tem
    -- produtor; o `lib/fsm.lua` lê esta tabela do `vael.game()`.
    if (hits or 0) > 0 then
      local g = vael.game()
      local alert = g.attack_alert
      if type(alert) ~= "table" then
        alert = {}
        g.attack_alert = alert
        alert = g.attack_alert or alert -- tabela partilhada: escreve no proxy
      end
      alert.x, alert.z = px, pz
      alert.t = vael.time()
      alert.seq = (alert.seq or 0) + 1
    end
    vael.burst("sparks", px, py + 1.0, pz, 6)
    vael.ring(px, pz, radius, burn > 0 and "#ff8c47" or nil)
    vael.shake(0.22)
    vael.kick(0, 0.35, 0)
    vael.punch(0.12, 0.04)
    vael.sound("whoosh", { x = px, y = py, z = pz })
    if hits > 0 then
      vael.hit_stop(0.07)
      vael.sound("hit", { x = px, y = py, z = pz })
      if feel then
        feel.duck("music", 0.62, 0.22)
      end
    end
  end

  -- [B] bomba: larga o prototype à frente (detona com fuse — bomb-lua.lua).
  if vael.input.pressed("b") then
    if rules.item_count("bomb") > 0 then
      local fx, fz = vael.player_forward()
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
      local gy = vael.ground_below and vael.ground_below(bx, py + 2.0, bz)
      local by = (gy or py) + 0.25
      rules.vault_take("bomb", 1)
      local ok = pcall(vael.spawn_prototype, "bomb-lua", bx, bz, { y = by })
      if ok then
        vael.sound("bomb_drop", { x = px, y = py, z = pz })
      else
        rules.item_add("bomb", 1)
        vael.toast("Aqui dentro não há espaço para bombas.")
        vael.sound("error", { x = px, y = py, z = pz })
      end
    else
      vael.toast("Sem bombas — compra ao mercador.")
      vael.sound("error", { x = px, y = py, z = pz })
    end
  end
end
