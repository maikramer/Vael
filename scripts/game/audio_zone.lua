-- game/audio_zone.lua: reverb e BGM por cena.
--
-- Vive num AUTOLOAD (`world/autoload/player.xml`), por isso corre também
-- dentro das cenas de interior (que não incluem `world/gameplay.xml`).
-- A engine só expõe o preset e a layer (`viber.audio.reverb`,
-- `viber.music.layer`); a regra "interior leva room, exterior leva off" e
-- "a cena de menu leva a música do título" é do jogo.

local current_scene
local current_reverb
local current_layer

function on_update(_dt)
  local scene = viber.scene_name and viber.scene_name() or nil
  if scene == current_scene then
    return
  end
  current_scene = scene

  -- A cauda de 8 s do combate é renovada por `game/audio.lua`, que só é
  -- carregado em `main`: ao entrar num interior/menu ela seguia para a cena
  -- nova (a engine não reinicia o `CombatMusicState` no load da cena). Um
  -- clear por troca de cena chega e não briga com o host — o hold só se
  -- renova onde há hostis.
  if viber.music and viber.music.combat then
    viber.music.combat(false, false)
  end

  local menu = scene == "menu"
  -- Interiores são `scenes/interiors/*` (id `interiors/<sala>`). A regra
  -- antiga ("tudo o que não é main nem menu") punha reverb de sala numa cena
  -- própria ao ar livre (ex.: `qa-batch`).
  local interior = scene ~= nil and scene:find("^interiors/") ~= nil
  local reverb = interior and "room" or "off"
  if reverb ~= current_reverb and viber.audio and viber.audio.reverb then
    current_reverb = reverb
    viber.audio.reverb(reverb)
  end

  local layer = menu and "menu" or nil
  if layer ~= current_layer and viber.music and viber.music.layer then
    current_layer = layer
    viber.music.layer(layer)
  end
end
