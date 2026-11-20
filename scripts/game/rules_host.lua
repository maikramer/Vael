-- Controller sempre ativo: veneno, eventos de morte, binds do HUD.
local rules = vael.load("lib/rules.lua")

-- A cena principal é recriada ao voltar de um interior. O registo de
-- inimigos vive em `vael.game()` e sobrevivia: entradas de criaturas já
-- destruídas contavam para o gate do chefe (soft-lock) e cresciam a sessão
-- inteira. `vael.state()` distingue arranque de hot-reload.
local st = vael.state()
if not st.ready then
  st.ready = true
  local game = vael.game()
  game.enemies = {}
  game.elites = {}
end

-- Um frame que a UI já está a usar (título/menu) não leva regras: o mundo
-- por trás é decoração e o respawn de 5 s (ou o prólogo) não pode correr
-- por baixo do véu. `scene_name` nil = mundo avulso (QA), que continua a
-- tickar. O título pode viver sobre `main` (arranque direto no mundo).
local function ui_owns_frame()
  if vael.scene_name and vael.scene_name() == "menu" then
    return true
  end
  return vael.ui ~= nil and vael.ui.is_open ~= nil
    and vael.ui.is_open("title") == true
end

function on_update(dt)
  if ui_owns_frame() then
    -- Drena a fila deste path para os eventos da última frame de jogo
    -- (hit/kill) não serem aplicados quando o jogador entrar em `main`.
    for _ in ipairs(vael.events()) do end
    return
  end
  local game = vael.game()
  if (tonumber(game.hp) or 0) > 0
      and (tonumber(game.dying) or 0) <= 0
      and not game.death_pending then
    local pid = vael.player_id and vael.player_id()
    if pid and vael.vael and vael.vael.puppet_state and vael.vael.puppet_release
        and vael.vael.puppet_state(pid) == "dead" then
      vael.vael.puppet_release(pid)
    end
  end
  rules.tick(dt)
end
