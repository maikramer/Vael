local rules = viber.load("lib/rules.lua")
-- game/hotbar.lua: a HOTBAR é do jogo (Lua). Poção e antídoto nos slots 1/2.

local POTION_HEAL = 50
local COOLDOWN = 1.0

-- A hotbar nativa gastava a poção no mesmo [1] que este script.
viber.own_system("hotbar")

-- Ctrl/Alt+dígito são atalhos (o `<PostFxDebugToggle>` vive em Ctrl+1..6).
local function modifier_held()
  return viber.input.down("lctrl") or viber.input.down("rctrl")
    or viber.input.down("lalt") or viber.input.down("ralt")
end

-- O host da hotbar vive na origem: posiciona o SFX no herói.
local function sfx(name)
  local has, x, y, z = viber.player_position()
  if has then
    viber.sound(name, { x = x, y = y, z = z })
  else
    viber.sound(name)
  end
end

function on_update(dt)
  local st = viber.state()
  -- Novo Jogo / save de outra partida: as recargas não vêm do jogo antigo.
  local sid = tonumber(viber.game().session_id) or 0
  if st.session ~= sid then
    st.session = sid
    st.cd = 0
  end
  -- Relógio para trás = load: as recargas da sessão anterior não valem no
  -- jogo carregado (senão a poção ficava 1 s presa, ou mais com o dash).
  local now = viber.time()
  if st.clock and now < st.clock then
    st.cd = 0
  end
  st.clock = now
  st.cd = math.max(0, (st.cd or 0) - dt)
  -- O profiler é `profiler-win`: `is_open("profiler")` era sempre falso e a
  -- poção saía com o painel aberto. Mapa e viagem entram no mesmo portão.
  if viber.menu_open() or st.cd > 0 or modifier_held() then
    return
  end
  -- Balão de diálogo aberto: 1/2 são escolhas do grafo (townsfolk.lua).
  -- Só conta um timestamp PRÓXIMO: `viber.time()` recomeça no load e um
  -- valor gravado no bind antes do reset ficava "no futuro" durante minutos.
  local dialogue_until = tonumber(viber.ui.get("dialogue.until")) or 0
  local left = dialogue_until - viber.time()
  if left > 0 and left <= 2.0 then
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return
  end
  if viber.input.pressed("1") then
    local _, hp, max_hp = rules.player_hp()
    local healed = math.min(POTION_HEAL, math.max(0, (max_hp or 0) - (hp or 0)))
    if rules.item_count("potion") > 0 and healed < 1 then
      -- Vida cheia: a poção fica no cinto e o [2] não fica bloqueado 1 s.
      viber.toast("Vida já está cheia.")
    elseif rules.item_count("potion") > 0 then
      st.cd = COOLDOWN
      rules.vault_take("potion", 1)
      rules.heal_player(healed)
      -- O número nasce SOBRE O HERÓI (sem x/y/z saía sobre o controller).
      local _, px, py, pz = viber.player_position()
      viber.damage_number("+" .. math.floor(healed) .. " HP", { color = "#7ef29d", x = px, y = py + 2.0, z = pz })
      sfx("heal")
      viber.toast("Poção usada (+" .. math.floor(healed) .. " HP)")
    else
      viber.toast("Sem poções.")
      sfx("error")
    end
  elseif viber.input.pressed("2") then
    if rules.item_count("antidote") > 0 then
      -- `status.venom` é o bind do chip ENVENEADO (texto vazio = sem veneno).
      -- Antes o antídoto era gasto e o toast dizia "neutralizado" mesmo são.
      if not rules.poisoned() then
        viber.toast("Não estás envenenado.")
      else
        st.cd = COOLDOWN
        rules.vault_take("antidote", 1)
        rules.status_clear("venom")
        -- A poção confirma com "heal"; o antídoto calava-se e o jogador não
        -- sabia se o veneno tinha saído.
        sfx("heal")
        viber.toast("Veneno neutralizado.")
      end
    else
      viber.toast("Sem antídotos.")
      sfx("error")
    end
  end
end
