#!/usr/bin/env bash
# Reconstrói os artefactos comprimidos do d1-omni para o Vael.
# Requer: curl, tar, python3 com `tokenizers` (pip install tokenizers).
set -euo pipefail

LAB="$(cd "$(dirname "$0")" && pwd)"
cd "$LAB"
mkdir -p llama logs results

LLAMA_VER=b11669
HF_GGUF=https://huggingface.co/mradermacher/d1-omni-600M-GGUF/resolve/main
HF_ORIG=https://huggingface.co/LiquidAI/d1-omni-600M/resolve/main

# 1. llama.cpp (build com /v1/systemone)
if [ ! -x "llama/llama-$LLAMA_VER/llama-server" ]; then
  echo "== llama.cpp $LLAMA_VER"
  curl -L --fail -o llama-bin.tar.gz \
    "https://github.com/ggml-org/llama.cpp/releases/download/$LLAMA_VER/llama-$LLAMA_VER-bin-ubuntu-x64.tar.gz"
  tar xzf llama-bin.tar.gz -C llama
fi
Q="llama/llama-$LLAMA_VER/llama-quantize"
IM="llama/llama-$LLAMA_VER/llama-imatrix"

# 2. ficheiros base
for spec in "d1-omni-600M.f16.gguf|$HF_GGUF/d1-omni-600M.f16.gguf" \
            "d1-omni-600M.Q8_0.gguf|$HF_GGUF/d1-omni-600M.Q8_0.gguf" \
            "tokenizer.json|$HF_ORIG/tokenizer.json"; do
  f=${spec%%|*}; url=${spec#*|}
  [ -f "$f" ] || { echo "== $f"; curl -L --fail -o "$f" "$url"; }
done

# 3. imatrix (opcional; o calib.txt já cobre o jogo e os prompts)
if [ ! -f imatrix.gguf ]; then
  echo "== imatrix"
  # decisão = embedding mode; sem -b 512 -c 512 o assert n_ctx == n_seq*n_ctx falha
  "$IM" -m d1-omni-600M.f16.gguf -f calib.txt -o imatrix.gguf \
        -c 512 -b 512 -ub 512 --chunks 128 --no-ppl -t 8
fi

# 4. vocabulário podado (small = máximo; safe = +8k tokens base)
[ -f vocab_small.f16.gguf ] || python3 prune_vocab.py d1-omni-600M.f16.gguf vocab_small.f16.gguf --keep-base 0
[ -f vocab_safe.f16.gguf ]  || python3 prune_vocab.py d1-omni-600M.f16.gguf vocab_safe.f16.gguf  --keep-base 8192

# 5. quantização final: cabeça e embedding em Q8, trunk no tipo escolhido
COMMON=(--imatrix imatrix.gguf --token-embedding-type q8_0 --tensor-type 'blk\.1[67]\..*=q8_0')

echo "== q4_safe"
"$Q" "${COMMON[@]}" vocab_safe.f16.gguf q4_safe.gguf Q4_K_S 8
echo "== q4_vocab"
"$Q" "${COMMON[@]}" vocab_small.f16.gguf q4_vocab.gguf Q4_K_S 8
echo "== q3m_vocab"
"$Q" "${COMMON[@]}" vocab_small.f16.gguf q3m_vocab.gguf Q3_K_M 8
echo "== iq3_vocab"
"$Q" "${COMMON[@]}" --tensor-type 'cls\.weight=q8_0' vocab_small.f16.gguf iq3_vocab.gguf IQ3_XXS 8

echo
ls -lh q4_safe.gguf q4_vocab.gguf q3m_vocab.gguf iq3_vocab.gguf
