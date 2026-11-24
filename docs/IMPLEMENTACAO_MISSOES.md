# O Silêncio de Vael — plano técnico das missões

**Estado:** especificação para implementação. O [enredo](CAMPANHA_PRINCIPAL.md) é a referência para a ordem, as falas e os desfechos. O [GDD](../GDD.md) define a experiência. Este documento identifica o trabalho no repositório Vael, as dependências entre entregas e a prova de que cada uma funciona. Os nomes de IDs, chaves e operações assinalados como propostos são decisões de desenho.

**Primeira entrega:** de Novo Jogo até a criança encontrar o portão em M01a e o diário apontar à região escolhida. Usar Edmar, um marco real e duas opções de legenda. A pessoa que testa deve poder sair e voltar durante o exercício, comprar na vila e encontrar apenas ofertas da história principal. A segunda entrega liga esse começo aos quatro guardiões existentes. A primeira fatia para testar a mecânica nova de maior risco é M06 → M06a → M07 → M08, com um cenário que comece na entrega das Notas.

## Jogar a abertura implementada

Para jogar desde o prólogo, abre **Novo Jogo**. Para testar directamente no mundo principal, corre o comando abaixo na raiz do Vael. O cenário coloca o herói perto de Edmar, com vida protegida, e usa uma pasta temporária para os saves. Os slots de QA ficam separados dos slots da partida normal.

```sh
cargo run --release -p vael -- world.bsn --scenario campaign/opening
```

| Missão | O que experimentar | Como reconhecer o resultado |
|---|---|---|
| M01 · O contrato dobrado | Junto de Edmar, abre o contrato com **E**. Anda à volta dele até o convite indicar **Bosque · Vessa** e confirma com **E**. Afastar-te permite recomeçar a escolha. | O contrato fica assinado e o diário segue **A primeira dobra**. As outras três margens também podem ser escolhidas. |
| M01a · A primeira dobra | Segue a seta até à **Atalaia do Vale** e usa **F** para assinar. Junto da criança no portão norte, abre a folha com **E**. Anda à volta dela e confirma a seta do curral. Segue-a, abre a folha outra vez e escolhe o telhado ou o muro. | A criança experimenta o desenho a pé. Confirma com **E** junto dela quando chegar ao portão. O diário guarda a legenda e a seta aponta para o dador escolhido. Experimenta carregar o autosave entre o desenho e a chegada. |
| M02 · A cantiga de Vessa | Fala com Vessa na orla da Floresta Sombria. Segue as cinco fitas ou a cantiga. Podes fazer um desvio ao círculo de menires e examinar os sulcos com **E**. Na clareira, sai da marca da Bruxa antes da queda da maldição. | A vitória concede uma **Nota do Bosque** para a missão das quatro Notas. Volta a Vessa, mostra-lhe a Nota com **E** e ouve o verso final. Ela paga uma vez e a Nota continua contigo. |

No ensaio do quadro de avisos, observa-o antes de falar com Edmar e depois de voltar de Vessa. As novas ofertas secundárias ficam reservadas até à entrega das quatro Notas, o fim alcançável deste capítulo. O desbloqueio definitivo em M14 continua no plano de produção. Um save anterior que já tenha o contrato em curso mantém a progressão que tinha. Para validar a abertura completa usa Novo Jogo ou o cenário acima.

## 1. Pontos de integração verificados

