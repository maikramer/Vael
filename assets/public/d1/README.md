# d1-omni — artefacto de decisão (ONNX/ORT)

Gerado por `tools/d1/build_onnx.sh`. O motor é o mesmo do `vael-nlu`:
`ort` + `tokenizers` + `embedding.bin` int8 + worker thread.

| Ficheiro | Papel |
|---|---|
| `model.onnx` + `model.onnx.data` | trunk LFM2 + cabeça de decisão, int4 (MatMulNBits b64), sem embedding |
| `embedding.bin` | VAELEMB1 int8 (safe, ~9 660 tokens) |
| `tokenizer.json` | vocabulário podado |
| `decision_config.json` | ids especiais, temperaturas, `max_len` |

Uso:

```lua
viber.decision.load("assets/public/d1")
```

Qualidade: 63,8% no bench v2, 90,3% de concordância com o `q4_safe` GGUF;
p50 58 ms no cartão. Detalhe e limitações em `tools/d1/README.md`.

Este diretório está **untracked** (os `.onnx`/`.data` são grandes). Para
versionar, entrarem no LFS. `tools/d1/build_onnx.sh` reconstrói.
