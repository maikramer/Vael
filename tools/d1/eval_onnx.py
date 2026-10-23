#!/usr/bin/env python3
"""Avalia o ONNX fp32/int8/int4 no bench2 e compara com o Q8 do GGUF."""
import json, os, sys, time
import numpy as np
import onnxruntime as ort
import torch
from safetensors.torch import load_file
from tokenizers import Tokenizer

from export_d1_onnx import build_prompt
from prompt import QTYPES, temperature_key

LAB = os.path.dirname(os.path.abspath(__file__))
TEMPS = json.load(open(f"{LAB}/config.json"))["temperatures"]


def answer(q, logits):
    z = torch.tensor(logits, dtype=torch.float64)
    n = 2 if q["type"] == "noul" else len(q.get("criteria", []))
    key = q["type"] + ":" + ("2" if n <= 2 else "3-5" if n <= 5 else "6-10" if n <= 10 else "11+")
    temp = TEMPS.get(key, TEMPS.get(q["type"], 1.0))
    p = torch.softmax(z / temp, -1).tolist()
    if q["type"] == "noul":
        return {"type": "noul", "noul": p[1], "probs": p}
    if q["type"] == "choice":
        names = list(q["criteria"])
        best = max(range(len(p)), key=p.__getitem__)
        return {"type": "choice", "choice": names[best], "confidence": p[best],
                "probabilities": dict(zip(names, p))}
    return {"type": "score", "score": sum(i * x for i, x in enumerate(p)), "probs": p}


def main():
    items = [json.loads(l) for l in open(f"{LAB}/bench2.jsonl")]
    tok = Tokenizer.from_file(f"{LAB}/tokenizer.json")
    emb = load_file(f"{LAB}/model.safetensors")["encoder.embed_tokens.weight"]
    ref = {json.loads(l)["id"]: json.loads(l) for l in open(f"{LAB}/preds_q8_b2.jsonl")}

    models = [("fp32", "model.onnx"), ("int8", "model_int8.onnx"), ("int4", "model_int4.onnx")]
    for name, path in models:
        sess = ort.InferenceSession(f"{LAB}/{path}", providers=["CPUExecutionProvider"])
        out = []
        lat = []
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
                        "answer": answer(q, logits), "ms": lat[-1]})
        path_out = f"{LAB}/preds_onnx_{name}.jsonl"
        with open(path_out, "w") as f:
            for r in out:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
        # acerto
        ok = n = 0
        agree = nn = 0
        for r in out:
            a = r["answer"]
            got = a.get("choice") if a["type"] == "choice" else (a["noul"] >= 0.5)
            ok += int(got == r["expected"]); n += 1
            rr = ref.get(r["id"])
            if rr and "error" not in rr and rr["answer"]["type"] == a["type"]:
                nn += 1
                ra = rr["answer"]
                if a["type"] == "choice":
                    agree += int(a["choice"] == ra["choice"])
                else:
                    agree += int((a["noul"] >= 0.5) == (ra["noul"] >= 0.5))
        sl = sorted(lat)
        size = os.path.getsize(f"{LAB}/{path}") + (os.path.getsize(f"{LAB}/{path}.data") if os.path.exists(f"{LAB}/{path}.data") else 0)
        print(f"{name:5s} {size/1e6:6.0f} MB  acc {ok}/{n} = {ok/n:.1%}  agreeQ8 {agree}/{nn} = {agree/nn:.1%}  p50 {sl[len(sl)//2]:.0f} ms p95 {sl[int(len(sl)*.95)]:.0f} ms")


if __name__ == "__main__":
    main()