| Área | Onde trabalhar | Consequência para a implementação |
|---|---|---|
| Catálogo | [Definições](../content/src/lib.rs), [validação](../content/src/validate.rs), [índice de quests](../quests/index.json) | `QuestDef` já tem `stages`, `requires`, `next`, recompensa e diálogo. Os tipos reconhecidos hoje são `kill`, `collect`, `visit`, `decision` e `craft`. Uma visita pode ser reconhecida por proximidade. Assinar o marco e confirmar um desenho exigem uma interacção explícita. |
| Autoridade | [Regras de quest](../src/quest.rs), [eventos](../src/quest_events.rs), [ponte Luau](../scripts/lib/rules.lua) | O Rust decide aceitar, avançar e entregar. `requires` exige **todas** as dependências em `done`. `next` segue a cadeia do dador. A ordem interna de `QuestRules` é alfabética. Mudar a ordem dos ficheiros no índice é insuficiente para ordenar as ofertas. A ponte Luau apresenta a resposta, sincroniza recolhas e agenda o save da entrega. |
| Guardiões e festa | [Controlador de história](../scripts/game/story.lua), [contrato de Edmar](../quests/story_quests.json), [cinemáticas](../world/cinematics.bsn) | O primeiro abate de cada espécie de guardião atribui uma Nota e deixa uma marca individual. A entrega de `story_notas` consome os itens e desencadeia `epilogo`. O sucessor de Edmar no catálogo actual é `story_echoes`. |
| Ofertas | [Dador genérico](../scripts/npc/dialogue.lua), [dador próprio](../scripts/npc/forest-wolves.lua), [decisões](../scripts/npc/decision-quest.lua), [conversas](../scripts/townsfolk.lua), [quadro](../scripts/notice-board.lua) | Várias entradas podem mostrar uma fala ou abrir a barra de texto antes da tentativa de aceitar. O quadro percorre uma lista própria de quatro recompensas e tem fluxo com e sem interpretação de texto. Filtrar antes do convite e voltar a verificar ao confirmar. |
| Apresentação | [Diário e seguimento](../src/ui/screens.rs), [seta e marcadores](../src/ui/waypoint.rs), [mapa](../src/ui/map.rs), [HUD](../src/hud.rs) | `tracked_quest`, a escolha do cartão e a procura do dador mais próximo podem produzir alvos distintos. O mesmo objectivo deve dar o nome no diário, o texto do cartão, o alvo no mundo e a posição no mapa. A UI dos créditos já pode abrir a partir do título. |
| Persistência e autoria | [Save](SAVE.md), [QA](QA.md), [editor Jackdaw](JACKDAW.md) | Usar o KV persistente, os slots e as cenas BSN estáticas. Para novas entidades, criar poses e referências autorais no editor. Um ID de entidade em memória não é uma referência válida depois de carregar uma cena. |

## 2. Contrato de progressão

### Catálogo e referências

Manter os IDs públicos de quests, recompensas e visitas. Acrescentar ao conteúdo de Vael uma tabela ordenada proposta em `data/campaign.json`, lida e validada junto do catálogo. Cada entrada contém um ID editorial (M01, M01a…), a referência da quest, o marco de conclusão e as âncoras relevantes. A ordem da tabela é a ordem de apresentação. A presença de uma quest na tabela identifica-a como principal. Uma quest catalogada que não pertença à tabela é local, inclusive `story_glass_envoy` e `story_echoes`. Validar referências em ambas as direcções entre conteúdo e mundo, incluindo âncoras novas, no CLI e no painel de conteúdo do Jackdaw.

Exemplo parcial do formato **proposto** para as duas vistas de um único contrato. `accepted` significa que a quest está em `active`, `ready` ou `done`. `done` significa entrega confirmada. Os nomes de Mxx servem para ligações e QA. Os títulos apresentados ao jogador vêm do catálogo de textos PT/EN.

```json
[
  {"step": "M01", "quest": "story_notas", "complete_on": "accepted", "anchor": "npc_elder"},
  {"step": "M06", "quest": "story_notas", "complete_on": "done", "anchor": "npc_elder"}
]
```

O ficheiro completo incluirá as 17 entradas. M01 e M06 são a única referência de quest repetida, com condições distintas. Manter o catálogo de itens e as recompensas nos JSON de quest. Evitar um segundo conjunto de estados `Mxx_done`. A vista do diário deriva das quests, das quatro marcas individuais e dos poucos factos físicos e escolhas descritos abaixo.

