# NLU do Vael — GLiNER2 quantizado e motor de inferência

Estudo de quantização e runtime para `ottema/gliner2-ptbr` como camada de
**compreensão de texto** do jogo (não de geração de fala). Tudo o que está aqui
foi medido neste checkout; os scripts reproduzem cada número.

## 1. O que o modelo é (e o que não é)

`gliner2-ptbr` é um **encoder bidirecional** mDeBERTa-v3-base com cabeças de
*span scoring* — 307,1 M parâmetros, `model_type: "extractor"`. Ele **não gera
texto**, portanto não dialoga. O papel dele no jogo é transformar a frase do
jogador em dados:

```
"Quero comprar duas poções de cura do mercador."
→ produto?  (nenhum)   pessoa: mercador   quantidade: duas
→ intenção: comprar (0.9999)
```

Quem responde continua sendo o grafo Lua (`scripts/npc/dialogue.lua`,
`dialogues/*.json`).

### Estrutura que importa para a quantização

| Módulo | Parâmetros | Fração |
|---|---:|---:|
| `word_embeddings` (250 112 × 768) | 192,1 M | **62,5 %** |
| Transformer (12 camadas) | 85,5 M | 27,8 % |
| `span_rep` (cabeça de spans) | 16,5 M | 5,4 % |
| `count_embed` + `count_pred` | 11,8 M | 3,8 % |
| `classifier` (intenção) | 1,2 M | 0,4 % |

O vocabulário multilíngue é o maior componente. Quantizar o transformer sem
podar o vocabulário deixaria o modelo grande de qualquer jeito.

## 2. Benchmark de domínio

Não existe benchmark público de comandos de jogador em PT-BR. Foram criados
dois conjuntos, ambos versionados:

| Conjunto | Frases | Entidades | Uso |
|---|---:|---:|---|
| `bench/domain_test.jsonl` | 140 | 312 | manuscrito, difícil (gíria, frases longas) |
| `bench/synth_test.jsonl` | 491 | 1 497 | gerado por templates, calibração de medição |

Rótulos de entidade: `item`, `npc`, `local`, `criatura`, `quantidade`, `moeda`.
O modelo é *open-vocabulary*; os nomes no prompt são `produto`, `pessoa`,
`local`, `animal`, `quantidade`, `dinheiro` e são mapeados na saída.

**Achado de rótulos:** nomes abstratos (`item`, `npc`) dão F1 0,49; nomes
próximos do treino (`produto`, `pessoa`) dão 0,59 no manuscrito. Verbo como
entidade (`acao`) é inútil (F1 0,04) — intenção vai na cabeça de classificação
e acerta 0,9999 nos exemplos.

## 3. Quantização: o que funciona

Todas as medições usam o mesmo benchmark, pesos apenas (ativações fp32), salvo
indicação.

### 3.1 Poda de vocabulário (maior ganho)

| Vocabulário | Embedding (int8) | F1 synth | F1 hand |
|---|---:|---:|---:|
| 250 112 (original) | 192 MB | 0,7453 | 0,7073 |
| 14 415 | 11 MB | 0,7422 | 0,7040 |
| 24 138 (final) | 18,6 MB | 0,7748 | 0,6773* |

\* o valor final inclui o encoder 4-bit e a cabeça int8; ver §4.

A poda remove o vocabulário dos outros scripts (CJK, árabe, cirílico…) e fica
com PT-BR + inglês + o corpus do jogo. **Cuidado:** a inferência passa as
palavras por `WhitespaceTokenSplitter`, que **minúscula** o texto e separa
pontuação; a poda tem de ser feita sobre formas minúsculas.

### 3.2 Pesos por bit (RTN, per-channel ou grupo)

| Bits | F1 synth | Δ | Observação |
|---|---:|---:|---|
| int8 | 0,7518 | +0,0 | sem perda |
| int6 | 0,7542 | +0,0 | sem perda |
| int5 g64 | 0,7446 | −0,1 pp | praticamente sem perda |
| int4 g32 | 0,6971 | −4,8 pp | RTN puro |
| int4 g128 | 0,7281 | −1,7 pp | |
| int3 g32 | 0,5118 | −23 pp | inviável |
| int2 | 0,0000 | colapso | inviável |

### 3.3 Sensibilidade por camada/módulo

- **Por camada (int4):** plana. Nenhuma camada isolada passa de −1,6 pp; o erro
  é aditivo. Não compensa uma alocação manual de bits por camada.
- **Por tipo de módulo (int2):** `ffn_in` e `ffn_out` colapsam; `qkv` colapsa;
  `attention.output` resiste (F1 0,60). Regra: **nunca abaixo de 4 bits em FFN**.
- **Cabeças:** `span_rep` e `classifier` em int4 são indistinguíveis do fp32.

### 3.4 GPTQ, HQQ e quantização de ativação

