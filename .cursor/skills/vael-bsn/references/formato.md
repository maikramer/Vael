# Formato observado no Jackdaw 0.20.2 e no Vael

## Gramática do documento físico

O lexer, os tipos de sintaxe e as regras de leitura estão em `jackdaw_bsn/src/syntax/{lex,types,parse}.rs` da versão fixada pelo Cargo.lock. `parse_bsn_text` transforma a sintaxe em `SceneBsnAst`. A CLI usa tokens dessa mesma biblioteca para delimitar as substituições em bytes UTF-8.

| Construção | Significado e consequência para a edição |
|---|---|
| `#nome`, `#"Praça central"` | Nome da entidade. Pode repetir entre irmãos. O Vael também deriva nomes de tag, id e caminho de asset. |
| `Tipo`, `Tipo(valor)`, `Tipo { campo: valor }` | Componente unitário, tupla ou struct. O caminho completo desambigua tipos homônimos. A forma antiga `Tipo { 0: valor }` também representa uma tupla. |
| `bevy_ecs::hierarchy::Children [ … -- … ]` | Uma relação contém uma lista de entidades. `--` separa irmãos ou raízes. `,` separa campos e valores. |
| `glam::Vec3 { … }`, `Tipo::Variante`, `[a, b]`, `map[(k, v)]` | Valores tipados, variantes, listas e mapas. Para substituir uma lista ou mapa inteiro, passe um literal BSN ao campo. Consulte o schema para o tipo de elemento. |
| `:"cena.bsn"`, `@"cena.bsn"`, `@Tipo { … }` | Entradas da gramática de cenas do Jackdaw. No Vael a composição do mundo usa principalmente o componente IsA com Include. |
| `// …`, nomes e texto Unicode | Comentários, cabeçalhos e grafia fazem parte do arquivo de autoria. Delimitadores dentro de strings não dividem entidades. |

O Jackdaw também tem representação binária `.bsb` e migrações para documentos históricos. Os comandos de edição desta CLI operam na sintaxe textual atual. O leitor de mundos do Vael usa sua própria resolução de módulos. A conversão `bsn convert` aceita XML legado e serve geradores que ainda o produzem.

## Compatibilidade, cabeçalhos e representação binária

O preâmbulo pode conter `// jackdaw 0.20.2 | bevy 0.20` e `// jackdaw asset caminho::Tipo`. O primeiro orienta a migração de nomes refletidos e o segundo anuncia o tipo de asset. O loader tenta a gramática atual primeiro. Em cenas antigas ele também reconhece vírgulas separando entidades. Para editar esse legado com os seletores físicos, salve uma cópia no editor atual e confira o diff.

No codec v1 de `jackdaw_bsn/src/binary.rs`, os quatro primeiros bytes são `89 42 53 42` e a versão seguinte é um `u16` little-endian. Seguem o preâmbulo opcional, a quantidade de raízes e os nós. Tamanhos de listas e strings usam `u64` little-endian. Cada patch indica nome, base, tipo, struct, tupla, template ou filhos. Os valores têm discriminantes distintos para números, booleanos, strings, tipos, structs, tuplas, listas e mapas. O leitor detecta o formato pela assinatura e verifica truncamento, tags, versão e bytes excedentes.

`bsn transcode cena.bsn export.bsb` usa esse codec e preserva o arquivo de origem. A conversão de volta recupera os valores e o preâmbulo. Espaçamento e comentários no corpo são propriedades do texto original. Para uma alteração que precise manter o diff mínimo, trabalhe no `.bsn` original.

## Qual propriedade prevalece

| Onde está gravado | Como o Vael lê |
|---|---|
| `bevy_world_xml::bsn::WorldNode` | Preserva `tag`, pares ordenados de strings em `attrs` e `text`. Parâmetros como o `xz` de um Way e `paving-feather` de uma RoadNetwork vêm daqui. |
| `Transform` e `pose` | Em nós com `pose: true`, a translação, a rotação e a escala nativas alimentam a pose efetiva. Nas tags de terreno e UI que usam coordenadas próprias, confira `pose` e os campos da tag. A rotação de uma luz direcional determina a direção. |
| `GltfSource`, luzes, `ColliderSpec`, `PortalSpec`, outros specs | O loader projeta campos nativos na representação consumida pelo jogo. Ao ver o mesmo parâmetro em duas formas, edite a fonte que o loader usa. `GltfSource.path`, por exemplo, alimenta o URL do modelo. |
| `CustomProperties` e `AuthoredProps` | A leitura suporta dados antigos. Uma propriedade Jackdaw homônima tem precedência. |
| Entidade sem `WorldNode` | Pode ser uma pasta transparente na vista do mundo ou conter configurações nativas, como `PostFxSettings`. Uma busca por tag pode omiti-la. Use `inspect --component`. |
| `SceneNodeId`, `PrefabEntityId`, `WorldOrigin`, `origin_file/origin_path` | Identificam o nó do editor, o vínculo de prefab e a proveniência usada por consumidores como saves. Um caminho histórico `.xml` pode continuar válido como identidade. Conserve-o nas edições comuns. |

## Prefabs e resolução

`world.bsn` é a entrada do jogo. `IsA.source` referencia o prefab e `deleted` registra remoções da instância. O Vael chama o resolver de prefabs do Jackdaw e reconstrói a árvore de gameplay. No checkout padrão, `jackdaw.toml` aponta para `assets/public`, e `assets/public/world` referencia `world/`. Um nó presente no Outliner expandido pode existir fisicamente apenas no módulo. Alterar o módulo afeta as instâncias que o herdam. Para uma exceção local, selecione a instância no editor e grave o override. Na raiz da instância, adicionar um `WorldNode` substitui todo o `WorldNode` herdado. Para registrar somente proveniência, use o componente `WorldOrigin`. Durante o Play, abra o módulo em uma aba própria para testar a edição de seus nós internos por `SceneNodeId`.

## Onde conferir detalhes de implementação

- Em `../bevy_extensions/crates/bevy_ext_world/src/bsn/`, `components.rs` define os componentes de ponte, `load.rs` resolve prefabs e faz a projeção, `specs.rs` traduz os componentes tipados e `pose.rs` converte a pose. `terrain.rs` trata as features que aparecem como filhas no editor e sobem na vista de gameplay.
- Em `Vael/crates/vael_engine/src/recipes/`, o parser das tags define os nomes, unidades e defaults das strings em `attrs`. O schema de reflexão descreve os tipos nativos. Para uma tag nova, confira ambos os lados.
- Em `Vael/cli/src/bsn_edit/` ficam a seleção física e as mutações. `Vael/cli/src/bsn.rs` contém a verificação pelo loader e a conversão de XML. `Vael/docs/JACKDAW.md` descreve o editor, suas extensões e a ligação ao jogo.
