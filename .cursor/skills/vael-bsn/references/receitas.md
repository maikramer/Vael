# Receitas de autoria

## Criar e inserir

Um fragmento pode começar como uma entidade BSN inteira. Este exemplo cria um grupo do mundo com nome e pose local. Salve-o como `marco.bsn` em uma pasta de trabalho.

```text
#"Marco da praça"
bevy_transform::components::transform::Transform {
    translation: glam::Vec3 { x: 2.0, y: 0.0, z: 1.5 },
}
bevy_world_xml::bsn::WorldNode {
    tag: "Group",
    attrs: [],
    text: "",
    origin_file: "",
    origin_path: [],
    pose: true,
}
```

```sh
vael-tools bsn insert caminho/modulo.bsn marco.bsn --parent 0 --dry-run
```

`insert` acrescenta o fragmento a Children ou abre a lista se o pai for uma folha. É possível inserir uma subárvore completa. Ao copiar um fragmento existente, preserve IDs apenas para a mesma identidade. A CLI recusa SceneNodeIds repetidos na inserção. O Jackdaw atribui IDs ao salvar novos nós. Uma entidade nova que precise aparecer no jogo deve ser alcançável pelo mundo, e sua tag e seus componentes precisam ser reconhecidos pelos consumidores.

## Alterar vários valores no mesmo arquivo

Para editar a rede e um ponto de controle em conjunto, grave este array em `ajustes.json`. A chave `value` do JSON é sempre uma string. Em `field`, o conteúdo dessa string é interpretado como um literal BSN. Para escrever um campo tipado string, por exemplo, use `"value": "\"pedra\""`.

```json
[
  {"node":"name:RoadNetwork","attr":"paving-feather","value":"0.65"},
  {"node":"name:Way mid_n","attr":"xz","value":"0 32.5"}
]
```

```sh
vael-tools bsn patch world/paths/network.bsn ajustes.json --dry-run
vael-tools bsn patch world/paths/network.bsn ajustes.json
vael-tools bsn check world.bsn
```

O lote aplica as operações em ordem em memória. Uma falha identifica a operação e deixa o original intacto. A substituição final preserva as permissões e segue o symlink até o documento real. `--dry-run` emite BSN para stdout; use outro caminho para guardar uma prévia. Redirecionar para o próprio arquivo de entrada trunca o original antes de iniciar a CLI.

## Separar uma falha de sintaxe de uma edição sem efeito

1. Abra o trecho com `show`. Confira nome, componente completo, tipo do valor e o arquivo físico.
2. A CLI verifica a sintaxe com o parser nativo. A reflexão e os sistemas do jogo dão significado a cada valor. Para um campo nativo, consulte `bsn schema Tipo --project caminho/do/Vael`. O campo pode estar omitido por representar um default. Use o inspector para materializá-lo ou uma edição BSN explícita baseada no schema. A edição pontual de campo requer que o campo exista. Para um atributo DSL ausente, `set --attr` insere o par em `attrs`.
3. Rode `bsn check` na raiz do mundo. Prefabs, includes e caminhos relativos precisam do contexto do projeto. `--root` informa a raiz quando necessário.
4. Se o jogo ainda mostra outro valor, confira a precedência do Transform, das propriedades tipadas e dos overrides da instância. Para configurações de ambiente, verifique também os overrides de QA usados ao lançar o processo.

## Verificar uma reorganização

Ao reorganizar pastas do Outliner ou migrar componentes, guarde uma cópia anterior para comparar o mundo resolvido. No checkout irmão `bevy_extensions`, execute `cargo run -p bevy_ext_world --features terrain --example bsn_equiv -- /caminho/antes.bsn /caminho/depois.bsn`. Preserve a resolução dos prefabs nas duas cópias. Uma alteração deliberada de pose ou de valor deve produzir uma diferença. Para trabalhos que preservem saves, confira também `bsn_origin_equiv` e os contratos de identidade em `Vael/AGENTS.md`.
