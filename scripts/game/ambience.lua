-- game/ambience.lua: camas de ambiente por bioma.
--
-- A MÚSICA de bioma é da engine (`music::auto_zone`: o `BiomeRegion.id` acende
-- a `<MusicLayer>` homónima, e o combate ganha sempre). Aqui só vivem os loops
-- contínuos que não são água/chuva — esses são nativos (`ambient.rs`).
--
-- O volume vai para o bus de AMBIENTE (`<AudioMixer ambience>`), não para o
-- SFX. Os clips são pedidos por caminho sob o `sfx_dir` do config.yaml.
-- Host: `world/gameplay.xml` (`tag="always-active"`).

local LOOPS = {
  ["dark-forest"] = { clip = "ambient/forest_loop.ogg", volume = 0.34 },
  desert = { clip = "ambient/desert_loop.ogg", volume = 0.30 },
  swamp = { clip = "ambient/swamp_loop.ogg", volume = 0.32 },
  ["frozen-peaks"] = { clip = "ambient/peaks_loop.ogg", volume = 0.38 },
}

-- A vila não é um `BiomeRegion` (fica no vão dos polígonos): usa o mesmo
-- raio do `bgm_zone` para a camada de mercado.
local VILLAGE = { clip = "ambient/village_loop.ogg", volume = 0.26 }
local ID = "biome-ambience"
local current
local next_t = 0

local function desired()
  local scene = viber.scene_name and viber.scene_name() or nil
  if scene and (scene == "menu" or scene:find("^interiors/") ~= nil) then
    return nil
  end
  local biome = viber.biome and viber.biome() or nil
  if biome and LOOPS[biome] then
    return LOOPS[biome]
  end
  local ok, x, _, z = viber.player_position()
  if ok and x * x + z * z < 55 * 55 then
    return VILLAGE
  end
  return nil
end

function on_update(_dt)
  if not viber.sound_loop or not viber.sound_stop then
    return
  end
  -- 4×/s chega: o bioma muda ao atravessar uma fronteira, não por frame.
  local now = viber.time()
  if now < next_t then
    return
  end
  next_t = now + 0.25

  local spec = desired()
  local clip = spec and spec.clip or nil
  if spec then
    -- IDEMPOTENTE: a troca de cena limpa os loops do lado da engine
    -- (`reset_scene_resources`), mas os locals do módulo sobrevivem. Re-pedir
    -- a cada tique garante que o loop volta; o driver reconcilia sem
    -- reiniciar (mesmo id + mesmo clip).
    viber.sound_loop(ID, clip, { bus = "ambience", volume = spec.volume })
    if clip ~= current then
      viber.log("ambiência: " .. clip)
    end
  elseif current then
    viber.sound_stop(ID)
    viber.log("ambiência: parada")
  end
  current = clip
end
