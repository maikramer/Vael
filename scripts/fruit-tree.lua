-- scripts/fruit-tree.lua: a berry_bush é um alvo de interação [J] NATIVO.
--
-- A colheita (golpe, loot, XP, troca pela árvore sem frutas) vive em
-- game/harvest.lua. Este script só registra o verbo no `ScriptInteraction`
-- da própria árvore — é esse componente que o `collect_ui_prompt` da engine
-- lê para o prompt do HUD. Sem ele, o `vael.prompt` do controller é
-- sobrescrito pelo collect nativo do frame seguinte (a ordem da engine é
-- Collect → Bind → Script; um prompt publicado no Script só apareceria no
-- bind do frame seguinte, já limpo).
--
-- `set_interaction` escala o alcance autorado (3.5 m → 1.75 m), o mesmo
-- número que `vael.interact_range` devolve à colheita.
--
-- Os props abaixo são DADOS de colheita lidos pelo controller
-- (`game/harvest.lua` via `vael.player_nearby`), não por este script. A
-- anotação existe para o lint saber que são intencionais
-- (`script-prop-unknown` sem ela).
--@export destructible: string = ""
--@export resource-kind: string = ""
--@export resource-yield: number = 1
function on_update(dt)
  local st = vael.state()
  if st.registered then
    return
  end
  st.registered = true
  vael.set_interaction("Colher frutas", "j", 3.5)
end
