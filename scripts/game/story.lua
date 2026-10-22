local rules = viber.load("lib/rules.lua")
local dev = viber.load("lib/dev.lua")
-- game/story.lua: o fio do Capítulo I — O Silêncio de Vael (ver STORY.md).
--
--   * PRÓLOGO na primeira entrada no mundo (marca `story_prologo` no save);
--   * cada GUARDIÃO derrotado devolve uma Nota (item `nota`, a missão-mãe
--     `story_notas` do Ancião Edmar conta-as) e toca a cena `nota`;
--   * ENTREGAR as quatro ao Ancião toca o `epilogo`.
--
-- As marcas vivem em `viber.game()` (valores primitivos persistem no save):
-- recarregar um save não repete cenas nem dá Notas a dobrar.
--
-- Host: `world/gameplay.xml` (`tag="always-active"`).

-- kind do abate (nome do script do chefe) → nome da Nota
local GUARDIANS = {
  witch = "Nota do Bosque",
  ["sand-worm"] = "Nota do Ermo",
  ["bog-warden"] = "Nota do Lodo",
  boss = "Nota dos Picos",
}
local PROLOGUE_DELAY = 0.1 -- a cena começa em negro: o carregamento passa direto para ela
-- A cena `nota` espera pelo frame seguinte: um abate na MESMA frame de uma
-- troca de cena empurrava a cinemática (planos do mundo grande) para dentro
-- da cena nova; e com o diário aberto tocava por cima dele.
local NOTE_CINE_DELAY = 0.1
local NOTE_CINE_TTL = 1.5
local note_cine_at, note_cine_scene = 0, nil

local function notes_found(g)
  local n = 0
  for kind in pairs(GUARDIANS) do
    if g["nota_" .. kind] then n = n + 1 end
  end
  return n
end

-- O abate chega do FSM (regra do jogo) via `rules.award_kill`; não há
-- evento global de kill na engine.
rules.on_kill = function(kind)
  local g = viber.game()
  if not GUARDIANS[kind] or g["nota_" .. kind] then
    return
  end
  g["nota_" .. kind] = true
  rules.item_add("nota", 1)
  local n = notes_found(g)
  -- O abate pode chegar durante a troca de cena: sem player não há burst.
  local has, x, y, z = viber.player_position()
  if has then
    viber.burst("sparkle", x, y + 1.2, z, 40)
    viber.ring(x, z, 4, "#9fd8ff")
  end
  viber.toast(GUARDIANS[kind] .. " recuperada (" .. n .. "/4)")
  -- Toca no `on_update` seguinte (ver NOTE_CINE_DELAY): se a cena mudou
  -- entretanto, este controller já não corre e a cena não aparece na sala.
  note_cine_at = viber.time() + NOTE_CINE_DELAY
  note_cine_scene = viber.scene_name and viber.scene_name() or nil
end

function on_update(dt)
  local g = viber.game()
  local st = viber.state()
  st.clock = (st.clock or 0) + dt

  -- Arranque de DEBUG (`viber run --scene main --entry dev`): o título não
  -- abriu, por isso o prólogo também não pode disparar. A marca fica em
  -- `viber.game()` (persiste no save) porque esta entidade RENASCE ao voltar
  -- de um interior: com um latch só de `state`, a cena aparecia à primeira
  -- saída da vila. Um "Novo Jogo" normal limpa a tabela e volta a vê-la.
  if dev.skip_intro() then
    g.story_prologo = true
  end

  -- O ecrã de título (autoload) marca `false` até o jogador escolher. Um
  -- mundo AVULSO (QA, `run mundo.xml` sem projeto) não tem autoload de
  -- título nem campanha: `title_done` fica nil e o prólogo disparava por
  -- cima do cenário de teste. A janela de 1 s deixa o autoload marcar
  -- `false` no arranque normal antes de decidir.
  if g.title_done == nil then
    if st.clock < 1.0 then
      return
    end
    -- Save antigo (anterior ao ecrã de título) carregado em `main`: a
    -- campanha já começou, o `nil` não pode calar o prólogo/epílogo para
    -- sempre. Num mundo avulso `scene_name` é nil e a campanha fica fora.
    local scene = viber.scene_name and viber.scene_name() or nil
    if scene ~= "main" and scene ~= "menu" then
      return
    end
    g.title_done = true
  end
  if g.title_done == false then
    return
  end

  -- Nota pendente: um abate no mesmo frame de uma troca de cena ou com o
  -- diário aberto só toca quando o jogo volta a estar livre (e na mesma
  -- cena); fora da janela curta, perde-se a cena mas não a Nota.
  if note_cine_at > 0 then
    local scene = viber.scene_name and viber.scene_name() or nil
    if viber.time() > note_cine_at + NOTE_CINE_TTL or scene ~= note_cine_scene then
      note_cine_at = 0
    elseif viber.time() >= note_cine_at
        and not viber.cinematic_playing() and not viber.menu_open() then
      note_cine_at = 0
      viber.cinematic("nota")
    end
  end

  if not g.story_prologo and st.clock > PROLOGUE_DELAY
      and not viber.cinematic_playing() and not viber.menu_open() then
    g.story_prologo = true
    viber.cinematic("prologo")
  end

  -- A entrega vive em Lua (`quest_turn_in`). A engine já não emite
  -- `quest_done` para estas missões, por isso o epílogo nunca arrancava.
  if rules.quest_state("story_notas") == "done" and not g.story_epilogo
      and not viber.cinematic_playing() and not viber.menu_open() then
    g.story_epilogo = true
    viber.cinematic("epilogo")
  end

  for _, ev in ipairs(viber.events()) do
    if ev.type == "cinematic_end" and ev.id == "prologo" then
      viber.toast("Fala com o Ancião Edmar, junto ao salão comunal.")
    end
  end
end