### Disponibilidade e seguimento

1. **Início.** Novo Jogo só oferece o contrato de Edmar. Ao aceitá-lo, registar a região preferida e oferecer M01a. A aceitação de `story_notas` satisfaz M01. Exigir a sua *entrega* para abrir M01a criaria uma dependência circular.
2. **Quatro terras.** M01a entregue abre os quatro dadores e fixa a região escolhida. Um guardião vencido cedo continua a valer. O diário deriva a Nota dessa marca e reconcilia o objectivo regional quando o dador o aceita. O pagamento local continua disponível uma única vez. M06 requer M01a e as quatro Notas distintas. O jogador pode ir à mesa sem recolher quatro pagamentos regionais. Uma recarga ou um segundo abate não duplica uma Nota.
3. **Depois da festa.** `story_notas=done` dá acesso a M06a na praça após a cena de Edmar. `story_epilogo` pertence ao primeiro capítulo. A passagem do Campo antes da hora ou os cinco marcos da romaria já assinados preservam as descobertas e deixam o novo objectivo disponível.
4. **Fecho.** Confirmar a correcção do mapa conclui M14 e grava os créditos da campanha como pendentes. Sair ou saltar esses créditos leva à retoma no Vale. Aí abrem as ofertas locais, mantendo os seus `requires`. Abrir os créditos pelo título continua a ser uma acção de menu. Na cadeia do próprio Edmar pode manter-se o elo para os Ecos se a oferta do sucessor obedecer ao desbloqueio pós-campanha.
5. **Oferta, acompanhamento e entrega são decisões diferentes.** Um save antigo com uma local `active` ou `ready` conserva-a no diário e no dador. Entregar uma repetível volta a `not_taken`, tornando **a nova oferta** sujeita ao pós-jogo. Comprar, curar, fabricar e ouvir uma fala ambiental continuam disponíveis. Se o quadro anuncia o trabalho de Bram, aponta para `npc_blacksmith`; o pagamento de `city_stone` continua no ferreiro.

Após os créditos, separar **elegível para descoberta** de **já anunciado no diário**. Na primeira conversa ou leitura de um aviso elegível, guardar a descoberta da local. Até aí mostrar o habitante e os serviços habituais. O anúncio aponta para quem entrega o trabalho. Descobrir ou aceitar uma secundária mantém o alvo escolhido pelo jogador até este pedir para a seguir. Preservar esta distinção no quadro, marcadores, ofertas por texto livre e diário. Se o jogador voltar a escolher uma das regiões durante o primeiro capítulo, a mudança de foco não altera as marcas de guardião.

Implementar uma consulta pura de apresentação no jogo Rust, com o catálogo, a quest consultada e o KV. Distinguir «oculta», «oferecida», «em curso», «pronta» e «concluída». Expor o resultado às entradas Luau e usá-lo em `quest_accept`. Em M06 verificar também as condições na entrega, incluindo o caminho de relato. A seta pode indicar uma acção de campanha mesmo quando a missão-mãe já está `ready` por causa de abates precoces. Na ponte de Rust para Luau há chamadas que lêem só um subconjunto de chaves via `read_keys`. Alargar essas leituras e a projecção da UI para que a regra receba a etapa, a escolha e o estado de pós-jogo tanto no jogo como no simulador.

### Interacções, autoria e confirmação

Para as acções de desenho, alinhamento e transporte, propor um tipo adicional de objectivo, `interact`. O `target` identifica a acção autorada (por exemplo, `fold_confirm`). O actor na cena envia a intenção. A regra verifica quest activa, etapa esperada, âncora válida e escolha permitida. Uma acção válida pode gravar um valor e usar o avanço já existente de `quest_set_progress`. O valor é o **progresso absoluto da etapa**. Repetir o mesmo input depois de mudar de etapa deve falhar por alvo/etapa diferentes. Acrescentar o tipo ao validador, ao editor, ao waypoint e aos passos do simulador ao implementar o primeiro uso. Eventos de visita, recolha e abate continuam nos caminhos actuais.

