#!/usr/bin/env python3
"""Gera a tabela final (tamanho, acerto, fidelidade) a partir das previsões."""
import json, os, statistics

LAB = os.environ.get("D1_LAB", os.path.dirname(os.path.abspath(__file__)))
RES = os.path.join(LAB, "results") if os.path.isdir(os.path.join(LAB, "results")) else LAB

# tamanhos no disco (bytes) para a tabela funcionar mesmo sem os GGUFs presentes
SIZES = {
    "d1-omni-600M.Q8_0.gguf": 407_200_000,
    "d1-omni-600M.Q4_K_S.gguf": 235_600_000,
    "q4im.gguf": 235_600_000,
    "q3mmix.gguf": 212_000_000,
    "q3smix.gguf": 200_000_000,
    "iq3mix.gguf": 178_000_000,
    "iq2mix.gguf": 162_000_000,
    "q4_safe.gguf": 202_000_000,
    "q4_vocab.gguf": 195_000_000,
    "q3m_vocab.gguf": 167_000_000,
    "iq3_vocab.gguf": 142_000_000,
    "prune4_iq3_fix.gguf": 150_000_000,
}

MODELS = [
    # nome, ficheiro, preds v2, preds v1
    ("Q8_0 completo", "d1-omni-600M.Q8_0.gguf", "preds_q8_b2.jsonl", "preds_q8_0.jsonl"),
    ("Q4_K_S completo", "d1-omni-600M.Q4_K_S.gguf", "preds_q4hf.jsonl", "preds_q4_k_s.jsonl"),
    ("Q4_K_S + imatrix", "q4im.gguf", "preds_q4im.jsonl", None),
    ("Q3_K_M MIX", "q3mmix.gguf", "preds_q3mmix.jsonl", None),
    ("Q3_K_S MIX", "q3smix.gguf", "preds_q3smix.jsonl", None),
    ("IQ3_XXS MIX", "iq3mix.gguf", "preds_iq3mix.jsonl", "preds_iq3mix_v1.jsonl"),
    ("IQ2_M MIX", "iq2mix.gguf", "preds_iq2mix.jsonl", None),
    ("Q4 safe vocab", "q4_safe.gguf", "preds_q4_safe.jsonl", None),
    ("Q4 small vocab", "q4_vocab.gguf", "preds_q4_vocab.jsonl", "preds_q4_vocab_v1.jsonl"),
    ("Q3_K_M small vocab", "q3m_vocab.gguf", "preds_q3m_vocab.jsonl", None),
    ("IQ3_XXS small vocab", "iq3_vocab.gguf", "preds_iq3_vocab.jsonl", None),
    ("Poda 4 camadas IQ3", "prune4_iq3_fix.gguf", "preds_prune4_iq3_fix.jsonl", None),
]

REF = "preds_q8_b2.jsonl"


def resolve(name):
    for base in (LAB, RES, os.path.join(LAB, name)):
        p = os.path.join(base, name)
        if os.path.exists(p):
            return p
    return None


def load(path):
    p = resolve(path) if path else None
    if not p:
        return None
    return {json.loads(l)["id"]: json.loads(l) for l in open(p) if l.strip()}


def metrics(preds, ref):
    ok = n = 0
    agree = nn = 0
    tv = []
    dn = []
    for i, p in preds.items():
        if "error" in p:
            continue
        a = p["answer"]
        n += 1
        if a["type"] == "choice":
            got = a["choice"]
        elif a["type"] == "noul":
            got = a["noul"] >= 0.5
        else:
            got = a["score"]
        ok += int(got == p["expected"])
        r = ref.get(i) if ref else None
        if r and "error" not in r and r["answer"]["type"] == a["type"]:
            nn += 1
            if a["type"] == "choice":
                agree += int(a["choice"] == r["answer"]["choice"])
                keys = set(a["probabilities"]) | set(r["answer"]["probabilities"])
                tv.append(0.5 * sum(abs(a["probabilities"].get(k, 0) - r["answer"]["probabilities"].get(k, 0)) for k in keys))
            elif a["type"] == "noul":
                agree += int((a["noul"] >= 0.5) == (r["answer"]["noul"] >= 0.5))
                dn.append(abs(a["noul"] - r["answer"]["noul"]))
            else:
                agree += int(abs(a["score"] - r["answer"]["score"]) <= 0.25)
    return {
        "acc": ok / n if n else 0, "n": n,
        "agree": agree / nn if nn else None,
        "tv": statistics.mean(tv) if tv else None,
        "dn": statistics.mean(dn) if dn else None,
    }


def v1_agreement(preds):
    ref = load("preds_q8_0.jsonl")
    if not preds or not ref:
        return None
    a = metrics(preds, ref)
    return a["agree"]


def main():
    ref2 = load(REF)
    print("| Modelo | Disco | vs Q4 | v2 acc | v2 = Q8 | TV | Δ noul | v1 = Q8 |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    base = SIZES["d1-omni-600M.Q4_K_S.gguf"]
    for name, gguf, p2, p1 in MODELS:
        path = resolve(gguf)
        size = os.path.getsize(path) if path else SIZES.get(gguf)
        m = metrics(load(p2) or {}, ref2)
        a1 = v1_agreement(load(p1))
        sz = f"{size/1e6:.0f} MB" if size else "—"
        rel = f"{(size/base-1)*100:+.0f}%" if size else "—"
        agree = f"{m['agree']:.1%}" if m["agree"] is not None else "—"
        tv = f"{m['tv']:.3f}" if m["tv"] is not None else "—"
        dn = f"{m['dn']:.3f}" if m["dn"] is not None else "—"
        a1s = f"{a1:.1%}" if a1 is not None else "—"
        print(f"| {name} | {sz} | {rel} | {m['acc']:.1%} | {agree} | {tv} | {dn} | {a1s} |")


if __name__ == "__main__":
    main()
