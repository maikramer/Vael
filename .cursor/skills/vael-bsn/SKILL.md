---
name: vael-bsn
description: Inspecionar, criar e editar cenas, prefabs e componentes em arquivos .bsn de Vael e Jackdaw. Usar para localizar o dono de uma propriedade, alterar o documento físico pela CLI ou verificar o mundo resolvido.
---

# Edição BSN do Vael

Trabalhe a partir do checkout do Vael, que contém `world.bsn`, `jackdaw.toml` e o pacote `vael-tools`. Com a CLI instalada, execute `vael-tools bsn --help`. A implementação do checkout também roda com `cargo run --release -p vael-tools -- bsn …`.

## Localizar o alvo

`inspect` mostra o arquivo físico antes da expansão de prefabs. Estes comandos localizam a rede de estradas e mostram o trecho original.

```sh
vael-tools bsn inspect world/paths/network.bsn --tag RoadNetwork --json
vael-tools bsn show world/paths/network.bsn --node 'name:RoadNetwork'
```

Copie um seletor do resultado. `id:1551948037216087203` identifica um SceneNodeId decimal no arquivo. Preserve os IDs como strings em clientes JSON. `name:…` exige um nome exato e único. Caminhos como `0/0/0` seguem a ordem física desde zero e podem mudar depois de inserir ou reorganizar irmãos. Um ID de gameplay em `WorldNode.attrs` é outra chave.

Para decidir entre o prefab, a instância, uma propriedade de texto e um componente tipado, consulte [o formato e a ordem de leitura do Vael](references/formato.md).

## Escolher uma operação

- **Mudar um parâmetro da rede**. Confira o diff da prévia ou use o mesmo comando sem `--dry-run` para gravar. O stdout da prévia contém o documento completo.

  ```sh
  vael-tools bsn set world/paths/network.bsn --node 'name:RoadNetwork' \
    --attr paving-feather --value 0.65 --dry-run
  ```

- **Mover uma entidade com pose autoritativa**. Escolha o Transform visível em `show`. Use `--field Transform.translation.x --value 2.5` para um eixo ou `--field Transform.translation --value 'glam::Vec3 { x: 2.5, y: 0.0, z: 8.0 }'` para o vetor. Um campo cujo valor está omitido pelo editor pode ser adicionado no inspector. Confira a pose do pai ao converter coordenadas globais em locais.
- **Editar configurações nativas**. `vael-tools bsn schema PostFxSettings` consulta o schema do projeto. Um literal de enum inclui o caminho da variante. Por exemplo, em `world/settings/postfx.bsn`, use `--node '0' --field PostFxSettings.ssao_quality --value 'bevy_ext_postfx::SsaoQuality::High'`. `schema` reflete a versão da última extração de `jd build`.
- **Inspecionar um binário**. `inspect` e `show` também leem `.bsb`. Para editar uma cópia, use `vael-tools bsn transcode cena.bsb copia.bsn`. O destino deve ser novo. A [referência do formato](references/formato.md) descreve o que sobrevive à conversão.
- **Criar uma entidade, aplicar várias alterações ou editar um prefab**. Siga [as receitas de autoria](references/receitas.md), com exemplos de fragmento BSN, patch JSON e escolha do arquivo correto.

## Conferir o resultado

Examine o diff do arquivo. Execute `vael-tools bsn check world.bsn` para ler a árvore com as referências resolvidas. `vael-tools lint world.bsn --strict` verifica, entre outros itens, referências de assets e scripts. Quando houver mudança visual ou de gameplay, abra o resultado em `jd open .` ou confira no jogo. Esses testes exercitam camadas diferentes.

Para automação, cheque o status de saída. Um filtro de leitura pode devolver zero linhas. Uma mutação exige um alvo único. Se houver edições do mesmo documento ainda abertas no editor, coordene a gravação e o recarregamento para preservar as duas versões.
