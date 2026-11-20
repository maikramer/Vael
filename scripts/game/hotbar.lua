local rules = vael.load("lib/rules.lua")
local actions = vael.load("lib/actions.lua")
-- game/hotbar.lua: a HOTBAR é do jogo (Lua). Poção [1], antídoto [2] e
-- poção de musgo [3].

local POTION_HEAL = 50
local COOLDOWN = 1.0

-- Espírito/cura do equipamento: multiplica toda a cura de consumíveis.
local function heal_power()
  local value = rules.stat("heal_power")
  if not value or value <= 0 then
    return 1.0
  end
  return value
end

-- A hotbar nativa (que gastava a poção no mesmo [1]) saiu da engine; o
-- `vael.own_system("hotbar")` era um no-op que só avisava no arranque.

-- Ctrl/Alt+dígito são atalhos (o `<PostFxDebugToggle>` vive em Ctrl+1..6).
local function modifier_held()
  return vael.input.down("lctrl") or vael.input.down("rctrl")
    or vael.input.down("lalt") or vael.input.down("ralt")
end

-- O host da hotbar vive na origem: posiciona o SFX no herói.
local function sfx(name)
  local has, x, y, z = vael.player_position()
  if has then
    vael.sound(name, { x = x, y = y, z = z })
  else
    vael.sound(name)
  end
end

function on_update(dt)
  local st = vael.state()
  -- Novo Jogo / save de outra partida: as recargas não vêm do jogo antigo.
  local game = vael.game()
  local sid = tonumber(game.session_id) or 0
  -- Um LOAD na MESMA cena mantém o `session_id` e o relógio da engine é o
  -- uptime (NUNCA recua — o teste `st.clock` abaixo nunca disparava e a
  -- recarga da linha temporal abandonada seguia para o jogo carregado). O
  -- sinal certo, o mesmo de rules/abilities/bomb-lua, é o tempo de JOGO a
  -- recuar (`playtime_secs`).
  local playtime = tonumber(game.playtime_secs) or 0
  local loaded = st.playtime ~= nil and playtime < st.playtime - 1e-3
  if st.session ~= sid or loaded then
    st.session = sid
    st.cd = 0
  end
  st.playtime = playtime
  st.cd = math.max(0, (st.cd or 0) - dt)
  -- Título/menu aberto: 1/2 são da UI e o herói está parado atrás do véu.
  -- Sem isto, [1] gastava uma poção do jogo anterior por cima do título.
  if (vael.scene_name and vael.scene_name() == "menu")
      or (vael.ui and vael.ui.is_open and vael.ui.is_open("title")) then
    return
  end
  -- O profiler é `profiler-win`: `is_open("profiler")` era sempre falso e a
  -- poção saía com o painel aberto. Mapa e viagem entram no mesmo portão.
  if vael.menu_open() or st.cd > 0 or modifier_held() then
    return
  end
  -- Balão de diálogo aberto: 1/2 são escolhas do grafo (townsfolk.lua).
  -- Só conta um timestamp PRÓXIMO: `vael.time()` recomeça no load e um
  -- valor gravado no bind antes do reset ficava "no futuro" durante minutos.
  local dialogue_until = tonumber(vael.ui.get("dialogue.until")) or 0
  local left = dialogue_until - vael.time()
  if left > 0 and left <= 2.0 then
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return
  end
  if vael.input.pressed("1") then
    local _, hp, max_hp = rules.player_hp()
    local healed = math.min(POTION_HEAL * heal_power(), math.max(0, (max_hp or 0) - (hp or 0)))
    if rules.item_count("potion") > 0 and healed < 1 then
      -- Vida cheia: a poção fica no cinto e o [2] não fica bloqueado 1 s.
      vael.toast("Vida já está cheia.")
    elseif rules.item_count("potion") > 0 then
      st.cd = COOLDOWN
      rules.vault_take("potion", 1)
      rules.heal_player(healed)
      actions.record("usar", "item", "potion")
      -- O número nasce SOBRE O HERÓI (sem x/y/z saía sobre o controller).
      local _, px, py, pz = vael.player_position()
      vael.damage_number("+" .. math.floor(healed) .. " HP", { color = "#7ef29d", x = px, y = py + 2.0, z = pz })
      sfx("heal")
      vael.toast("Poção usada (+" .. math.floor(healed) .. " HP)")
    else
      vael.toast("Sem poções.")
      sfx("error")
    end
  elseif vael.input.pressed("2") then
    if rules.item_count("antidote") > 0 then
      -- `status.venom` é o bind do chip ENVENEADO (texto vazio = sem veneno).
      -- Antes o antídoto era gasto e o toast dizia "neutralizado" mesmo são.
      if not rules.poisoned() then
        vael.toast("Não estás envenenado.")
      else
        st.cd = COOLDOWN
        rules.vault_take("antidote", 1)
        rules.status_clear("venom")
        actions.record("usar", "item", "antidote")
        -- A poção confirma com "heal"; o antídoto calava-se e o jogador não
        -- sabia se o veneno tinha saído.
        sfx("heal")
        vael.toast("Veneno neutralizado.")
      end
    else
      vael.toast("Sem antídotos.")
      sfx("error")
    end
  elseif vael.input.pressed("3") then
    -- Poção de musgo: os números vêm do item (`use` em data/items.json) —
    -- cura um pouco e limpa veneno, queimadura e gelo de uma vez.
    local def = rules.item_def("moss_potion")
    local use = def and def.use or {}
    local _, hp, max_hp = rules.player_hp()
    local healed = math.min((tonumber(use.heal) or 0) * heal_power(), math.max(0, (max_hp or 0) - (hp or 0)))
    local cures = use.cure == true and rules.afflicted()
    if rules.item_count("moss_potion") < 1 then
      vael.toast("Sem poções de musgo.")
      sfx("error")
    elseif healed < 1 and not cures then
      vael.toast("Nada para tratar.")
    else
      st.cd = COOLDOWN
      rules.vault_take("moss_potion", 1)
      if healed >= 1 then
        rules.heal_player(healed)
      end
      if cures then
        rules.status_clear()
      end
      actions.record("usar", "item", "moss_potion")
      local _, px, py, pz = vael.player_position()
      if healed >= 1 then
        vael.damage_number("+" .. math.floor(healed) .. " HP", { color = "#7ef29d", x = px, y = py + 2.0, z = pz })
      end
      sfx("heal")
      vael.toast(cures and "Poção de musgo: estados limpos." or "Poção de musgo usada.")
    end
  end
end
