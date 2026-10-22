-- Controller sempre ativo: veneno, eventos de morte, binds do HUD.
local rules = viber.load("lib/rules.lua")

-- A cena principal é recriada ao voltar de um interior. O registo de
-- inimigos vive em `viber.game()` e sobrevivia: entradas de criaturas já
-- destruídas contavam para o gate do chefe (soft-lock) e cresciam a sessão
-- inteira. `viber.state()` distingue arranque de hot-reload.
local st = viber.state()
if not st.ready then
  st.ready = true
  local game = viber.game()
  game.enemies = {}
  game.elites = {}
end

-- Um frame que a UI já está a usar (título/menu) não leva regras: o mundo
-- por trás é decoração e o respawn de 5 s (ou o prólogo) não pode correr
-- por baixo do véu. `scene_name` nil = mundo avulso (QA), que continua a
-- tickar. O título pode viver sobre `main` (arranque direto no mundo).
local function ui_owns_frame()
  if viber.scene_name and viber.scene_name() == "menu" then
    return true
  end
  return viber.ui ~= nil and viber.ui.is_open ~= nil
    and viber.ui.is_open("title") == true
end

function on_update(dt)
  if ui_owns_frame() then
    -- Drena a fila deste path para os eventos da última frame de jogo
    -- (hit/kill) não serem aplicados quando o jogador entrar em `main`.
    for _ in ipairs(viber.events()) do end
    return
  end
  rules.tick(dt)
end