| Método | F1 synth | F1 hand | Latência | Tamanho |
|---|---:|---:|---:|---:|
| FP32 ONNX | 0,7422 | 0,7040 | 28 ms | 343 MB |
| int8 dinâmico (ORT) | **0,6042** | 0,5744 | **18,5 ms** | 190 MB |
| int8 estático (QDQ) | 0,2904 | 0,2954 | 103 ms | 133 MB |
| GPTQ 4-bit b64 | 0,7056 | 0,6502 | 32 ms | 92 MB |
| HQQ 4-bit b64 | 0,7226 | 0,6774 | 58 ms | 92 MB |
| **HQQ 4-bit b32** | **0,7525** | **0,6955** | 60 ms | 103 MB |

Conclusões:

1. **Quantizar ativações é inviável.** O int8 do ORT quantiza ativações por
   token; os outliers do DeBERTa derrubam 13 pp. Estático é pior (perda de 45 pp).
2. **GPTQ não ajudou** neste modelo (pior que HQQ em 4 bits); o Hessiano do
   encoder não compensa o custo.
3. **HQQ 4-bit com bloco 32** é o melhor ponto de qualidade/tamanho.
4. **2 bits não é viável** para esta arquitetura/tarefa, mesmo com GPTQ/HQQ.

### 3.5 Poda estruturada

| Camadas | F1 synth | F1 hand |
|---:|---:|---:|
| 12 | 0,7422 | 0,7040 |
| 11 | 0,7122 | 0,6388 |
| 10 | 0,5459 | 0,5572 |
| 8 | 0,0000 | 0,0000 |

Dropar uma única camada custa ~3 pp; de 10 para baixo colapsa. **Descartado.**

### 3.6 Embedding

Int8 per-row: sem perda (0,7437 / 0,7053). Int6: sem perda. Int4: −0,3 pp.
O embedding fica em int8.

## 4. Artefato final (`out/artifact/`, 145 MB)

| Arquivo | Tamanho | Conteúdo |
|---|---:|---|
| `encoder.onnx` + `.data` | 102,5 MB | DeBERTa podado, HQQ 4-bit b32, `inputs_embeds → last_hidden_state` |
| `embedding.bin` | 18,6 MB | embedding podado int8 + escala por linha |
| `heads.bin` | 29,7 MB | `span_rep`, `count_embed`/`count_pred`, `classifier`, int8 per-channel |
| `tokenizer.json` | 1,1 MB | Unigram podado (24 138 tokens) |
| `nlu_config.json` | 1 KB | ids especiais, rótulos, limiares |

O embedding **sai do grafo**: o runtime faz o gather e a desquantização, e o
ONNX recebe `inputs_embeds`. Isso evita o `Gather` int8 do ORT e reduz o
trabalho do encoder.

### Qualidade e latência (runtime Rust, fim a fim)

| Variante | F1 synth | F1 hand | Latência mediana | Tamanho |
|---|---:|---:|---:|---:|
| Referência fp32 (vocab 250k) | 0,7831 | 0,7111 | — | 1,2 GB |
| **Artefato 4-bit** | **0,7748** | **0,6773** | **88 ms** | **145 MB** |
| Encoder fp32 + mesmo tokenizer | — | 0,6986 | 57 ms | ~390 MB |

O custo do 4-bit é ~2 pp no conjunto manuscrito, contra 3,5× menos tamanho.
O encoder responde por ~66 ms dos 88 ms; a cabeça (AVX2) por ~12 ms.

## 5. Motor de inferência (`runtime/`)

Crate `vael-nlu` (Rust, `ort` + `tokenizers`):

1. **Splitter** — mesma regex de `WhitespaceTokenSplitter` (minúsculas,
   pontuação como token, offsets em bytes convertidos para caracteres).
2. **Prompt** — `( [P] entities ( [E] rótulo… ) ) [SEP_STRUCT]
   ( [P] intencao ( [C] intenção… ) ) [SEP_TEXT] palavras`.
3. **Embedding** — gather int8 + escala por linha.
4. **Encoder** — ONNX Runtime, 4-bit `MatMulNBits`, 8 threads.
5. **Cabeça** — implementada em Rust: `count_pred` → contagem;
   `count_embed` (GRU, passo 0) → queries; `span_rep` fatorizado → logits;
   `classifier` → intenção.
6. **Decode** — sigmoide, limiar 0,4, sobreposição gulosa por rótulo; offsets
   em caracteres.

Otimizações aplicadas:

- AVX2/FMA com `i8 → f32` e FMA contínuo nas GEMVs.
- GEMV de 4 vetores por linha de peso (reuso do carregamento).
- Projeções de span em lote para todas as palavras.
- Embedding fora do grafo ONNX.
- Threads do ORT configuráveis (`threads: 8`); 16 threads **piora** (102 ms
  contra 56 ms com 12) por oversubscription.

Uso:

```bash
./runtime/target/release/vael-nlu out/artifact "Comprar duas poções"
./runtime/target/release/vael-nlu out/artifact --bench bench/synth_test.jsonl saida.jsonl
```

## 6. Integração no jogo — "uma palavra basta"

O runtime vive agora em `../Viber/crates/nlu` (membro do workspace) e é exposto
ao Lua por `viber.nlu` (`src/luau/nlu.rs`). O artefacto está em
`assets/public/nlu/` (Git LFS: `*.onnx`, `*.data`, `*.bin`).

