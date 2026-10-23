#!/usr/bin/env python3
"""Gera o artefacto de decisão no formato do vael-nlu:
tokenizer.json (podado) + embedding.bin (int8 por linha) + decision_config.json.

Uso: python make_onnx_artifact.py <dir_saida> --keep-base 8192
"""
import argparse, json, os, sys
import torch
from safetensors.torch import load_file
from tokenizers import Tokenizer

from prune_vocab import build_keep

LAB = os.path.dirname(os.path.abspath(__file__))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--keep-base", type=int, default=8192)
    args = ap.parse_args()
    out = os.path.abspath(args.out)
    os.makedirs(out, exist_ok=True)

    tj, orig, texts, id2s, kept_ids, keep, merges, specials = build_keep(args.keep_base)
    new_of = {old: new for new, old in enumerate(kept_ids)}
    id_of = {s: new_of[old] for old, s in id2s.items() if old in new_of}

    # --- tokenizer.json podado
    nt = json.loads(json.dumps(tj))
    nt["model"]["vocab"] = {s: id_of[s] for old, s in id2s.items() if old in new_of}
    def parts(m): return m.split(" ") if isinstance(m, str) else m
    nt["model"]["merges"] = [m for m in merges if parts(m)[0] + parts(m)[1] in keep]
    added = []
    for a in tj["added_tokens"]:
        if a["content"] in id_of:
            b = dict(a)
            b["id"] = id_of[a["content"]]
            added.append(b)
    nt["added_tokens"] = added
    tok_path = os.path.join(out, "tokenizer.json")
    with open(tok_path, "w", encoding="utf-8") as f:
        json.dump(nt, f, ensure_ascii=False, separators=(",", ":"))
    print("tokenizer", os.path.getsize(tok_path) / 1e6, "MB", len(kept_ids), "tokens")

    # confirma que a tokenização não mudou no corpus
    red = Tokenizer.from_file(tok_path)
    total = same = 0
    for t in texts:
        a = orig.encode(t, add_special_tokens=False).tokens
        b = red.encode(t, add_special_tokens=False).tokens
        total += len(a)
        same += sum(1 for x, y in zip(a, b) if x == y)
    print(f"tokenização: {same/total:.4%} de {total} tokens")

    # --- embedding.bin int8 (VAELEMB1)
    emb = load_file(f"{LAB}/model.safetensors")["encoder.embed_tokens.weight"]
    rows = emb[torch.tensor(kept_ids)].to(torch.float32)
    scale = rows.abs().amax(dim=1) / 127.0
    scale = torch.where(scale > 0, scale, torch.ones_like(scale))
    q = torch.round(rows / scale[:, None]).clamp(-127, 127).to(torch.int8).numpy()
    sp = os.path.join(out, "embedding.bin")
    with open(sp, "wb") as f:
        f.write(b"VAELEMB1")
        f.write(len(kept_ids).to_bytes(4, "little"))
        f.write((1024).to_bytes(4, "little"))
        f.write(q.tobytes())
        f.write(scale.numpy().astype("<f4").tobytes())
    print("embedding", os.path.getsize(sp) / 1e6, "MB",
          "erro médio", (q.astype("float32") * scale.numpy()[:, None] - rows.numpy()).__abs__().mean())

    cfg = {
        "artifact": "d1-omni-onnx",
        "vocab_size": len(kept_ids),
        "hidden_size": 1024,
        "max_len": 2048,
        "special": {
            "bos": id_of["<|startoftext|>"],
            "state": id_of["<|reserved_7|>"],
            "question": id_of["<|reserved_8|>"],
            "option": id_of["<|reserved_9|>"],
            "option_end": id_of["<|reserved_10|>"],
            "decide": id_of["<|reserved_11|>"],
            "mask": id_of["<|mask|>"],
        },
        "qtypes": {"choice": 0, "score": 1, "noul": 2},
        "temperatures": json.load(open(f"{LAB}/config.json"))["temperatures"],
        "threads": 8,
    }
    with open(os.path.join(out, "decision_config.json"), "w") as f:
        json.dump(cfg, f, ensure_ascii=False, indent=2)
    print("config ok")


if __name__ == "__main__":
    main()