Quando a última interacção já é a entrega (a criança que encontrou o portão, por exemplo), fechar a quest naquele gesto, com a transição `active → ready → done` e a recompensa do catálogo uma única vez. Aplicar escolha, progresso e fecho no mesmo comando antes de apresentar som, movimento ou pedir autosave. Para um duplo clique, reenvio ao carregar ou saída da cena, derivar a pose final e a inscrição dos factos confirmados. Ao carregar, reapresentar a acção pendente; as recompensas já recebidas mantêm-se no estado guardado.

**Primeiro contrato de cena, M01a.** Propor duas etapas de objectivo com IDs de acção distintos. A assinatura e a confirmação podem ocorrer em momentos diferentes, com um load entre elas.

| Etapa | Quem a produz | Verificações e escrita |
|---|---|---|
| 1 · `fold_sign` | Interacção de assinatura no primeiro marco, ou leitura de uma assinatura anterior ao activar a etapa | Conferir no registo de marcos a assinatura válida. Promover apenas a etapa 1. A proximidade do poste, sozinha, deixa o desenho por fazer. |
| 2 · `fold_confirm` | Criança no portão, quando acaba o ensaio do caminho | Validar a orientação e a legenda escolhida. Gravar `campaign.fold` e concluir. Uma direcção errada leva ao curral e volta à etapa 2. Uma segunda confirmação na mesma frame encontra a quest entregue. |

No Jackdaw, atribuir nomes estáveis à criança, ao poste de orientação e aos destinos do ensaio (portão/curral). Verificar no terreno a linha de vista para o telhado e para a esquina da cerca antes de fixar as poses. Ainda se pode mudar o marco de referência na autoria. O teste em cena deve usar o nome referenciado no conteúdo. Na tradução, distinguir `fold_sign` («Assina o marco») de `fold_confirm` («Mostra o caminho de volta»); manter os dois rótulos disponíveis na navegação por comando.

## 3. Fichas de implementação das 17 missões

Na coluna ID, **base** identifica uma quest existente. Os outros IDs são propostos. As âncoras em linguagem corrente são alvos de autoria a posicionar no Jackdaw. Cada linha indica a dependência, a interacção a ligar e o resultado observável que permite aceitar a entrega.

