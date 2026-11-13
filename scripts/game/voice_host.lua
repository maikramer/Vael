-- scripts/game/voice_host.lua: Host da Voz Viva ([T] Coro Semântico + Pacto de Caça).
--
-- Carregado em `world/autoload/player.bsn` como entidade `always-active`.
-- Abre a barra de fala (`say.ask` com `allow_fallback = true`) ao premir [T]
-- e encadeia `rules.on_kill` de forma segura (preservando `game/story.lua`)
-- para resolver Pactos de Caça.

local rules = viber.load("lib/rules.lua")
local say = viber.load("ui/say.lua")
local voice = viber.load("lib/voice.lua")

local our_on_kill

local function dialogue_open()
  if say.active and say.active() then
    return true
  end
  if not (viber.ui and viber.ui.get and viber.time) then
    return false
  end
  local now = viber.time()
  local t = tonumber(viber.ui.get("dialogue.until")) or 0
  return t > now and (t - now) <= 2.0
end

local function handle_pact_kill(kind, _id)
  local g = viber.game()
  local pact_kind = g.voice_pact_kind
  local pact_until = tonumber(g.voice_pact_until) or 0
  local now = (viber.time and viber.time()) or 0
  if not pact_kind or pact_kind == "" or now >= pact_until then
    return
  end

  g.voice_pact_kind = nil
  g.voice_pact_until = 0

  local has_pos, px, py, pz = false, 0, 0, 0
  if viber.player_position then
    has_pos, px, py, pz = viber.player_position()
  end
  local pos = has_pos and { x = px, y = py, z = pz } or nil

  local matches = (kind == pact_kind)
    or (rules.kind_matches and rules.kind_matches(kind, pact_kind))

  if matches then
    local label = voice.label_for_creature(pact_kind)
    rules.add_xp(15, "Pacto de Caça")
    rules.heal_player(15)
    if has_pos and viber.burst then
      viber.burst("sparkle", px, py + 1.1, pz, 22)
    end
    if pos and viber.sound then
      viber.sound("levelup", pos)
    end
    viber.toast("Pacto cumprido contra " .. label .. "! (+15 XP, +15 vida)")
  else
    g.hp = math.max(1, (tonumber(g.hp) or 10) - 10)
    if pos and viber.sound then
      viber.sound("error", pos)
    end
    if has_pos and viber.burst then
      viber.burst("smoke", px, py + 0.9, pz, 12)
    end
    viber.toast("Pacto quebrado: o eco volta-se contra ti! (-10 vida)")
  end
end

our_on_kill = function(kind, id)
  local prev = rules._vael_voice_prev_on_kill
  if prev and prev ~= our_on_kill then
    prev(kind, id)
  end
  handle_pact_kill(kind, id)
end

local function ensure_kill_hook()
  if rules.on_kill ~= our_on_kill then
    if rules.on_kill ~= rules._vael_voice_hook then
      rules._vael_voice_prev_on_kill = rules.on_kill
    end
    rules._vael_voice_hook = our_on_kill
    rules.on_kill = our_on_kill
  end
end

ensure_kill_hook()

function on_update(_dt)
  ensure_kill_hook()

  if (viber.scene_name and viber.scene_name() == "menu")
      or (viber.ui and viber.ui.is_open and viber.ui.is_open("title")) then
    return
  end
  if viber.menu_open and viber.menu_open() then
    return
  end
  if dialogue_open() then
    return
  end

  local g = viber.game()
  if (tonumber(g.hp) or 1) <= 0 then
    return
  end
  local alive, hp = rules.player_hp()
  if alive and (hp or 0) <= 0 then
    return
  end

  if viber.input and viber.input.pressed and viber.input.pressed("t") and not say.active() then
    say.ask({
      prompt = "Entoar no Coro:",
      hint = "Diga um verso elemental ou o nome de uma fera...",
      speaker = viber.player_id and viber.player_id() or nil,
      allow_fallback = true,
      on_result = function(parsed, text)
        if text and text ~= "" then
          voice.cast(text, parsed)
        end
      end,
    })
  end
end

function self_test()
  ensure_kill_hook()
  assert(voice.self_test() == true, "voice.self_test falhou")
  assert(rules.on_kill == our_on_kill, "rules.on_kill não encadeado")
  return true
end

return {
  on_update = on_update,
  self_test = self_test,
  handle_pact_kill = handle_pact_kill,
}
