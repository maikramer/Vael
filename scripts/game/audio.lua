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

-- Título (ou cena de menu) por cima: o mundo por trás é decoração. O
-- `rules_host` não corre as regras nesses frames; sem este portão um hostil
-- ao alcance acendia (e renovava) a música de combate por baixo do véu, e o
-- hold de 8 s seguia para a partida depois do "Continuar".
local function ui_owns_frame()
  if viber.scene_name and viber.scene_name() == "menu" then
    return true
  end
  return viber.ui ~= nil and viber.ui.is_open ~= nil
    and viber.ui.is_open("title") == true
end

function on_update(dt)
  local st = viber.state()
  -- Novo Jogo/partida carregada: o estado de sessão (hold aceso) não
  -- atravessa a fronteira. Um LOAD na MESMA cena não muda `session_id` nem
  -- recua o relógio da engine (uptime) — o sinal é o tempo de jogo a recuar,
  -- o mesmo que combat/abilities/hotbar usam.
  local game = viber.game()
  local sid = tonumber(game.session_id) or 0
  local playtime = tonumber(game.playtime_secs) or 0
  local loaded = st.playtime ~= nil and playtime < st.playtime - 1e-3
  if st.session ~= sid or loaded then
    st.session = sid
    if engaged then
      -- O hold pertence à linha temporal abandonada: apaga já, mesmo que o
      -- load tenha vindo de um menu (o ramo abaixo só limpa uma vez).
      combat(false, false)
    end
    engaged, was_boss = false, false
  end
  st.playtime = playtime

  local _, hp = rules.player_hp()
  -- Menu/diário/mapa/cinemática, título e morte roubam o input: o hold de
  -- 8 s da engine manteria a música de combate por baixo (e para sempre,
  -- porque a renovação continuava). Só se limpa quando está acesa.
  if viber.menu_open() or ui_owns_frame() or hp <= 0 then
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