| Missão e ID | Entrada e local | Etapas técnicas | Saída e prova de aceitação |
|---|---|---|---|
| M01 · O contrato dobrado · `story_notas` (base, aceitação) | Novo Jogo, `npc_elder` no salão | Abrir contrato, escolher uma das quatro terras, confirmar. Fixar preferência e aceitar a missão-mãe. Cancelar deixa a escolha editável. | M01 aparece concluída e M01a em foco. Reabrir Edmar não reinicia o contrato. |
| M01a · A primeira dobra · `story_first_fold` | M01 aceite, criança e poste ao portão | Confirmar assinatura de um marco válido, orientar folha, experimentar legenda, acompanhar a criança até à bifurcação. Confirmar regresso correcto. | Guardar `roof` ou `wall`. Um erro leva ao ensaio da folha. Uma assinatura anterior conta. O nome e a seta passam ao destino escolhido ou a M06 se já houver quatro Notas. |
| M02 · A cantiga de Vessa · `forest_witch` (base) | Depois da dobra, `npc_forest_witch` | Fitas ou eco convergem na mesma clareira. Guardar observação dos sulcos se examinados. Resolver o abate de `witch` pelo evento existente. | Uma Nota do Bosque. Vessa muda de verso. M07 usa a versão com desenho quando a observação existe. |
| M03 · O peso de uma carroça · `desert_worm` (base) | Depois da dobra, `npc_desert_worm` | Inspecção da Caravana Encalhada revela o manifesto. Usar tremor visível da areia na aproximação. Abate de `sand-worm`. | Uma Nota do Ermo. A carroça pode partir na próxima visita. O pagamento de Kadi é independente da leitura do manifesto. |
| M04 · O lugar de quem falta · `swamp_bogwarden` (base) | Depois da dobra, `npc_swamp_bogwarden` | Alinhar soleira e covas por terra ou de barco. Reusar arena e abate de `bog-warden`. | Uma Nota do Lodo. A tigela volta à soleira. M11 fornece a medida no próprio local a quem já tinha abatido o chefe. |
| M05 · Quando a serra se cala · `peaks_ogre` (base) | Depois da dobra, `npc_peaks_ogre` | Orientar para o sector habitado, deixar o mecanismo dos covis fazer descer o Ogro, tratar o abate de `boss`. O verso do mojão é examinável. | Uma Nota dos Picos. O posto muda de rotina. Esta terra pode ser a primeira ou a última. |
| M06 · Quatro lugares à mesa · `story_notas` (base, entrega) | Dobra concluída, quatro guardiões, mesa de Edmar | Sincronizar recolha e entregar por uma única chamada. Preservar consumo, pagamento e cinemática `epilogo`. | Contabilidade da base uma única vez. Fechar ou saltar a cena abre o sinal na praça. O diário aponta a M06a. |
| M06a · A colher e o sino · `story_call_square` | M06, praça, Bram e Veterano | Escolher qual objecto travar. Mostrar a vibração do outro. Rodar a tigela para o Campo e validar direcção por orientação discreta. | Traço do Campo na Nota e Veterano pronto a partir. Testar cada primeira escolha, incluindo com o som a zero. |
| M07 · A coluna anda de lado · `story_column` | Sinal localizado, Campo e `npc_battle_survivor` | Usar ecos de cena independentes dos inimigos locais. Activar o corredor entre o par em marcha. Ambos pousam a carga. Interagir com a fivela. | A fivela é prova narrativa. O Campo já visitado dá fala alternativa, inclusive com `citadel_shades` entregue. |
| M08 · A outra margem · `story_stretcher` | M07, vau e Veterano | Levantar, trocar pega, escolher dois pousos e chegar ao abrigo. Usar apoios BSN identificados e deslocações curtas do par. Pouso inseguro repõe o último apoio. | Associar o pano de Ada ao diário ao inspeccioná-lo no abrigo. Recarregar no segundo pouso repõe ambos os actores ali e mantém o lado da pega. |
| M09 · A folha raspada · `story_erased_map` | Pano encontrado, portão e arquivo da Cidadela | Copiar marca da oficina, ler o pedido de Rui, rodar planta e confirmar decalque quando coincide. Permitir desfazer a disposição provisória. | Bilhete e cópia consultáveis. M10 abre no salão. Testar sair do arquivo antes e depois de confirmar. |
| M10 · O direito à margem · `story_hearing` | M09, mesa com Edmar, Onno e Kadi | Apresentar pano ou cópia primeiro. Ambas as ordens revelam a confissão. Pré-visualizar a acta ou a lista selada e confirmar por texto/opção. | Uma escolha de memória persistente. Reabrir o salão permite consultar a prova. O diário envia ao Veterano com o bilhete. |
| M10a · Duas pancadas para Ada · `story_gate_reply` | M10, guarita já descoberta | Levar o bilhete ao portão. Aceitar dois toques ou selecção da figura equivalente. Acender a lanterna na margem militar e comparar a direcção com o mapa. | A resposta errada prova que o sinal precisa das duas margens. Repetir toques fora do padrão deixa a leitura disponível. |
| M11 · Duas pontas do mesmo fio · `story_two_banks` | M10a, postos de Onno e Sig | Confirmar régua na margem das casas e pano alinhado na margem da estrada. Gravar cada impressão por identidade. Contar lados distintos. | O mesmo lado, mesmo visitado duas vezes, vale uma impressão. Ambas as ordens e um load entre elas produzem «CHEGOU» e abrem M12. |
| M12 · O caminho que sobe de volta · `story_return_climb` | Fio completo, `npc_pilgrim_master` e Alto | Conduzir a resposta por `vael-way-1`, `vael-way-4`, `vael-choir-2`, `vael-watch` e `vael-rim-cairn`. Confirmar escuta/alinhamento em cada estação e receber a cunha. | Retomar na última estação confirmada. A visita do levantamento local não substitui o transporte do fio. |
| M13 · A última chamada · `story_last_call` | M12, estação de resposta | Assentar cunha, orientar partida e chegada, escolher quem fala, ensaiar as duas inscrições, confirmar o acesso. | Chamamento encerrado. Acesso público mostra a régua no mapa. Acesso guiado aponta para os postos e oferece a travessia. O memorial continua a ler a escolha de M10. |
| M14 · O mapa à altura de uma criança · `story_handover` | M13, poço e mapa de Discórdia | Projectar a combinação memória × acesso, recuperar a legenda de M01a, segurar a folha e confirmar a correcção da criança. | Guardar antes dos créditos. Uma única retoma abre «Pedidos do Vale» e o anúncio de Bram. Testar as quatro combinações e um segundo load. |

