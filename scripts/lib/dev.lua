-- scripts/lib/dev.lua: interruptores de ARRANQUE para debug — sem efeito num
-- jogo normal.
--
-- O sinal vem do CLI, não de um save nem de um ficheiro de configuração:
--
--   viber run --scene main --entry dev
--
-- `--scene main` começa no mundo em vez da cena de menu; `--entry dev` chega
-- pela entrada `<SceneEntry name="dev">` de `world.xml`. O nome da entrada é
-- visível em `viber.scene_entry()`, por isso o título (`ui/title.lua`) e o
-- prólogo (`game/story.lua`) — que vivem em controladores diferentes — leem
-- o MESMO sinal sem passar nada entre si.
--
-- Porque não um save ou `viber.game()`: um save é estado do jogador e um
-- modo de arranque não pode ficar gravado por engano; `--entry` é por
-- execução. A entrada fica em `main` de propósito: numa cena própria
-- (`interiors/*`) ou num mundo QA o `entry` é outro/nil e nada muda.
--
-- `world.xml` tem uma entrada neutra `spawn` ANTES de `dev`: um
-- `change_scene("main")` sem entrada (o "Novo Jogo" do título) chega a
-- `spawn`, não ao modo debug. Não inverter a ordem.
local M = {}

local DEBUG_ENTRY = "dev"

--- Nome da entrada de arranque, só quando a cena é `main`.
function M.entry()
  if not (viber.scene_name and viber.scene_entry) then
    return nil
  end
  if viber.scene_name() ~= "main" then
    return nil
  end
  return viber.scene_entry()
end

--- `true` quando o run foi pedido com `--scene main --entry dev`.
function M.skip_intro()
  return M.entry() == DEBUG_ENTRY
end

return M
