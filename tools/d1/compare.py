#!/usr/bin/env python3
"""Compara duas execuções do benchmark: acerto, concordância e deriva de probabilidade."""
import json, sys, statistics

def load(path):
    return {json.loads(l)["id"]: json.loads(l) for l in open(path) if l.strip()}

def main(ref_path, cand_path):
    ref, cand = load(ref_path), load(cand_path)
    common = sorted(set(ref) & set(cand))
    by_cat = {}
    all_tv, all_noul, agree = [], [], 0
    for i in common:
        r, c = ref[i], cand[i]
        if "error" in r or "error" in c:
            continue
        cat = r["cat"]
        d = by_cat.setdefault(cat, {"n": 0, "agree": 0, "tv": [], "noul": []})
        d["n"] += 1
        ar, ac = r["answer"], c["answer"]
        if ar["type"] != ac["type"]:
            continue
        if ar["type"] == "choice":
            same = ar["choice"] == ac["choice"]
            keys = set(ar["probabilities"]) | set(ac["probabilities"])
            tv = 0.5 * sum(abs(ar["probabilities"].get(k, 0) - ac["probabilities"].get(k, 0)) for k in keys)
            d["tv"].append(tv); all_tv.append(tv)
            d["agree"] += int(same); agree += int(same)
        elif ar["type"] == "noul":
            diff = abs(ar["noul"] - ac["noul"])
            same = (ar["noul"] >= 0.5) == (ac["noul"] >= 0.5)
            d["noul"].append(diff); all_noul.append(diff)
            d["agree"] += int(same); agree += int(same)
        elif ar["type"] == "score":
            same = abs(ar["score"] - ac["score"]) <= 0.25
            d["agree"] += int(same); agree += int(same)

    print(f"referência: {ref_path}\ncandidato:  {cand_path}\n")
    print(f"{'cat':10s} {'n':>4s} {'argmax':>8s} {'TV média':>9s} {'|Δ noul|':>9s}")
    for cat, d in sorted(by_cat.items()):
        tv = statistics.mean(d["tv"]) if d["tv"] else 0
        nl = statistics.mean(d["noul"]) if d["noul"] else 0
        print(f"{cat:10s} {d['n']:4d} {d['agree']/d['n']:7.1%} {tv:9.3f} {nl:9.3f}")
    print(f"\nTOTAL concordância: {agree}/{len(common)} = {agree/len(common):.1%}")
    if all_tv: print(f"TV média (choice): {statistics.mean(all_tv):.4f}  p95 {sorted(all_tv)[int(len(all_tv)*0.95)]:.4f}")
    if all_noul: print(f"|Δ noul| média: {statistics.mean(all_noul):.4f}  p95 {sorted(all_noul)[int(len(all_noul)*0.95)]:.4f}")

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