## 4. Estado persistente e migração

Conservar `q.<id>` e os campos existentes de progresso, `tracked_quest`, `story_prologo`, `story_epilogo` e as marcas `nota_witch`, `nota_sand-worm`, `nota_bog-warden`, `nota_boss`. A contagem de Notas recuperadas vem dessas quatro marcas. A quantidade física de `nota` no cofre serve a entrega e pode ser zero depois dela. Manter como factos de campanha apenas dados que não se consigam reconstruir do estado das quests. Exemplos de chaves novas, todas primitivas:

| Chave proposta | Valores | Momento de escrita e leitor |
|---|---|---|
| `campaign.region`, `campaign.fold` | ID de uma das quatro quests regionais; `roof` ou `wall` (`wall` é a esquina da cerca; o id fica por causa dos saves) | Confirmação no contrato e no portão. Seguimento da primeira região e folha mostrada no fim. A região pode mudar enquanto se procuram Notas. |
| `campaign.seen.<pista>` | booleano por vestígio opcional | Inspecção dos sulcos, manifesto, soleira ou seta. As provas obrigatórias da fivela, pano e cópia podem ser derivadas de M07–M09 entregues. |
| `campaign.stretcher.rest`, `campaign.stretcher.grip` | `start`, `rest1`, `rest2`, `shelter`; `original` ou `swapped` | Confirmar pouso ou troca. O posicionamento no retorno usa o identificador do apoio e as poses autoradas do par. |
| `campaign.discovered.<id>` | booleano por pedido local | Primeiro anúncio válido após os créditos. Uma local em curso ou entregue num save antigo já conta como descoberta. |
| `campaign.bank.houses`, `campaign.bank.road` | booleanos | Interacção válida na respectiva margem. A contagem do objectivo é a soma dos dois lados. |
| `campaign.memory`, `campaign.access` | `signed` ou `sealed`; `public` ou `guided` | Apenas após confirmação em M10 ou M13. As vistas e as entidades do vau lêem estes valores ao entrar na cena. |
| `campaign.credits` | ausente, `pending`, `returned` | M14 confirmada → `pending` antes de abrir os créditos de campanha. Terminar ou saltar → `returned` e autosave. A oferta local exige M14 entregue e `returned`. |

Usar o progresso da etapa para o último ponto de escuta de M12. Conservar como valores de cena apenas poses em animação, foco temporário, selecção a pré-visualizar e controlos mantidos. Se uma decisão altera uma fala futura (como quem respondeu no Alto), gravar a variante no mesmo comando. Se serve apenas a fala corrente, pode terminar com a cena. Localização por chaves PT/EN e IDs estáveis para decisões, com escolha navegável por comando em todos os passos obrigatórios.

