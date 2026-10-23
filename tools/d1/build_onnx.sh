#!/usr/bin/env bash
# Reconstrói o artefacto ONNX/ORT do d1-omni em assets/public/d1.
# Requer torch, safetensors, tokenizers, onnx, onnxruntime e onnx-ir.
# Neste repo, o venv do Text3D já tem torch/safetensors; onnx/onnxruntime
# podem ir para um --target e entrar por PYTHONPATH.
#
#   D1_PY=/caminho/venv/bin/python PYTHONPATH=/tmp/d1py ./build_onnx.sh
set -euo pipefail

LAB="$(cd "$(dirname "$0")" && pwd)"
GAME="$(cd "$LAB/../.." && pwd)"
PY="${D1_PY:-python3}"
HF=https://huggingface.co/LiquidAI/d1-omni-600M/resolve/main
cd "$LAB"

$PY - <<'PY'
mods = ["torch", "safetensors", "tokenizers", "onnx", "onnxruntime", "onnx_ir"]
missing = [m for m in mods if not __import__("importlib").util.find_spec(m)]
if missing:
    raise SystemExit("faltam módulos: " + ", ".join(missing) +
                     "\ninstala torch/safetensors e onnx/onnxruntime/onnx-ir")
PY

for f in config.json encoder.py prompt.py tokenizer.json model.safetensors; do
  [ -f "$f" ] || { echo "== $f"; curl -L --fail -o "$f" "$HF/$f"; }
done

echo "== export ONNX (trunk + cabeça, sem embedding)"
$PY export_d1_onnx.py

echo "== quantização 4-bit (MatMulNBits, bloco 64)"
$PY - <<'PY'
from onnxruntime.quantization.matmul_nbits_quantizer import MatMulNBitsQuantizer
q = MatMulNBitsQuantizer("model.onnx", bits=4, block_size=64,
                         is_symmetric=True, accuracy_level=4)
q.process()
q.model.save_model_to_file("model_int4.onnx", use_external_data_format=True)
PY

echo "== tokenizer podado + embedding int8 (safe)"
$PY make_onnx_artifact.py onnx_safe --keep-base 8192

echo "== artefacto final em $GAME/assets/public/d1"
mkdir -p "$GAME/assets/public/d1"
cp onnx_safe/embedding.bin onnx_safe/tokenizer.json onnx_safe/decision_config.json \
   "$GAME/assets/public/d1/"
$PY - "$LAB/model_int4.onnx" "$GAME/assets/public/d1" <<'PY'
import os, shutil, sys
import onnx
src, out = sys.argv[1], sys.argv[2]
os.chdir(out)
for f in ("model.onnx", "model.onnx.data"):
    if os.path.exists(f):
        os.remove(f)
m = onnx.load(src)
onnx.save_model(m, "model.onnx", save_as_external_data=True,
                all_tensors_to_one_file=True, location="model.onnx.data",
                size_threshold=0)
PY
ls -lh "$GAME/assets/public/d1"