### API

```lua
viber.nlu.available("assets/public/nlu")  -- bool (ficheiros presentes)
viber.nlu.load("assets/public/nlu")       -- async; true se arrancou
viber.nlu.ready()                         -- bool
local id, err = viber.nlu.submit("lobos") -- não bloqueia; devolve id
local r = viber.nlu.poll()                -- nil | {kind="loaded"|"parsed"|"error", ...}
viber.nlu.parse("lobos")                  -- bloqueante; só QA/ferramentas
```

O load (~230 ms) e a inferência (~40–90 ms) correm numa **worker thread**; o
jogo só drena resultados no `on_update`. Sem a feature `nlu` ou sem o
artefacto, `available()` devolve `false` e tudo cai no fluxo antigo.

### A barra de fala

`world/hud.xml` ganhou a linha `say` (prompt + `<UiInput id="say-input">`).
`scripts/ui/say.lua` é a máquina de estados: auto-foco, Enter submete, Esc
cancela, timeout de 8 s, e um estado "A ouvir…" enquanto o worker responde.
Enquanto está focada, o `UiInput` come as teclas: a engine bloqueia
`viber.input.pressed/down/released`, `viber.interacted` e o movimento do herói
(`player_movement` lê `UiFocusedInput`), para que as letras de "pedra" ou
"lobos" não disparem [E], hotbar ou magia. Os scripts também checam
`viber.ui.focused()` antes de [E]/dígitos.

### Todas as 28 quests

Cada JSON tem agora:

```json
"parley": {
  "report": {
    "prompt": "Diz-me o que encontraste.",
    "example": "lobos",
    "expect": [{"kind": "criatura", "id": "wolf"}],
    "hint": "Não te ouvi dizer o nome da criatura."
  },
  "partial": false
}
```

- **kill:** uma palavra de criatura (`lobos`, `sombras`, `ogro`).
- **collect:** uma palavra de item (`pedras`, `lenha`, `musgo`); `partial: true`
  em três quests aceita um número menor e reduz a meta (`q.<id>.need`).
- **visit:** o nome de um dos lugares (`poço da encruzilhada`, `mojões`).

`scripts/lib/parley.lua` resolve o texto cru **e** as entidades do modelo
contra `data/nlu_aliases.json` (itens, criaturas, marcos, números). O texto cru
é essencial: substantivos isolados (`nota`, `poço`) às vezes não são extraídos.
`rules.quest_turn_in_parley` valida, guarda a palavra em `q.<id>.said` e, em
collect parcial, consome o que existe e reduz o que falta.

### Extras

- **Quadro de avisos:** [E] abre a barra; `lobos`, `goblins` ou `lenha` escolhem
  o cartaz e servem de relato.
- **Rota falada:** num townsfolk, a escolha `9. Perguntar por um lugar` abre a
  barra; `poço` resolve o marco e `travel.pin` marca o waypoint.
- **Pipeline autoral:** `tools/nlu/build_quest_index.py --model` lê as linhas
  das 28 quests com o modelo e escreve `data/nlu_quest_index.json`; `--apply`
  gera/valida os blocos `parley`.

### Degradação

Sem NLU: `dialogue.lua`/`forest-wolves.lua`/`notice-board.lua` chamam o
`quest_turn_in` antigo. Nenhuma quest fica presa; o relato é que deixa de ser
exigido.

## 7. Limites e próximos passos

- **Diálogo gerado** exige um LLM decoder-only (Gemma 3 270M, Qwen3-0.6B).
  É outra stack (GGUF/llama.cpp) e outro estudo.
- **Latência**: 88 ms é aceitável para comando de texto, não para uso por
  frame. Para <20 ms, o caminho é **destilação** para 4–6 camadas com os dados
  do jogo, não mais quantização.
- **Vocabulário**: 3 palavras do benchmark ainda segmentam diferente
  (`estalajadeiro`). Um fine-tune curto no domínio resolveria também os erros
  de rótulo (`cura` como `pessoa`, por exemplo).
- **Intenção**: 9 rótulos custam ~36 tokens no prompt. Reduzir para 5–6
  economiza ~5 ms.

## 8. Reprodução

```bash
cd tools/nlu
uv venv --python 3.12 .venv && uv pip install --python .venv/bin/python \
  torch --index-url https://download.pytorch.org/whl/cpu
uv pip install --python .venv/bin/python "gliner2[local]" onnx onnxruntime \
  onnx-ir onnxconverter-common wordfreq seqeval

.venv/bin/python bench/build_benchmark.py
.venv/bin/python bench/build_synth.py
.venv/bin/python scripts/quant_lab.py --only lowbits
.venv/bin/python scripts/gptq_lab.py --configs 4 --groups 64
.venv/bin/python scripts/ort_nbits.py --variants hqq4b32
.venv/bin/python scripts/build_artifact.py --top-pt 50000 --top-en 20000
cd runtime && cargo build --release && ./target/release/vael-nlu ../out/artifact --bench ../bench/synth_test.jsonl
```
