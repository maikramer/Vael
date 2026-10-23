#!/usr/bin/env python3
"""Exporta o d1-omni (texto) para ONNX sem o embedding.

O embedding fica fora do grafo, como no `vael-nlu`: a engine faz o gather int8
e passa `inputs_embeds`. O grafo recebe também as posições/máscara dos
marcadores e o tipo de pergunta, e devolve os logits por opção.

Uso (no venv do Text3D, com onnx/onnxruntime em PYTHONPATH):
  PYTHONPATH=/tmp/d1py python export_d1_onnx.py
"""
import json, os, sys

import torch
import torch.nn as nn

from encoder import Trunk, DecisionHead
from prompt import QTYPES

LAB = os.path.dirname(os.path.abspath(__file__))


# ---------------------------------------------------------------- prompt
def build_prompt(tok, state, question, max_len=2048, per_option=24):
    """Porta de prompt.encode, com `tokenizers.Tokenizer` em vez de HF."""
    import re
    DELIM = {"state": "<|reserved_7|>", "q": "<|reserved_8|>", "opt": "<|reserved_9|>",
             "opt_end": "<|reserved_10|>", "decide": "<|reserved_11|>"}
    MARKER = "<|mask|>"
    SPEC = re.compile(r"<\|([A-Za-z0-9_]+)\|>")

    def esc(s):
        return SPEC.sub(r"<¦\1¦>", s)

    def enc(s):
        return tok.encode(esc(s), add_special_tokens=False).ids

    def id_of(s):
        i = tok.token_to_id(s)
        if i is None:
            raise ValueError(f"token em falta: {s}")
        return i

    def criterion(v):
        return v if isinstance(v, str) else json.dumps(v, ensure_ascii=False, default=str)

    t = question["type"]
    if t == "choice":
        opts = [k if v in (None, "") else f"{k}: {criterion(v)}" for k, v in question["criteria"].items()]
    elif t == "score":
        opts = [f"level {i}: {criterion(c)}" for i, c in enumerate(question["criteria"])]
    else:
        crit = question.get("criteria") or {}
        false, true = crit.get("false", crit.get("no")), crit.get("true", crit.get("yes"))
        opts = ["false: " + (criterion(false) if false not in (None, "") else "no, the statement does not hold"),
                "true: " + (criterion(true) if true not in (None, "") else "yes, the statement holds")]

    budget = max(96, min(len(opts) * per_option + 32, max_len // 2))
    per = max(2, (budget - 3 * len(opts)) // len(opts))
    q = ([id_of(DELIM["q"])] + enc(question["instructions"]))[: max(16, budget)]
    markers = []
    for text in opts:
        markers.append(len(q) + 1)
        q += [id_of(DELIM["opt"]), id_of(MARKER)] + enc(" " + text)[:per] + [id_of(DELIM["opt_end"])]
    q.append(id_of(DELIM["decide"]))
    room = max(0, max_len - len(q) - 2)
    state_ids = [id_of(DELIM["state"])] + enc(state if isinstance(state, str) else json.dumps(state, ensure_ascii=False))[:room]
    ids = ([tok.token_to_id("<|startoftext|>")] + state_ids + q)[:max_len]
    markers = [m + 1 + len(state_ids) for m in markers]
    if markers[-1] >= max_len:
        raise ValueError("as opções não cabem no contexto")
    return ids, markers


class Export(nn.Module):
    def __init__(self, trunk, head):
        super().__init__()
        self.trunk = trunk
        self.head = head

    def forward(self, inputs_embeds, pad, marker_pos, marker_mask, qtype):
        prefix = torch.zeros(inputs_embeds.shape[0], dtype=torch.long, device=inputs_embeds.device)
        h = self.trunk(inputs_embeds, pad, prefix)
        return self.head(h, pad, marker_pos, marker_mask, qtype)


def main():
    cfg = json.load(open(f"{LAB}/config.json"))["text_config"]
    trunk = Trunk(cfg)
    head = DecisionHead(cfg["hidden_size"], 2)

    from safetensors.torch import load_file
    sd = load_file(f"{LAB}/model.safetensors")
    enc_sd = {k[len("encoder."):]: v for k, v in sd.items() if k.startswith("encoder.")}
    head_sd = {k[len("head."):]: v for k, v in sd.items() if k.startswith("head.")}
    missing, unexpected = trunk.load_state_dict(enc_sd, strict=False)
    print("trunk missing", missing[:5], "unexpected", len(unexpected))
    missing, unexpected = head.load_state_dict(head_sd, strict=False)
    print("head missing", missing[:5], "unexpected", len(unexpected))
    trunk.eval(); head.eval()

    # Para o export não vale a pena arrastar o embedding (67M) dentro do grafo.
    del trunk.embed_tokens
    model = Export(trunk, head).eval()

    # substitui SDPA por matmul/softmax (exportável em opset 17)
    from encoder import Attention
    def attn_export(self, x, cos, sin, mask):
        b, length, _ = x.shape
        shape = (b, length, -1, self.head_dim)
        q = self.q_layernorm(self.q_proj(x).view(shape)).transpose(1, 2)
        k = self.k_layernorm(self.k_proj(x).view(shape)).transpose(1, 2)
        v = self.v_proj(x).view(shape).transpose(1, 2)
        q, k = q * cos + _rotate_half(q) * sin, k * cos + _rotate_half(k) * sin
        groups = self.heads // self.kv_heads
        k = k.repeat_interleave(groups, dim=1)
        v = v.repeat_interleave(groups, dim=1)
        scores = torch.matmul(q, k.transpose(-1, -2)) * (self.head_dim ** -0.5) + mask
        y = torch.matmul(torch.softmax(scores, dim=-1), v)
        return self.out_proj(y.transpose(1, 2).reshape(b, length, -1))
    from encoder import _rotate_half
    Attention.forward = attn_export

    # A cabeça usa nn.TransformerEncoderLayer; o export legado do MHA fixa a
    # sequência do dummy. Reimplementamos a layer com formas dinâmicas.
    import torch.nn.functional as F
    def layer_forward(self, src, src_mask=None, src_key_padding_mask=None, is_causal=False):
        def sa(x):
            b, l, d = x.shape
            qkv = F.linear(x, self.self_attn.in_proj_weight, self.self_attn.in_proj_bias)
            q, k, v = qkv.chunk(3, dim=-1)
            nh, hd = self.self_attn.num_heads, self.self_attn.head_dim
            q = q.view(b, l, nh, hd).transpose(1, 2)
            k = k.view(b, l, nh, hd).transpose(1, 2)
            v = v.view(b, l, nh, hd).transpose(1, 2)
            scores = torch.matmul(q, k.transpose(-1, -2)) * (hd ** -0.5)
            if src_key_padding_mask is not None:
                scores = scores.masked_fill(src_key_padding_mask[:, None, None, :], float("-inf"))
            y = torch.matmul(torch.softmax(scores, dim=-1), v)
            return self.self_attn.out_proj(y.transpose(1, 2).reshape(b, l, d))
        def ff(x):
            return self.linear2(self.activation(self.linear1(x)))
        x = src
        if self.norm_first:
            x = x + sa(self.norm1(x))
            x = x + ff(self.norm2(x))
        else:
            x = self.norm1(x + sa(x))
            x = self.norm2(x + ff(x))
        return x
    nn.TransformerEncoderLayer.forward = layer_forward

    B, L, K = 1, 32, 3
    dummy = (torch.randn(B, L, cfg["hidden_size"]), torch.ones(B, L, dtype=torch.bool),
             torch.zeros(B, K, dtype=torch.long), torch.ones(B, K, dtype=torch.bool),
             torch.zeros(B, dtype=torch.long))
    raw = f"{LAB}/model_raw.onnx"
    torch.onnx.export(
        model, dummy, raw,
        input_names=["inputs_embeds", "pad", "marker_pos", "marker_mask", "qtype"],
        output_names=["logits"],
        dynamic_axes={
            "inputs_embeds": {0: "batch", 1: "seq"},
            "pad": {0: "batch", 1: "seq"},
            "marker_pos": {0: "batch", 1: "nopts"},
            "marker_mask": {0: "batch", 1: "nopts"},
            "qtype": {0: "batch"},
            "logits": {0: "batch", 1: "nopts"},
        },
        opset_version=17, do_constant_folding=True, dynamo=False,
    )
    print("raw export", os.path.getsize(raw) / 1e6, "MB")

    import onnx
    m = onnx.load(raw)
    onnx.save_model(m, f"{LAB}/model.onnx", save_as_external_data=True,
                    all_tensors_to_one_file=True, location="model.onnx.data", size_threshold=1024)
    os.remove(raw)
    print("model.onnx", os.path.getsize(f"{LAB}/model.onnx") / 1e6, "MB +",
          os.path.getsize(f"{LAB}/model.onnx.data") / 1e6, "MB")

    # validação com um prompt real
    from tokenizers import Tokenizer
    tok = Tokenizer.from_file(f"{LAB}/tokenizer.json")
    state = "I was charged twice for my order last week and nobody has replied."
    q = {"type": "choice", "instructions": "Which team should handle this?",
         "criteria": {"billing": "payments and refunds", "shipping": None, "technical": None}}
    ids, markers = build_prompt(tok, state, q)
    print("prompt", len(ids), "tokens, markers", markers)
    # o embedding foi apagado; usa a matriz do state dict
    emb = enc_sd["embed_tokens.weight"][torch.tensor(ids)]
    inputs = (emb[None], torch.ones(1, len(ids), dtype=torch.bool),
              torch.tensor([markers]), torch.ones(1, len(markers), dtype=torch.bool),
              torch.tensor([QTYPES[q["type"]]]))
    with torch.no_grad():
        ref = model(*inputs).numpy()
    import onnxruntime as ort
    sess = ort.InferenceSession(f"{LAB}/model.onnx", providers=["CPUExecutionProvider"])
    feeds = {
        "inputs_embeds": emb[None].numpy(),
        "pad": inputs[1].numpy(),
        "marker_pos": inputs[2].numpy(),
        "marker_mask": inputs[3].numpy(),
        "qtype": inputs[4].numpy(),
    }
    out = sess.run(None, feeds)[0]
    print("pt", ref, "onnx", out, "maxdiff", abs(ref - out).max())
    print("softmax", torch.softmax(torch.tensor(out[0]), -1).numpy())


if __name__ == "__main__":
    main()
