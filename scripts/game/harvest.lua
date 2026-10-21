-- A colheita nativa (harvest.rs) é quem parte o prop: toco, loot, XP,
-- nome gravado para o load não devolver a árvore. Este controller fazia o
-- mesmo golpe no mesmo [J] e o recurso entrava duas vezes (três, com o
-- evento collect). Fica vazio de propósito: apagá-lo do XML reabria o
-- caminho nativo na mesma, mas o include já aponta para aqui.
function on_update(_dt)
end
