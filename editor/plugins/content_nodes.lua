-- Templates do Vael no diálogo Add Node.
--
-- O catálogo da engine continua a ser a lista genérica; este plugin
-- SOBREPÕE entradas com a mesma tag (o `<ResourceNode>` é um conceito de
-- colheita do jogo, não da engine).

return {
  api = 1,
  id = "content_nodes",
  title = "Nós do Vael",
  nodes = {
    {
      tag = "ResourceNode",
      label = "ResourceNode (colheita — dentro do template de um spawner)",
      snippet = '<ResourceNode kind="wood" yield="3" />',
      group = "content",
    },
  },
}