**Reconciliação no carregamento.** Executar depois de restaurar o KV, antes de publicar HUD e ofertas. Torná-la idempotente e testar duas execuções consecutivas.

| Save de entrada | Resultado esperado |
|---|---|
| Antigo, contrato por aceitar, dois guardiões vencidos | Preservar marcas e inventário. Edmar reconhece dois. M01a continua jogável. Ao escolher uma região já resolvida, o respectivo dador reconhece a vitória. |
| Contrato activo/pronto, quatro marcas e quatro itens | Manter os itens. Mostrar a dobra se faltar. Confirmar a entrega apenas depois da dobra, incluindo pelo relato de Edmar. |
| Contrato entregue antes da expansão | Entrar em M06a quando o epílogo existente tiver terminado. Se a marca da cena ainda estiver pendente, respeitar o controlador actual. Saltar a dobra histórica e mostrar a versão do telhado aprendido com a mãe em M14. Não voltar a creditar objectos ou pagamentos. |
| Local aceite antes da mudança | Mostrar e permitir entregar a local já em curso. Para repetíveis, cada novo aceite volta a consultar a disponibilidade. Preservar marcos/romaria concluídos e a disponibilidade da loja. |
| Load no Alto ou entre M14 e o regresso | Projectar escolhas já confirmadas. `pending` permite continuar ou saltar os créditos. `returned` restaura o mundo e o quadro sem repetir a correcção. |

## 5. Primeira entrega pronta para desenvolvimento

| Ordem | Tarefa e dependência | Artefacto de implementação | Demonstração que encerra a tarefa |
|---|---|---|---|
| 1 | Regra de oferta e catálogo, antes da UI | Em `content`, descrever e validar a tabela. Em `src/quest.rs`, calcular visibilidade, autorização de aceite e prioridade de campanha. Cobrir M01/M06 como vistas distintas. | Um teste puro mostra Edmar como única oferta inicial. Outro mantém uma secundária antiga entregável e recusa uma nova. |
| 2 | Projectar a regra nas entradas, após 1 | Ligar consulta ao diário, cartão, seta, marcadores, quadro, diálogos genéricos e especiais. Dar precedência à conversa de história de Bram e manter a compra disponível. | Novo Jogo, aproximação a Bram e ao quadro. Um pedido oculto também é recusado pela chamada directa. Uma conversa ambiental continua a funcionar. |
| 3 | Autorar M01a, após 1 | Novo JSON de quest incluído no índice, conteúdo de interacção, figura da criança, poste, marcador, dois pontos de chegada e texto PT/EN. Criar no Jackdaw uma cena QA pequena com as mesmas entidades de interacção. | Falhar a orientação e experimentar novamente. Confirmar cada legenda, com teclado e comando; o checkpoint e a assinatura sobrevivem à saída para o menu. |
| 4 | Ligar contrato, dobra e região, após 2–3 | Tratar o aceite de Edmar, preferência regional, assinatura preexistente e alvo seguinte. Pôr o resultado de domínio antes do efeito audiovisual e do pedido de save. | Sem flags de debug, Novo Jogo → Edmar → dobra → Vessa (ou outra região escolhida). Repetir com quatro Notas precoces encaminha para Edmar. |
| 5 | Validar a fatia no mundo e a retoma, após 4 | Cenários para começo limpo, dobra interrompida e save legado; passagem no mundo principal além da cena de QA. | Guardar, sair e continuar na mesma bifurcação. Nenhum marcador local antecipado, uma assinatura, uma legenda e um único alvo de campanha. |

**Exemplo de fixture proposta para o simulador**, a adicionar só quando a quest e o aceite de M01a existirem. Exercita a fronteira de oferta. A posição e a mão da criança são verificadas numa cena viva separada. O campo `steps` usa operações que o simulador já conhece.

