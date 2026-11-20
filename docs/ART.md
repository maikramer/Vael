# Direção de Arte e Identidade Visual — O Silêncio de Vael

Identidade: **Ofício Talhado & Iluminura Rural**.
O Vale é visto pelos olhos de um **Cartógrafo**. Tudo o que é humano neste mundo foi desbastado, erguido, cerzido ou medido com ferramentas de trabalho; tudo o que é extraordinário vem da ressonância mineral do **Coro** (os veios de cristal azul do Alto) ou da ferida que o seu silêncio deixou na terra.

Este documento é a referência canónica para autoria no Jackdaw, escrita de manifests em `assets/manifests/*.yaml`, configuração do perfil `assets/game.yaml`, materiais procedurais (`bevy_ext_gradmesh`), iluminação (`bevy_elements` / `bevy_lights`) e interface (`ui/*.xaml`). A visão de design de jogo vive no [`../GDD.md`](../GDD.md#9-direcção-artística-e-sonora).

---

## 1. Os três pilares plásticos

Para que qualquer captura de ecrã seja imediatamente reconhecível como *O Silêncio de Vael* — e nunca confundida com geração genérica de IA ou fantasia móvel — a imagem combina três camadas plásticas com papéis distintos:

| Camada | Onde atua | Linguagem plástica | O que evita (anti-IA) |
|---|---|---|---|
| **1. Volume Talhado** | Geometria 3D (GLBs, primitivas `<Composition>`, terreno `.ahgt` e pavimento) | Planos facetados e chanfros francos: madeira desbastada a enxó, pedra aparelhada com cal, ferro batido em faces planas, dobras de tecido espesso. A silhueta lê-se contra o céu antes de qualquer textura. | O aspeto «sabonete derretido» ou inchado típico de malhas extraídas por Marching Cubes / Image-to-3D sem arestas estruturais. |
| **2. Guache e Têmpera Mate** | Albedo, materiais PBR (`paint3d`), relva (`bevy_ext_grass`) e flora (`bevy_flora`) | Pigmentos minerais e vegetais opacos (`roughness` 0.78–0.95), organizados em **blocos largos de cor limpa** com transições de valor suaves e desgaste concentrado apenas onde há uso real (pegas, bainhas, soleiras). | Brilho plástico/resina, saturação néon uniforme e micro-ruído/filigranas que o `xatlas` rasga em costuras UV visíveis. |
| **3. Gravura, Carvão e Pergaminho** | Interface (`ui/*.xaml`), mapa (`ui/vael_map.png`), ícones (`assets/talent-icons.json`) e decalques da campanha | Traço de pena e xilogravura, carvão friccionado sobre papel, tinta sépia/marfim e ouro velho oxidado sobre ardósia ou pergaminho escuro. | HUDs modernos de vidro néon, ícones «flat design» genéricos ou ilustrações digitais com aerógrafo macio. |

---

## 2. Decálogo Anti-IA — diagnóstico e regras de produção

Modelos generativos 2D/3D tendem para um ponto médio estatístico («fantasia cartoon brilhante com ornamentos simétricos borrados»). No Vael, cada vício desse ponto médio é bloqueado por uma regra de arte e de pipeline:

| Vício visual de IA (Proibido) | Causa no pipeline | Regra de Vael (Obrigatório) |
|---|---|---|
| **Superfície plástica ou envernizada** | Reflexos especulares falsos pintados na imagem 2D ou `roughness` baixo em materiais orgânicos | Acabamento mate de guache/têmpera. Especularidade alta (`roughness < 0.35`) é exclusiva da **água**, do **cristal do Coro** e do **gume de ferramentas afiadas**. |
| **Arestas «derretidas» em arquitetura e madeira** | Reconstrução 3D suaviza cantos de vigas, degraus e cantaria | Pedir sempre `chamfered planes`, `hand-hewn timber with crisp flat facets`, `squared dressed stone`. Para arquitetura modular, escadas, pilares, barris, muros e réguas, preferir primitivas `<Composition>` (`bevy_ext_gradmesh`). |
| **Mosaico e costuras nas texturas (UV seams)** | Padrões de alta frequência (listras, escamas miúdas, filigrana) cortados pelas ilhas do `xatlas` ou `Materialize` sobre atlas | Desenhar em **zonas largas de cor sólida por material** (madeira lisa com veio largo, pano liso com barra grossa). Manter `lod.patch_pbr: false` e `paint3d.pbr_enrich: false` no `game.yaml`. |
| **Ornamentação sem função («barroco de ruído»)** | Prompts com `ornate`, `magical runes`, `intricate details` geram relevos indecifráveis | **Zero ornamento gratuito.** Cada detalhe visível tem função mecânica ou narrativa: cravos de ferro numa porta, corda de amarração, remendo no joelho, entalhe de medida numa régua. |
| **Simetria morta de manequim** | Geração frontal sem história de uso | **Assimetria de ofício**: desgaste num só lado da pega (M08), bolsa de mapas a tiracolo, manto preso num ombro para libertar o braço de trabalho, lama acumulada apenas na base. |
| **Paleta «doces / parque temático»** | Legado de prompts `bright cheerful cartoon, vibrant colors` | Paleta de **pigmentos de terra, cal, ardósia e pinho**. A saturação máxima é reservada para o **Azul do Coro** (sinal acústico/mágico) e o **Âmbar da Chama** (abrigo/fogueira). |
| **Proporções inconsistentes (Chibi vs. Adulto)** | Mistura de prompts `cartoon boy`, `chibi shopkeeper` e presets dispares | Padrão único de **Estilização de Ofício (6 a 6,5 cabeças)** para todo o elenco humano. A diversidade vem da compleição (tronco largo, esguio, curvado) e nunca de cabeças gigantes. |
| **Fumo, fogo ou magia esculpidos como massa sólida** | O Image-to-3D converte plumas de fumo ou chamas da imagem em geometria opaca | Chaminés, tochas, forjas e braseiros são **sempre gerados frios e vazios** no GLB. O fogo, as brasas e o fumo entram exclusivamente em runtime via `bevy_lights` (`src/lights.rs`) e `ParticleSpec`. |

---

## 3. Personagens e criaturas — Estilização de Ofício

### 3.1. Proporções e anatomia (6 a 6,5 cabeças)

Os personagens humanos de Vael não são bonecos *chibi* nem manequins fotorealistas. Têm **proporções de ofício (6 a 6,5 cabeças de altura)**, pensadas para dar peso físico real às animações e ao sistema de corpo ativo (`bevy_ext_puppet` em `src/puppet.rs`):

- **Cabeça e rosto:** planos faciais esculpidos (arcada supraciliar, cana do nariz, maçãs do rosto e maxilar definidos por facetas suaves). O rosto lê-se pela luz do sol e da lanterna a 8 metros de distância, sem depender de micro-textura de pele.
- **Mãos, antebraços e calçado (+10% a 15% de volume):** ligeiramente enfatizados. O jogo enquadra mãos a segurar as pegas da maca (M08), a passar o carvão na planta (M09), a afiar a raspadeira ou a tocar na palavra gravada no Alto (M13). Mãos demasiado pequenas ou delicadas desaparecem na câmara de 3.ª pessoa e fundem dedos no bake 3D.
- **Postura e centro de massa:** ombros e bacia com leitura clara para que os desequilíbrios, passos de captura (`CatchFall`), bloqueios de escudo e esbarrões (`ActorBump`) comuniquem força e fadiga através da silhueta.

### 3.2. Regra das três camadas de vestuário

Cada personagem humano veste até três blocos cromáticos de leitura imediata:

1. **Camada base (Linho, lã cardada, cânhamo):** tons de cru, aveia, cinza-pedra ou barro claro.
2. **Camada de proteção / trabalho (Couro curtido, avental grosso, gibão acolchoado, manto de viagem):** define a ocupação e dá a cor secundária da região. Sem armaduras de fantasia com ombreiras espinhosas ou ouro polido — a defesa no Vale é couro fervido, ferro batido na forja de Bram e madeira ferrada.
3. **Ponto focal / ferramenta (O que a pessoa faz):** um único elemento de alto contraste que identifica o personagem à distância.

| Personagem / Grupo | Silhueta e marca de ofício | Paleta dominante |
|---|---|---|
| **O Cartógrafo (Herói)** | Túnica de couro macio com mangas arregaçadas, manto curto assimétrico que liberta o braço direito, bolsa/estojo de mapas cilíndrico às costas e **lanterna de cintura** na anca esquerda (`spine_01`). Botas de marcha escuras da lama até meia canela. | Couro castanho-seco (`#6E4E32`), linho cru (`#D8CEBE`), manto verde-musgo fechado (`#3A4F41`) e o brilho quente da lanterna (`#E7C987`). |
| **Ancião Edmar** | Manto comprido de lã pesada do salão comunal, ombros largos mas ligeiramente curvados pelo tempo, cinto com chaves e a **raspadeira de ferro com cabo polido pelo uso diário**. | Cinza-ardósia (`#4A525A`), ocre escuro (`#8C6D46`) e branco-cal nos cabelos/barba. |
| **Bram, o Ferreiro** | Tronco largo, avental pesado de couro chamuscado sobre camisa arregaçada, luva ou braçadeira de forja num só braço, fuligem nas bordas. | Couro queimado (`#3D2B22`), ferro escuro (`#363A3E`) e reflexo brasa (`#D9774F`). |
| **Irmã Vessa (Bosque)** | Capuz e capa de vigia cortada em bicos largos como folhagem de pinheiro, fitas de pano carmim/ocre atadas ao cinto (as mesmas que marcam as árvores da Floresta Sombria). | Verde-pinho profundo (`#263829`), pardo (`#5C4B3B`) e carmim oxidado (`#9E4738`). |
| **Kadi (Ermo Rubro)** | Roupa de caravana em camadas leves mas fechadas contra a areia, lenço de poeira ao pescoço, correias de carga e cantil de couro. | Terracota (`#A85232`), areia crua (`#C9B18B`) e azul-índigo desbotado (`#3B4F63`). |
| **Onno (Pântano / Lodo)** | Capote encerado impermeável, botas altas de vau, corda enrolada ao ombro e a vara comprida marcada com cotas de água. | Pardo-lodo (`#3D382E`), verde-junco (`#5A6346`) e osso/cal nas marcas da vara (`#E6DEC8`). |
| **Batedora Sig (Picos)** | Pelica de lã grossa e couro nas costas, perneiras de neve, aljava/equipamento de montanha compacto contra o vento. | Azul-granito (`#4A5866`), lã crua (`#D4CFC4`) e couro escuro (`#47362B`). |
| **Sombras do Campo de Batalha** | Não são espectros néon translúcidos: são figuras com a densidade de basalto molhado e carvão, inclinadas no gesto coletivo de quem transportava peso a dois. As mãos e os antebraços são a parte mais nítida do corpo. | Carvão profundo (`#1C2024`), cinza de cinzas (`#6E7378`) e um fio subtil de azul-Coro apagado (`#4C7A82`) nas mãos. |

### 3.3. Os Quatro Guardiões — a ferida da Nota

Cada chefe engoliu uma das quatro Notas quando a última cantora morreu, e cresceu deformado pelo pedaço de canto que guarda. Visualmente, **todos os quatro partilham uma assinatura anatómica**: veios ou lascas do **cristal azul-turquesa do Coro** incrustados na sua matéria natural, que se iluminam e vibram quando preparam o ataque especial (cumprindo o Pilar 2 do GDD: *«Um perigo ensina antes de cobrar»*):

1. **A Bruxa do Bosque:** manto de casca de pinheiro negro, raízes torcidas e líquen pálido; na garganta e no cajado, lascas de cristal azul vibram antes de fixar a marca de maldição no solo.
2. **O Verme do Ermo:** placas quitinosas facetadas como arenito vermelho lascado e ferro oxidado (nunca um tubo liso genérico); entre os anéis da cabeça brilha a crista azul da Nota do Ermo quando a areia treme antes da emersão.
3. **O Guardião do Lodo:** massa colossal de turfa, raízes de mangue, destroços de barcos (tábuas e cordame) e lama escura em planos pesados; no peito aberto repousa o núcleo cristalino azul envolto em algas.
4. **O Ogro dos Picos:** silhueta monolítica de granito gelado, pelagem hirsuta branca-acinzentada e punhos facetados como blocos de pedreira; veios de cristal azul correm-lhe pelos ombros e punhos antes de romper o chão.

---

## 4. Paleta cromática e linguagem de materiais por região

O mundo proíbe cores primárias puras nos materiais naturais. A paleta baseia-se em **pigmentos minerais e terras** (cal, ocre, siena, sombra queimada, ardósia, verdete), garantindo que os dois sinais de jogabilidade — o **Âmbar das Fogueiras/Lanterna** (calor, rota segura, regresso) e o **Azul do Coro** (as Notas, marcos assinados, ressonância) — saltem à vista em qualquer bioma.

| Região | Pigmentos de superfície (sRGB ref.) | Materiais dominantes | Luz, névoa e atmosfera (`bevy_elements`) | Sinal visual / contraste |
|---|---|---|---|---|
| **Discórdia e Vale Central** | Cal (`#E6DEC8`), Ardósia (`#4E565E`), Carvalho (`#7A5C3A`), Prado (`#3D6B2C`) | Pedra rebocada a cal gasta na base, vigas de carvalho talhado, telha de ardósia, pano cerzido, calçada em grelha (`paving="1"`). | Luz solar temperada, céu amplo com sombras de nuvem a correr no prado (`VAEL_SURFACE`). À noite, piscina quente das 24 luzes do `bevy_lights`. | Ouro velho e brasa (`#E7C987` / `#D9774F`) nas janelas, forja e lanternas; marcos com entalhe turquesa. |
| **Floresta Sombria (Bosque)** | Pinho Negro (`#232B24`), Musgo (`#2F4F38`), Turfa (`#3B2F26`) | Troncos altos escuros (`pine-dark`), raízes expostas, pedras cobertas de musgo, madeira húmida. | Copa fechada, névoa azul-esverdeada curta que esconde o horizonte, feixes de luz fria. | Fitas de pano carmim/ocre (`#A64B38`) atadas aos ramos; brilho azul-frio nos menires e na clareira da Bruxa. |
| **Ermo Rubro** | Ferrugem/Arenito (`#A85232`), Ocre Seco (`#C98A4B`), Osso/Madeira Sol (`#C2B39B`) | Rocha estratificada em degraus secos, madeira rachada e branqueada pelo sol, ferro oxidado, areia grossa. | Luz dura e alta, sombras compridas de tom azul-ardósia (`#3B4954`), poeira quente no horizonte. | Contraste entre a areia clara/ocre e a madeira escura das carroças encalhadas; cristal azul nos obeliscos. |
| **Pântano da Bruma (Lodo)** | Lodo (`#2C3028`), Alcatrão/Madeira (`#3D352E`), Junco (`#8C7A4B`) | Tábuas encharcadas, barcos partidos, pedra limosa, água turva com reflexo rasante. | Visibilidade baixa, bruma húmida cinza-oliva (`#6E7D71`), chão permanentemente molhado (`wetness` alto nos sulcos). | **A linha da água corta objetos humanos reconhecíveis** (ombreiras de portas, régua de pedra com marcas brancas de cal). |
| **Picos Gelados** | Granito Frio (`#545E69`), Neve do Manto (`#CCD4E0`), Pinho de Abrigo (`#47362B`) | Rocha lascada em arestas vivas, neve acumulada por cota e inclinação (`VAEL_SNOW`), estruturas de trave grossa nos postos. | Ar cristalino ou nevasca (`Snow`/`Blizzard`), sombras azuis profundas na neve, vento alto nas copas. | Silhueta escura dos três mojões e do gesto do Ogro recortada contra o céu/neve; brilho ciano nas bocas da mina. |
| **Campo e Cidadela Caída** | Pedra Calcinada (`#5A5854`), Barro (`#4D4237`), Ferro Velho (`#6B3E2E`) | Cantaria tombada em blocos geométricos, estandartes desbotados, macas de madeira gasta, mesas de pedra para decalque. | Luz oblíqua e silenciosa, neblina baixa rente ao barro revirado. | O preto seco do carvão sobre o papel claro nas mesas de cópia e os pares de sombras imóveis. |
| **Vau e Alto de Vael** | Basalto (`#38404A`), Líquen Dourado (`#9E8C4A`), Cristal do Coro (`#38B6C4` → `#A3F7FF`) | Pedra de romaria talhada com sulcos acústicos, réguas de margem, colunas de cristal azul com veios de intervalos distintos. | No Vau, espelho largo da água entre as duas margens; no Alto, horizonte aberto acima das nuvens. | Os **veios com intervalos rítmicos** no cristal azul — que a cópia a carvão reproduz mesmo a preto e branco. |

---

## 5. Narrativa nos objetos — a mão de quem usa

Em *O Silêncio de Vael*, a direção de arte resolve mistérios de design sem precisar de caixas de texto explicativas (Pilar 1 e Secção 9 do GDD):

1. **Desgaste direcional como pista jogável:**
   - Na travessia do vau (M08), a pega da maca está **polida e gasta apenas num dos lados**, indicando onde o Veterano apoiava a mão sã e onde a mão ferida cedia.
   - Na mesa do salão comunal (M09/M10), a **raspadeira de Edmar** tem o gume limpo e o cabo lustroso no meio de ferramentas paradas, mostrando que ele conviveu trinta anos com a tentação de raspar o mapa oficial.
2. **Escala humana como medida de tempo:**
   - Na praça de Discórdia, o mapa final (M14) usa exatamente o mesmo enquadramento arquitetónico da primeira manhã (M01a), mas a moldura de madeira foi **baixada na parede** para ficar à altura dos olhos das crianças.
   - No Pântano e no Vau, as cotas de cheia são lidas porque a água corta uma soleira, uma ombreira ou os entalhes graduados da régua de pedra.
3. **Tradução entre cristal e carvão:**
   - Os veios do cristal do Alto não são um brilho aleatório: têm **espaçamentos largos e estreitos (intervalos musicais visuais)**. Assim, quando o jogador faz o decalque a carvão sobre papel (monocromático) ou joga sem som, reconhece o mesmo motivo pelas distâncias entre os sulcos.

---

## 6. Integração com o motor e arquitetura técnica de arte

A identidade visual tira partido direto dos sistemas nativos do Vael e dos crates `bevy_ext_*`, evitando lutar contra o pipeline:

### 6.1. Primitivas `<Composition>` (`bevy_ext_gradmesh`) vs. Malhas GLB

O sistema de primitivas procedurais ([`PRIMITIVES.md`](PRIMITIVES.md), `src/primitives.rs`) gera geometria limpa com **gradiente vertical neutro assado em vertex colors** (mais escuro na base, ancorando o objeto ao chão) e presets de estilo (`wood`, `stone`, `cobblestone`, `metal`, `cloth`, `flat`).

- **Quando usar `<Composition>` / Primitivas:** muros aparelhados, pilares de ponte (`Lathe`), torres partidas, barris, mesas de cópia, réguas de cheia, estradas/corrimãos (`Sweep`), paredes com vãos (`ExtrudedPolygon`) e marcos geométricos. Garantem arestas chanfradas perfeitas, zero costuras de UV e colisores Avian exatos.
- **Quando usar GLB (`assets/manifests/*.yaml`):** personagens, criaturas, edifícios compostos com telhado/madeiramento talhado, barcos, carroças e props heroicos de ofício.

### 6.2. Terreno, pavimento, flora e clima reativo

- **Pavimento em grelha ([`PAVING.md`](PAVING.md)):** as ruas e praças (`paving="1"`) nunca têm bordas retilíneas artificiais de grelha crua; usam o chanfro nos cantos convexos, filete nos côncavos e ondulação orgânica do rim (`paving-wobble`, default `0.35` do módulo), com camadas de elevação fixas (`DECAL_LIFT 0.040 < PAVING_LIFT 0.064 < ON_PAVING 0.072`).
- **Chão vivo ([`GROUND.md`](GROUND.md)):** o terreno responde fisicamente à jornada do Cartógrafo. As solas (`VAEL_TRAILS`) abrem sulcos na neve que revelam a terra escura por baixo e escurecem/alisam o lodo molhado (`SurfaceWeathering.wetness`). As manchas de sombra de nuvem (`VAEL_SURFACE`) quebram a monotonia das encostas abertas.
- **Relva e árvores ([`FLORA.md`](FLORA.md), `world/settings/grass.bsn`):** as paletas de `grass.bsn` mantêm raízes escuras (`root_color` próximo do solo para fundir a lâmina com o `.ahgt`) e pontas dessaturadas por bioma. As árvores procedurais (`bevy_flora`) partilham vento, humidade e acumulação de neve nas copas com o estado do clima.

### 6.3. Luz, exposição e pós-processamento ([`LIGHTING.md`](LIGHTING.md))

- **Céu físico e interiores honestos:** `bevy_elements` conduz o EV100 exterior pela iluminância real do sol/lua/nuvens. Nos interiores (`Exposure::INDOOR`, EV100 7), o céu e a atmosfera saem da câmara e a cena vive da luz quente de teto/lareira e da sombra de recanto.
- **Hierarquia de fontes luminosas à noite:**
  1. **Fogo prático (`bevy_lights`, `src/lights.rs`):** fogueiras, tochas e braseiros respiram com flicker determinístico e chama billboard (`FlamePlugin`), num pool orçamentado de 24 luzes reais.
  2. **Lanterna do Cartógrafo (`src/lantern.rs`):** pêndulo físico na cintura esquerda (`X` para acender/apagar). Cria uma ilha íntima de cor âmbar que revela planos faciais e texturas de chão, ao custo de atrair sentinelas.
  3. **Cristais do Coro e preenchimento autoral:** os cristais frios (`radius: 5`) ficam fora do pool de fogo — a sua luz é estável, mineral e ciano-turquesa, nunca trémula como uma tocha.
- **Pós-processamento (`world/settings/postfx.bsn`):**
  - `ssao_quality: Medium` (ou `High`) é essencial para marcar os vincos entre planos talhados e o contacto dos props com o chão.
  - `chromatic_aberration: false` **sempre**: aberração cromática é um artefacto de lente fotográfica que destrói a estética de pintura mate e gravura.
  - `motion_blur: false` por omissão para preservar a leitura limpa das poses e silhuetas em combate.

---

## 7. Interface, cartografia e tipografia (`ui/*.xaml`)

Toda a interface de utilizador funciona como uma extensão da **caixa de mapas e do caderno (`a Nota`)** do Cartógrafo:

### 7.1. Paleta canónica de UI (extraída de `ui/title.xaml` e `ui/hud.xaml`)

| Token | Hex (ARGB/sRGB) | Equivalente material | Uso na interface |
|---|---|---|---|
| `BoardDark` / Véu | `#F20B0F0D` / `#CC171D20` | Ardósia de campo / couro escuro engraxado | Fundo de painéis, véu do ecrã de título e cartões de diálogo (legível sob neve ou sol). |
| `Ink` | `#FFF2E8D5` | Pergaminho claro / giz de cal | Texto principal, títulos secundários, leitura de provas e legendas. |
| `Muted` | `#FFB7A98F` | Papel envelhecido / linho seco | Rótulos secundários, coordenadas, notas de contexto e atalhos. |
| `Gold` / `GoldDeep` | `#FFE7C987` / `#FFA8854C` | Folha de ouro velho / latão de compasso | Logótipo, molduras ativas, foco de comando, marcos assinados e brasão. |
| `Leaf` | `#FF8FBF9F` | Verdete / pigmento de salva | Estados positivos, conclusão de etapa, semente de canção e confirmação suave. |
| `Ember` | `#FFD9774F` | Lacre de cera / brasa de fogueira | Avisos de perigo, queimadura (`Brasas`), vida crítica e ações destrutivas. |
| `ChoirCyan` | `#FF5BC0BE` | Veio de cristal do Coro | Indicador do eco de *Contratempo* (anel turquesa na pegada), ressonância das Notas. |

### 7.2. Tipografia, ornamentos e ícones

- **Tipografia:** `Noto Serif Display` para cabeçalhos, nomes de terras, títulos de missão e o logótipo (evoca tipografia de atlas e folha de rosto impressa); fonte de texto limpa e escalável para falas e descrições de talentos, garantindo leitura rápida até duas linhas por legenda.
- **Ornamentação vetorial:** divisórias e emblemas em XAML usam linhas retas finas (`StrokeThickness="1"` a `"1.8"`) rematadas por pequenos losangos de bússola (`M54 2 L58 6 L54 10 L50 6 Z`), nunca molduras barrocas pesadas.
- **Mapa topográfico (`tools/make_map.py` → `ui/vael_map.png`):** renderizado diretamente do `world.data/terrain.ahgt` com sombreamento de relevo (*hillshade*) cartográfico, curvas de cota e aguadas de bioma, sobre o qual assentam as marcações da Nota e a legenda desenhada com a criança em M01a.
- **Medalhões de talento (`assets/talent-icons.json`):** gravuras circulares estilo manuscrito iluminado, com traço fino em sépia e ouro velho, apontamentos em verde-floresta profundo e silhueta central forte, sem texto nem molduras quadradas.
- **Acessibilidade visual por forma (Secção 8 do GDD):** as quatro Notas e as quatro regiões distinguem-se sempre pelo **contorno do entalhe** (além da cor), e os pulsos sonoros (M06a–M13) traduzem-se em cadência geométrica visível para jogar com áudio a zero.

---

## 8. Guia de prompts para o pipeline `AiGameKit` (`assets/`)

Ao editar `assets/game.yaml` ou criar entradas em `assets/manifests/*.yaml`, seguir a receita abaixo para que o `prompt_builder` do `gameassets` produza referências 2D e malhas 3D fiéis à identidade do Vale.

### 8.1. Vocabulário obrigatório vs. proibido

| Categoria | Termos a incluir (`idea:` / `tone:`) | Termos proibidos (ativam viés de IA genérica) |
|---|---|---|
| **Estilo e superfície** | `carved chamfered planes`, `matte gouache and tempera finish`, `broad clean color blocks`, `earthy mineral pigments`, `subtle edge wear` | `cartoon`, `playful`, `vibrant neon`, `glossy`, `photorealistic`, `unreal engine 5 render`, `shiny plastic` |
| **Madeira e arquitetura** | `hand-hewn timber beams with flat facets`, `squared dressed stone with lime mortar wash`, `slate shingles with crisp edges` | `gnarled melted wood`, `striped awning` (quebra no xatlas), `ornate filigree`, `magical glowing runes` |
| **Personagens** | `6.5-head stylized craft proportions`, `sculpted facial planes`, `sturdy working hands`, `layered wool and worn leather`, `functional tool belt` | `chibi`, `bobblehead`, `cute anime`, `oversized head`, `high-fantasy spiked armor`, `glowing eyes` |
| **Objetos de ofício** | `forged wrought iron`, `worn wooden handle`, `hand-stitched thick leather`, `practical rural tool` | `sci-fi`, `intricate engraving noise`, `floating parts`, `translucent glass shards` (exceto cristais do Coro) |

### 8.2. Templates de `idea:` para novos assets

**Personagem humano (A-pose para rig):**
```yaml
idea: >
  stylized rural fantasy craftsperson with 6.5-head proportions and sculpted facial planes,
  wearing layered coarse linen tunic, worn leather work apron, sturdy boots, matte gouache
  and tempera finish with broad clean color blocks and earthy mineral tones, standing upright
  in relaxed A-pose with both arms straight and angled down at 45 degrees away from the body,
  legs straight and shoulder-width apart with clear gap from thighs to ankles, relaxed open
  hands with palms facing thighs, hands empty, no weapons, no props, front-facing
```

**Edifício / estrutura do Vale:**
```yaml
idea: >
  rural medieval stone and hand-hewn timber building with crisp chamfered architectural planes,
  lime-washed squared stone foundation flush to the ground, broad slate roof tiles with clean
  geometric edges, matte tempera painted surfaces in warm grey-ochre and aged oak tones,
  solid closed base, eye-level three-quarter view, no chimney smoke, no ground slab
```

**Prop de ofício / marco cartográfico:**
```yaml
idea: >
  handcrafted rural cartographer landmark or tool with carved chamfered wood and forged iron
  fittings, matte gouache finish, broad clean color zones without micro-patterns, visible functional
  wear on the grip, clear bold silhouette, single isolated object
```

---

## 9. Checklist de revisão artística (Gate de Arte)

Antes de dar um asset, cenário ou ecrã de UI por fechado:

1. **Teste da silhueta a 20 metros:** o objeto ou personagem identifica-se pelo recorte exterior e por 2–3 blocos largos de cor sem precisar de zoom?
2. **Teste anti-plástico:** sob o sol do meio-dia (`VAEL_START_HOUR=12`), a madeira, a pedra e a roupa mantêm acabamento mate de têmpera/guache sem brilhos especulares de plástico?
3. **Teste de costura UV:** o modelo pintado pelo `paint3d` está livre de padrões listrados ou ruído miúdo partido pelas bordas das ilhas do `xatlas`?
4. **Teste de história e uso:** o objeto mostra *como* é usado no Vale (apoio no chão, pega gasta, cal na base, humidade/neve coerente com o `WeatheredMaterial`)?
5. **Teste de hierarquia cromática:** as cores de fundo respeitam a paleta mineral do bioma, deixando o **Azul do Coro** e o **Âmbar da Chama** como os únicos focos saturados da cena?
