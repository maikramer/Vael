# d1-omni no Vael — estudo, compressão e integração

Modelo: [`LiquidAI/d1-omni-600M`](https://huggingface.co/LiquidAI/d1-omni-600M) ·
GGUF: [`mradermacher/d1-omni-600M-GGUF`](https://huggingface.co/mradermacher/d1-omni-600M-GGUF)

## TL;DR

- **Não é um chat LLM.** É um classificador de decisão não-generativo: recebe um
  `state` e perguntas tipadas (`noul` sim/não, `choice`, `score`) e devolve
  probabilidades, com **zero tokens de saída**. O GGUF `lfm2-d1-omni` inclui a
  cabeça de decisão e o llama.cpp (build 11669) expõe `POST /v1/systemone`.
- **É um router semântico, não um planejador.** Vai bem em intenção, routing de
  diálogo, verificação de ações e moderação em inglês. Falha em aritmética de
  política (loja/combate) e em inferência indireta (enigmas).
- **A língua importa muito.** Estado e instruções na mesma língua. Em PT, com
  instruções PT, “vou matar-te” → `attack` 0,73 (com instruções EN: 0,11) e
  “equipo a espada” → `use` 0,94 (EN: 0,02). Mesmo assim, moderação em PT é
  fraca; manter regras/EN para segurança.
- **Compressão:** o Q4_K_S completo (236 MB) é quase lossless face ao Q8
  (407 MB). A grande vitória é **podar o vocabulário**: 65 536 → ≈3 200 tokens
  (small) ou ≈9 700 (safe), com tokenização **100% idêntica** em 9 717 textos
  do jogo. O embedding cai de 67 M para 3,5–10 M parâmetros.
- **Motor do jogo:** **ONNX int4 + ORT, 187 MB**, 63,8% no v2 e 90,3% de
  concordância com o `q4_safe`; **p50 58 ms** no cartão (53 tokens). Reaproveita
  o `vael-nlu` (`ort`, `tokenizers`, embedding int8, worker thread). O grafo
  não inclui o embedding, por isso o mesmo ONNX serve o vocabulário safe/small.
- **Referência GGUF (llama.cpp/benchmark):** `q4_safe.gguf` — 202 MB, 92,4% de
  concordância com o Q8 (o mesmo que o Q4 completo de 236 MB). Alternativas:
  `q4_vocab.gguf` 195 MB, `q3m_vocab.gguf` 167 MB, `iq3_vocab.gguf` 142 MB.
- **API `viber.decision` implementada e testada** (worker, `submit`/`poll`,
  `decide` bloqueante). Falta ligar aos scripts de jogo.
- **Poda de camadas sem healing não funciona:** 2 ou 4 camadas conv → 39–40%
  de acerto e confiança colapsada. Se for preciso <120 MB, o caminho é
  destilação, não poda.

## O modelo

| Parte | Parâmetros | Notas |
|---|---:|---|
| Trunk LFM2.5-Encoder-350M | ~381 M | 16 camadas híbridas conv/atenção, hidden 1024, FFN 4608 |
| Cabeça de decisão | ~1 M + 2 blocos | `blk.16/17` + `cls.*`, scoring nos `<|mask|>` |
| Visão (SigLIP2) | 94 M | só no `mmproj`, fora do GGUF de texto |
| Áudio (FastConformer) | 112 M | idem |

- Contexto: 16 384 tokens (texto); com imagem, texto limitado a 896.
- 15 línguas, incluindo PT.
- A API oficial (`system_one`) e a do servidor (`/v1/systemone`) são iguais:
  `state` + `questions: {nome: {type, instructions, criteria}}`.
- O servidor avalia **duas variantes** com as opções em ordem inversa e faz a
  média, para reduzir o viés de posição. O `choice` devolve `choice`,
  `confidence`, `probabilities`; o `noul` devolve `noul` = P(sim); o `score`
  devolve a expectativa e a legenda.

## Como correr

Pré-requisitos: llama.cpp build **b11669** (ou mais recente), ~2 GB de disco e
os ficheiros `d1-omni-600M.f16.gguf` (729 MB) / `Q8_0` (407 MB) do repo GGUF.

```bash
cd tools/d1
./build.sh                     # downloads + imatrix + vocabulário + quantização
```

Para avaliar um artefacto:

```bash
./run_variant.sh q4_safe.gguf q4_safe 18090 bench2.jsonl
python3 summary.py             # tabela final a partir de results/preds_*.jsonl
```

O `run_variant.sh` arranca o `llama-server`, espera por `/health`, corre o
`eval.py` e mata o servidor. Os logs ficam em `logs/`.

## Benchmarks

- `bench.jsonl` (v1, 250 itens): intenção, enigma, verificação de ação,
  combate, loja e tom. Metade são casos ambíguos/fora do domínio; serve para
  medir **fidelidade** da quantização, não acerto absoluto.
- `bench2.jsonl` (v2, 185 itens): tarefas no formato do modelo, com instruções
  em inglês e opções descritivas; inclui língua (`pt`/`en`) para medir o fosso.
- `bench_oov.jsonl` (22 itens): fala do jogador com vocabulário raro, para
  testar a poda do vocabulário.
- `bench_card.jsonl`: o exemplo oficial do cartão (billing/urgency/angry).

Métricas:
- **acc**: argmax/limiar contra o rótulo esperado.
- **= Q8**: concordância da decisão com o Q8 completo.
- **TV**: distância de variação total média entre as distribuições `choice`.
- **Δ noul**: diferença média absoluta de P(sim).

## Resultados

Tamanhos em bytes no disco; `vs Q4` é relativo ao Q4_K_S completo (236 MB).
`v1 = Q8` é a concordância no conjunto difícil.

| Modelo | Disco | vs Q4 | v2 acc | v2 = Q8 | TV | Δ noul | v1 = Q8 |
|---|---:|---:|---:|---:|---:|---:|---:|
| Q8_0 completo | 407 MB | +73% | 65.9% | 100.0% | 0.000 | 0.000 | 100.0% |
| Q4_K_S completo | 236 MB | +0% | 62.2% | 92.4% | 0.092 | 0.058 | 86.4% |
| Q4_K_S + imatrix | 236 MB | +0% | 61.1% | 90.8% | 0.087 | 0.055 | — |
| Q3_K_M MIX | 212 MB | -10% | 62.7% | 89.7% | 0.153 | 0.080 | — |
| Q3_K_S MIX | 200 MB | -15% | 63.8% | 84.9% | 0.189 | 0.135 | — |
| IQ3_XXS MIX | 178 MB | -24% | 66.5% | 82.2% | 0.233 | 0.117 | 64.4% |
| IQ2_M MIX | 162 MB | -31% | 60.0% | 71.9% | 0.284 | 0.193 | — |
| **Q4 safe vocab** | **202 MB** | **-14%** | **63.8%** | **92.4%** | **0.098** | **0.055** | — |
| Q4 small vocab | 195 MB | -17% | 63.8% | 91.4% | 0.114 | 0.055 | 90.0% |
| Q3_K_M small vocab | 167 MB | -29% | 61.6% | 86.5% | 0.162 | 0.084 | — |
| IQ3_XXS small vocab | 142 MB | -40% | 65.9% | 83.8% | 0.220 | 0.112 | — |
| Poda 4 camadas IQ3 | 150 MB | -36% | 40.5% | 30.8% | 0.669 | 0.384 | — |

O `v2 acc` tem ruído de rótulos (é um benchmark sintético). A coluna que importa
para “não perder qualidade” é **`v2 = Q8` / TV / Δ noul**.

### Por categoria (q4_safe, v2)

| Categoria | Acerto | Comentário |
|---|---:|---|
| action (verificação de ação) | 87,5% | entailment direto; funciona em PT e EN |
| dialogue (routing) | 67,1% | EN 100%, PT ~70%; opções descritivas ajudam |
| intent | 45,7% | PT com instruções PT; 9 opções é muito |
| riddle | 50,0% | inferência indireta; não confiar |
| tone (moderação) | 66,7% | EN 91,7%, PT 55,6% |

## O que o modelo faz bem — e o que não faz

**Faz bem**
- Routing de diálogo: `state = {npc, player_utterance}`, `choice` com as
  respostas/ramificações como critérios. Ex.: “do you have beds for the night?”
  → `quartos` 0,75.
- Intenção do jogador com instruções na língua do estado e verbos explícitos.
- Verificação de ação/relato: “o herói matou lobo” vs “matou um lobo?” → 0,96.
- Moderação/ameaça em inglês com uma definição explícita (“threat = promise of
  violence, insult, intimidation”) → 0,998 num caso claro.
- Reranking, triagem, classificação de tópico, guardrails.

**Não faz bem**
- Política com aritmética (loja/combate): escolhe sempre a primeira opção ou
  fica perto de uniforme. Manter a regra em Lua.
- Enigmas que exigem raciocínio multi-hop (pista → criatura). 45–65%; manter o
  `parley`/aliases atuais.
- Moderação em PT: “Vou queimar a tua loja” → P(ameaça) 0,005 com instruções
  EN e 0,287 com PT. Usar regras/lista ou traduzir para EN.
- Planeamento de longo prazo, contagem, matemática.

## Língua e prompt (o achado mais importante)

| Caso | Instruções EN | Instruções PT |
|---|---:|---:|
| “vou matar-te” → attack | 0,11 | **0,73** |
| “equipo a espada” → use | 0,02 | **0,94** |
| “Vou queimar a tua loja” → ameaça | 0,005 | 0,287 |
| Action “matou lobo” → sim | 0,998 | 0,969 |
| Dialogue “tens quartos?” → quartos | 1,00 (EN) | 0,76 (PT) |

Regras práticas:
1. **Instruções e critérios na mesma língua do estado.**
2. Opções com descrições concretas, não só a etiqueta (`quartos: “Dois quartos
   livres…”`).
3. `noul` para verificações diretas (“a frase diz X?”), não para inferência.
4. Poucas opções (4–6). Com 9, a precisão cai.
5. Temperatura/calibração já vêm do `config.json`; não mexer.

## Compressão: o que funcionou e o que não

### 1. Quantização (funciona)
- Q8_0 → Q4_K_S: 407 → 236 MB, 92,4% de concordância no v2 e 86,4% no v1.
- Q3_K_M/Q3_K_S: 212/200 MB, 90/85% de concordância. Abaixo disso a
  calibração degrada.
- IQ3_XXS: 178 MB, 82% no v2 e 64% no v1; é outro modelo em casos difíceis.
- IQ2_M: 162 MB, 72%; rejeitado.
- **Precisão mista é obrigatória**: cabeça `blk.16/17` e `cls.weight` em Q8,
  embedding em Q8/Q6. São poucos MB e seguram a calibração.

### 2. imatrix (neutro neste corpus)
Construído com o texto do Vael + prompts do benchmark. No Q4 não melhorou o
v2 (90,8% vs 92,4%); fica sobretudo para os IQ de baixo bit, onde é obrigatório.
O `llama-imatrix` tem um bug com modelos de decisão: forçar
`-c 512 -b 512 -ub 512` (senão aborta no assert `n_ctx == n_seq * n_ctx`).

### 3. Poda de vocabulário (a grande vitória)
O corpus do Vael usa **2 069 tokens únicos**. O vocabulário do modelo tem
65 536 (64 402 reais + 1 134 `[PAD]`). Mantendo:
- o bloco especial 0–508 (para `mask`/`bos`/`eos` não mudarem de ID),
- os 256 bytes,
- os tokens do corpus e o **fecho de merges BPE**,
- opcionalmente os primeiros 8 192 tokens base (variante “safe”),

a tokenização fica **idêntica** em 9 717 textos. Resultado:

| Vocabulário | Tokens | Embedding (Q8) | Q4 total |
|---|---:|---:|---:|
| completo | 65 536 | 67 MB | 236 MB |
| safe | 9 791 | 10 MB | 202 MB |
| small | 3 420 | 3,5 MB | 195 MB |

Ganho extra: o embedding fica em **Q8** (em vez de Q4), o que melhora a
confiança em texto OOV. No exemplo oficial, `q4_vocab` dá 0,76 de confiança e
`q4_safe` 0,98 (o completo dá 0,999).

### 4. Poda de camadas (não funciona sem healing)
Remover 2 ou 4 camadas conv com `--prune-layers` e quantizar:
- 2 camadas: 39,5% de acerto, confiança 0,25–0,3.
- 4 camadas: 40,5%, confiança 0,08–0,5.
A poda também **não atualiza** `feed_forward_length`/`head_count_kv`; é preciso
`gguf_patch.py`. Mas o problema é a qualidade, não a metadata. **Não usar** sem
destilação/fine-tuning.

## Artefactos

### GGUF (llama.cpp; benchmark e experimentação)

Os GGUFs finais ficam no laboratório `~/GitClones/vael-d1-lab/` (fora do git):

| Ficheiro | Tamanho | Uso |
|---|---:|---|
| `q4_safe.gguf` | 202 MB | referência; prompts variados, OOV, PT/EN |
| `q4_vocab.gguf` | 195 MB | prompts fixos, máximo Q4 |
| `q3m_vocab.gguf` | 167 MB | agressivo, validar por tarefa |
| `iq3_vocab.gguf` | 142 MB | máximo; aceitar deriva de calibração |
| `d1-omni-600M.Q8_0.gguf` | 407 MB | referência |

O `build.sh` reconstrói qualquer um.

### ONNX (o motor do jogo, em `assets/public/d1/`)

| Ficheiro | Tamanho | Uso |
|---|---:|---|
| `model.onnx` + `model.onnx.data` | 177 MB | trunk + cabeça int4 (MatMulNBits b64) |
| `embedding.bin` | 9,9 MB | VAELEMB1 int8 (safe, 9 660 tokens) |
| `tokenizer.json` | 0,3 MB | vocabulário podado |
| `decision_config.json` | <1 KB | ids especiais, temperaturas, `max_len` |
| **total** | **187 MB** | **runtime do jogo** |

O artefacto está no working tree, **untracked**. Os `.onnx`/`.data` são
grandes: quando for para versionar, entrarem no LFS. O `build_onnx.sh`
reconstrói.

## Motor ideal: ONNX + ORT (reaproveita o `vael-nlu`)

O `vael-nlu` já carrega um encoder ONNX via `ort` e um `embedding.bin` int8.
O d1 usa **exatamente o mesmo contrato**:

| Peça | Como |
|---|---|
| grafo | `model.onnx`: trunk LFM2 + cabeça de decisão, **sem embedding** |
| entradas | `inputs_embeds [1,L,1024]`, `pad [1,L]`, `marker_pos [1,K]`, `marker_mask [1,K]`, `qtype [1]` |
| saída | `logits [1,K]` (uma por opção) |
| embedding | `embedding.bin` VAELEMB1 (o loader do `vael-nlu`) |
| tokenizer | `tokenizer.json` podado (`tokenizers`, já usado pelo NLU) |
| quantização | 4-bit `MatMulNBits`, bloco 64; cabeça e normas em fp32 |
| runtime | `crates/decision`, worker thread e `viber.decision` |

Porque é o ideal:
- **Não há sidecar nem llama.cpp no jogo.** O ORT já é dependência e já corre
  o GLiNER2; o custo marginal é o modelo.
- O grafo é **agnóstico ao vocabulário**: o embedding fica fora, por isso o
  mesmo ONNX serve o vocabulário completo, o “safe” (9 660) ou o “small”
  (3 162). Só mudam `embedding.bin` e `tokenizer.json`.
- O ORT traz kernels int4 otimizados; não é preciso um port manual do LFM2.
- O prompt é o `prompt.py` portado para Rust (`crates/decision/src/lib.rs`).

Números (v2, 185 itens; a concordância do int4 é com o `q4_safe` GGUF, a do
fp32/int8 é com o Q8):

| Artefacto | Disco | acc v2 | concordância | p50 (185 itens) |
|---|---:|---:|---:|---:|
| fp32 ONNX | 1 255 MB | 65,4% | 98,4% | 211 ms |
| int8 ONNX | 316 MB | 64,9% | 88,6% | 95 ms |
| **int4 b64 safe** | **187 MB** | **63,8%** | **90,3%** | **148 ms** |
| int4 b64 small | 180 MB | 64,3% | 89,2% | 129 ms |

No cartão (53 tokens), o binário `vael-decision` deu **p50 58 ms / p95 65 ms**
com o artefacto safe, e **546 ms** a carregar. O int4 muda ~10% das decisões
face ao `q4_safe`; o acerto no v2 é o mesmo. Se a calibração for crítica,
o int8 (316 MB) é a alternativa.

Testes já a passar:
- `cargo test -p vael-decision` — cartão → `billing`.
- `cargo test --lib lua_api_card_example` — API Lua completa.
- `cargo test --lib lua_api_intent_pt_multi_question` — PT, duas perguntas.
- `cargo run -p vael-decision --bin vael-decision -- assets/public/d1 10` —
  QA/latência fora da engine.

## Integração no jogo (implementada)

`viber.decision` está na engine (feature `decision`, ligada por omissão) em
`src/luau/decision.rs`, com worker thread, `submit`/`poll` e `decide`
bloqueante. O artefacto vive em `assets/public/d1/`; `available()` verifica os
5 ficheiros. Sem ele, `ready()` é `false` e os scripts caem no fluxo antigo
(como no `viber.nlu`).

```lua
-- carrega uma vez (assíncrono)
viber.decision.load("assets/public/d1")

-- choice: criteria é um ARRAY ordenado (a ordem é a das opções no modelo)
local id = viber.decision.submit(state, {
  intent = {
    type = "choice",
    instructions = "O que quer o jogador fazer?",
    criteria = {
      { key = "comprar", description = "comprar um item" },
      { key = "falar", description = "conversar" },
      { key = "atacar", description = "atacar ou ameaçar" },
      { key = "fugir", description = "afastar-se do perigo" },
    },
  },
  ameaca = {
    type = "noul",
    instructions = "O jogador está a ameaçar o mercador?",
    criteria = { ["false"] = "não", ["true"] = "sim" },
  },
})

local r = viber.decision.poll()  -- nil | {kind="loaded"|"decided"|"error", ...}
-- r.answers.intent.choice, r.answers.intent.confidence, r.answers.ameaca.noul
```

Regras:
- Instruções/critérios na língua do estado; opções descritivas; 4–6 opções.
- `choice` usa **array** (ordem explícita). `noul` usa mapa `false`/`true`;
  `score` usa array de níveis (2–10).
- Decisões de **regra** (preços, quests, dano) continuam em Lua; o modelo é a
  camada semântica (como o `judge.lua` já defende).
- `viber.decision.decide(state, questions)` é bloqueante (QA/ferramentas);
  no jogo usar `submit`/`poll`.
- `last_error()` devolve o erro do último load (nil se ok).

## Reprodução

```bash
cd tools/d1
./build.sh        # GGUF (llama.cpp + imatrix + vocabulário podado)
./build_onnx.sh   # ONNX int4 + embedding + tokenizer -> assets/public/d1
```

O `build_onnx.sh` precisa de `torch`, `safetensors`, `tokenizers`, `onnx`,
`onnxruntime` e `onnx-ir`; neste repo o venv do Text3D já tem torch, e o
`onnx`/`onnxruntime` podem ir para um `--target` com `PYTHONPATH`:

```bash
D1_PY=/home/maikeu/GitClones/AiGameKit/Text3D/.venv/bin/python \
PYTHONPATH=/tmp/d1py ./build_onnx.sh
```

O `build.sh` descarrega o llama.cpp b11669, o F16/Q8 e o `tokenizer.json`,
corre o imatrix (com o workaround do assert) e quantiza. Os passos manuais
estão no próprio script.

## Gotchas

- `llama-imatrix` em modelo de decisão ativa “embedding mode”; sem
  `-b 512 -c 512` aborta em `GGML_ASSERT(params.n_ctx == n_seq * n_ctx)`.
- `llama-quantize --prune-layers` renumera tensores mas **não** atualiza os
  arrays `lfm2.feed_forward_length` / `lfm2.attention.head_count_kv`;
  `gguf_patch.py` corrige.
- IQ2/IQ3 exigem entrada de imatrix para **todos** os tensores abaixo de 4 bit,
  incluindo `cls.weight`; forçar `cls.weight=q8_0`.
- Ao podar o vocabulário, manter **todo** o bloco 0–508. `<|reserved_4|>` e
  `<|reserved_5|>` são type 1 (não estão nos `added_tokens` do HF); removê-los
  desloca `<|mask|>` de 16 para 14 e o servidor rejeita os pedidos.
- O GGUF de texto já não inclui visão/áudio; os `mmproj` são ficheiros à parte
  e não são necessários para o Vael.
- No ONNX, o `nn.MultiheadAttention` do export legado fixa a sequência do
  dummy; a cabeça é exportada com uma `TransformerEncoderLayer` explícita
  (matmul/softmax, formas dinâmicas).
- O grafo ONNX **não** inclui o embedding; `inputs_embeds` já vem do
  `embedding.bin`. O `model.onnx.data` tem de estar ao lado do `model.onnx`.
- O `onnx.save_model` recusa-se a escrever `model.onnx.data` se o ficheiro já
  existir no diretório de trabalho; correr o save dentro de `assets/public/d1`.
- `MatMulNBits` precisa de ORT >= 1.17; o `ort` 2.0.0-rc.15 do `vael-nlu` já o
  suporta.
- O int4 ONNX muda ~10% das decisões face ao `q4_safe`; validar por tarefa. O
  int8 é 316 MB e não é mais preciso no v2.
- Na API Lua, `criteria` de `choice` é um array; um mapa Lua não tem ordem.

## Próximos passos

1. **Validar nos prompts reais do jogo** (não sintéticos): juntar 200–500 falas
   de jogador e decisões de NPC, medir o artefacto ONNX e escolher o limiar de
   confiança por tarefa.
2. **Ligar a API** ao `say.lua`, `townsfolk.lua`/`npc/dialogue.lua` e ao
   `judge.lua` como camada semântica (a API já está pronta).
3. **Se for preciso <120 MB**, destilar: professor d1-omni → aluno de 8–12
   camadas (ou um classificador pequeno por tarefa), com as distribuições do
   professor como soft labels e dados do jogo. A poda direta não chega.
4. **Moderação PT**: manter regras/listas ou traduzir para EN antes do modelo;
   o `noul` em PT não é confiável.
