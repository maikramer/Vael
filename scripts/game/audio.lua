-- game/audio.lua: direção musical do combate.
--
-- A engine tem o estado (`CombatMusicState`) e o crossfade; a REGRA de
-- quando há combate é do jogo. `viber.music.combat(active, boss)` acende a
-- layer `battle`/`boss`; quando ninguém renova, a engine expira o hold
-- (`COMBAT_MUSIC_HOLD`, 8 s) e devolve a zona (`explore`/`village`/…).
--
-- Host: `world/gameplay.xml` (`tag="always-active"`).

local rules = viber.load("lib/rules.lua")

local COMBAT_RANGE = 24
local BOSS_KINDS = {
  witch = true, ["sand-worm"] = true, ["bog-warden"] = true, boss = true,
}

-- Estado de SESSÃO (locals do módulo): `combat(false)` limpa o hold da
-- engine, por isso não pode ser disparado em todos os frames sem hostis.
local engaged, was_boss = false, false

local function combat(active, boss)
  if viber.music and viber.music.combat then
    viber.music.combat(active, boss)
  end
end

function on_update(dt)
  local _, hp = rules.player_hp()
  -- Menu/diário/mapa/cinemática e ecrã de morte roubam o input: o hold de
  -- 8 s da engine manteria a música de combate por baixo (e para sempre,
  -- porque a renovação continuava). Só se limpa quando está acesa.
  if viber.menu_open() or hp <= 0 then
    if engaged then
      engaged, was_boss = false, false
      combat(false, false)
    end
    return
  end

  local hostile, boss = false, false
  for _, h in ipairs(rules.hostiles(COMBAT_RANGE)) do
    if (h.hp or 0) > 0 then
      hostile = true
      if BOSS_KINDS[h.kind or ""] then
        boss = true
      end
    end
  end
  if not hostile then
    -- Sem ninguém perto: deixa o hold expirar (a cauda de 8 s evita o
    -- liga/desliga ao limpar um acampamento). `was_boss` fica até a layer
    -- ser limpa pelo próximo `combat(false)` ou por um `engage` expirado.
    return
  end
  if was_boss and not boss then
    -- `engage` só troca a layer quando o hold ANTERIOR expirou: com
    -- inimigos comuns ao lado a renovar o combate, matar o chefe deixava a
    -- layer `boss` presa durante toda a luta. `clear` põe `until = 0`, por
    -- isso o `engage` deste mesmo frame já escolhe `battle`.
    combat(false, false)
  end
  -- Renova o hold todos os frames com um hostil vivo perto.
  combat(true, boss)
  engaged, was_boss = true, boss
end
