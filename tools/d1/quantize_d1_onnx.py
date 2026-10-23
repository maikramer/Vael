#!/usr/bin/env python3
"""Quantiza o grafo ONNX do d1 (MatMul int8 / 4-bit) e valida contra o fp32."""
import os
import numpy as np
import torch
import onnxruntime as ort

from export_d1_onnx import build_prompt
from safetensors.torch import load_file
from tokenizers import Tokenizer
from prompt import QTYPES

LAB = os.path.dirname(os.path.abspath(__file__))


def feeds_for(tok, emb, state, question):
    ids, markers = build_prompt(tok, state, question)
    e = emb[torch.tensor(ids)][None].numpy()
    return {
        "inputs_embeds": e,
        "pad": np.ones((1, len(ids)), dtype=bool),
        "marker_pos": np.array([markers], dtype=np.int64),
        "marker_mask": np.ones((1, len(markers)), dtype=bool),
        "qtype": np.array([QTYPES[question["type"]]], dtype=np.int64),
    }


def run(path, feeds):
    s = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
    return s.run(None, feeds)[0]


def main():
    from onnxruntime.quantization import quantize_dynamic, QuantType
    from onnxruntime.quantization.matmul_nbits_quantizer import MatMulNBitsQuantizer

    print("== int8")
    quantize_dynamic(f"{LAB}/model.onnx", f"{LAB}/model_int8.onnx",
                     weight_type=QuantType.QInt8, per_channel=True,
                     op_types_to_quantize=["MatMul"])
    print("int8", os.path.getsize(f"{LAB}/model_int8.onnx") / 1e6, "MB +",
          os.path.getsize(f"{LAB}/model_int8.onnx.data") / 1e6 if os.path.exists(f"{LAB}/model_int8.onnx.data") else 0, "MB")

    print("== 4-bit")
    q = MatMulNBitsQuantizer(f"{LAB}/model.onnx", bits=4, block_size=32,
                             is_symmetric=True, accuracy_level=4)
    q.process()
    q.model.save_model_to_file(f"{LAB}/model_int4.onnx", use_external_data_format=True)
    print("int4", os.path.getsize(f"{LAB}/model_int4.onnx") / 1e6, "MB +",
          os.path.getsize(f"{LAB}/model_int4.onnx.data") / 1e6, "MB")

    tok = Tokenizer.from_file(f"{LAB}/tokenizer.json")
    emb = load_file(f"{LAB}/model.safetensors")["encoder.embed_tokens.weight"]

    cases = [
        ("card", "I was charged twice for my order last week and nobody has replied.",
         {"type": "choice", "instructions": "Which team should handle this?",
          "criteria": {"billing": "payments and refunds", "shipping": None, "technical": None}}),
        ("intent", {"utterance": "vou matar-te", "npc": "Greta, mercadora"},
         {"type": "choice", "instructions": "O que quer o jogador fazer?",
          "criteria": {"buy": "comprar um item", "sell": "vender um item",
                       "talk": "conversar", "attack": "atacar ou ameaçar",
                       "travel": "ir a algum lugar", "use": "usar um item",
                       "deliver": "entregar um item", "ask": "perguntar",
                       "none": "nenhum pedido"}}),
        ("tone", {"utterance": "Vou queimar a tua loja.", "npc": "Greta"},
         {"type": "noul", "instructions": "O jogador está a ameaçar o mercador?",
          "criteria": {"false": "não", "true": "sim"}}),
    ]
    for name, state, q in cases:
        feeds = feeds_for(tok, emb, state, q)
        ref = run(f"{LAB}/model.onnx", feeds)[0].astype(np.float64)
        i8 = run(f"{LAB}/model_int8.onnx", feeds)[0].astype(np.float64)
        i4 = run(f"{LAB}/model_int4.onnx", feeds)[0].astype(np.float64)
        print(f"\n{name}: ref {np.round(ref, 4)}")
        print(f"  int8 maxdiff {np.abs(ref-i8).max():.4f}  int4 maxdiff {np.abs(ref-i4).max():.4f}")
        if len(q.get("criteria", [])) or q["type"] == "choice":
            print("  probs ref", np.round(torch.softmax(torch.tensor(ref[0]), -1).numpy(), 4))
            print("  probs i8 ", np.round(torch.softmax(torch.tensor(i8[0]), -1).numpy(), 4))
            print("  probs i4 ", np.round(torch.softmax(torch.tensor(i4[0]), -1).numpy(), 4))


if __name__ == "__main__":
    main()
