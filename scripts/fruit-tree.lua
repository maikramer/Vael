-- scripts/fruit-tree.lua: a berry_bush é um alvo de interação [J] NATIVO.
--
-- A colheita (golpe, loot, XP, troca pela árvore sem frutas) vive em
-- game/harvest.lua. Este script só registra o verbo no `ScriptInteraction`
-- da própria árvore — é esse componente que o `collect_ui_prompt` da engine
-- lê para o prompt do HUD. Sem ele, o `viber.prompt` do controller é
-- sobrescrito pelo collect nativo do frame seguinte (a ordem da engine é
-- Collect → Bind → Script; um prompt publicado no Script só apareceria no
-- bind do frame seguinte, já limpo).
--
-- `set_interaction` escala o alcance autorado (3.5 m → 1.75 m), o mesmo
-- número que `viber.interact_range` devolve à colheita.
--
-- Os props abaixo são DADOS de colheita lidos pelo controller
-- (`game/harvest.lua` via `viber.player_nearby`), não por este script. A
-- anotação existe para o `viber scene lint` saber que são intencionais
-- (`script-prop-unknown` sem ela).
--@export destructible: string = ""
--@export resource-kind: string = ""
--@export resource-yield: number = 1
function on_update(dt)
  local st = viber.state()
  if st.registered then
    return
  end
  st.registered = true
  viber.set_interaction("Colher frutas", "j", 3.5)
end