```json
{
  "description": "Contrato aceite permite iniciar a dobra sem activar o pedido de Bram",
  "extends": ["_base"],
  "scene": "qa/sandbox",
  "flags": {"q.story_notas": "active"},
  "steps": [
    {"accept": "story_first_fold"}
  ],
  "expect": {"quests": {"city_stone": "not_taken", "story_first_fold": "active"}}
}
```

O simulador actual devolve erro se um passo `accept` for recusado. Nesta fixture, `not_taken` verifica que Bram não foi activado. Testar a recusa separadamente no Rust: com o mesmo KV, a consulta deve devolver «oculta», `quest_accept("city_stone")` deve devolver falso e o KV deve permanecer igual. Na cena viva, verificar também que o quadro não mostra o convite. Para os passos físicos acrescentar um passo de QA que passe pelo mesmo comando validado do jogo, com alvo e escolha. Forçar `progress` serve para preparar estados, mas não demonstra que a interação real funciona.

## 6. Ordem das entregas seguintes e critérios de integração

| Entrega | Dependências e trabalho | Prova antes de integrar |
|---|---|---|
| Guardiões e mesa (M02–M06) | Primeira entrega aceite. Ligar pistas ao terreno, reacções por 1/2/3/4 Notas, reconhecimento de abate prévio e entrega única. | No teste de regras, percorrer as 24 ordens e repetir um abate em cada percurso. No mundo real, experimentar cada guardião como primeiro e entregar a quarta Nota com e sem pagamentos locais pendentes. |
| Fatia da maca (M06a–M08) | Disponibilidade após `story_notas`. Cena compacta com colher, sino, duas sombras, fivela, Veterano e apoios. Primeiro prototipar colisões e reposição do par no vau. | Sem áudio, localizar a chamada. Abandonar a maca em cada pouso, entrar num interior, carregar e concluir. A fivela continua única e o jogador consegue explicar quem os soldados levavam. |
| Arquivo e audiência (M09–M10a) | Pano de Ada confirmado; decalque, bilhete, duas ordens de prova, duas decisões de memória, duas formas de bater à porta. | Cancelar e reabrir cada pré-visualização. Guardar imediatamente após confirmar. O memorial, a próxima missão e a resposta na guarita concordam. |
| Duas margens e Alto (M11–M13) | Resposta falhada no portão. Autorar referências dos postos e das cinco estações. Montar a ligação pública e a travessia guiada. | Testar casas/estrada em ambas as ordens, voltar à mesma margem e carregar entre elas. Para cada acesso, atravessar efectivamente o vau e observar o habitante que responde. |
| Mapa, créditos e pedidos (M14 e pós-jogo) | Ambas as decisões disponíveis; correcção, variante da primeira dobra, créditos de campanha, quadro e descoberta de ofertas locais. | Quatro combinações de memória/acesso × créditos completos/saltados. Carregar antes e depois da saída. Cada caso abre a oferta de Bram apenas no regresso. Quem termina a local de Vessa continua a receber a cadeia certa. |

Entregar o desenvolvimento por fatias reproduzíveis com fixtures. Activar a nova política de pós-jogo numa versão distribuída quando existir um caminho alcançável até M14. Na build de desenvolvimento da primeira entrega já se exercita a política completa, com fixture de M14 para testar o outro lado. Isto evita publicar um intervalo em que os jogadores perdem acesso a todas as novas secundárias.

**Verificação da implementação.** Correr `cargo test -p vael-content --lib` para o esquema, `cargo test -p vael --lib` para regras, projecções da UI e save, e `cargo test --release --test qa_scenarios` para os percursos de dados. Validar os módulos de mundo com `vael-tools analyze world.bsn` e `vael-tools lint`. Usar `python3 tools/qa.py run <cenário>` para as acções em cena e fazer pelo menos uma entrada por **Novo Jogo**. A entrada `dev` salta o prólogo. As tarefas de produção e os novos cenários acima continuam propostos até serem implementados e testados no jogo.
