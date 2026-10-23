#!/usr/bin/env python3
"""Corre o ONNX int4 com um artefacto podado (tokenizer + embedding.bin)."""
import argparse, json, os, struct, sys, time
import numpy as np
import onnxruntime as ort
import torch

LAB = os.path.dirname(os.path.abspath(__file__))


def load_embedding(path):
    b = open(path, "rb").read()
    assert b[:8] == b"VAELEMB1"
    rows, dim = struct.unpack_from("<II", b, 8)
    q = np.frombuffer(b, dtype=np.int8, count=rows * dim, offset=16).reshape(rows, dim)
    scales = np.frombuffer(b, dtype="<f4", count=rows, offset=16 + rows * dim)
    return torch.tensor(q.astype(np.float32) * scales[:, None])


def answer(q, logits, temps):
    z = torch.tensor(logits, dtype=torch.float64)
    n = 2 if q["type"] == "noul" else len(q.get("criteria", []))
    key = q["type"] + ":" + ("2" if n <= 2 else "3-5" if n <= 5 else "6-10" if n <= 10 else "11+")
    p = torch.softmax(z / temps.get(key, temps.get(q["type"], 1.0)), -1).tolist()
    if q["type"] == "noul":
        return {"type": "noul", "noul": p[1]}
    names = list(q["criteria"])
    best = max(range(len(p)), key=p.__getitem__)
    return {"type": "choice", "choice": names[best], "confidence": p[best],
            "probabilities": dict(zip(names, p))}


def agree(preds, ref):
    ok = n = 0
    for p in preds:
        r = ref.get(p["id"])
        if not r or "error" in r or r["answer"]["type"] != p["answer"]["type"]:
            continue
        n += 1
        a, b = p["answer"], r["answer"]
        if a["type"] == "choice":
            ok += int(a["choice"] == b["choice"])
        else:
            ok += int((a["noul"] >= 0.5) == (b["noul"] >= 0.5))
    return ok, n


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("artifact")
    ap.add_argument("--model", default="model_int4.onnx")
    ap.add_argument("--name", default="int4_safe")
    args = ap.parse_args()
    art = os.path.abspath(args.artifact)
    tok = __import__("tokenizers").Tokenizer.from_file(os.path.join(art, "tokenizer.json"))
    emb = load_embedding(os.path.join(art, "embedding.bin"))
    cfg = json.load(open(os.path.join(art, "decision_config.json")))

    # importa o construtor de prompt (usa token_to_id; funciona com o podado)
    sys.path.insert(0, LAB)
    from export_d1_onnx import build_prompt
    from prompt import QTYPES

    items = [json.loads(l) for l in open(f"{LAB}/bench2.jsonl")]
    sess = ort.InferenceSession(f"{LAB}/{args.model}", providers=["CPUExecutionProvider"])
    out, lat = [], []
    for it in items:
        q = it["questions"]["q"]
        ids, markers = build_prompt(tok, it["state"], q)
        e = emb[torch.tensor(ids)][None].numpy()
        feeds = {"inputs_embeds": e,
                 "pad": np.ones((1, len(ids)), dtype=bool),
                 "marker_pos": np.array([markers], dtype=np.int64),
                 "marker_mask": np.ones((1, len(markers)), dtype=bool),
                 "qtype": np.array([QTYPES[q["type"]]], dtype=np.int64)}
        t0 = time.perf_counter()
        logits = sess.run(None, feeds)[0][0]
        lat.append((time.perf_counter() - t0) * 1000)
        out.append({"id": it["id"], "cat": it["cat"], "expected": it["expected"],
                    "answer": answer(q, logits, cfg["temperatures"]), "ms": lat[-1]})
    with open(f"{LAB}/preds_onnx_{args.name}.jsonl", "w") as f:
        for r in out:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")

    ok = n = 0
    for r in out:
        a = r["answer"]
        got = a.get("choice") if a["type"] == "choice" else (a["noul"] >= 0.5)
        ok += int(got == r["expected"]); n += 1
    refs = {
        "q4_safe": "preds_q4_safe.jsonl",
        "q8": "preds_q8_b2.jsonl",
        "int4_full": "preds_onnx_int4.jsonl",
        "fp32": "preds_onnx_fp32.jsonl",
    }
    print(f"{args.name}: acc {ok}/{n} = {ok/n:.1%}")
    for tag, path in refs.items():
        p = os.path.join(LAB, path)
        if not os.path.exists(p):
            continue
        r = {json.loads(l)["id"]: json.loads(l) for l in open(p)}
        a, b = agree(out, r)
        print(f"  = {tag:9s} {a}/{b} = {a/b:.1%}")
    sl = sorted(lat)
    print(f"  p50 {sl[len(sl)//2]:.0f} ms  p95 {sl[int(len(sl)*.95)]:.0f} ms")


if __name__ == "__main__":
    main()
