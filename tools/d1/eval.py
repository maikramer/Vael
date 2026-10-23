#!/usr/bin/env python3
"""Corre o benchmark contra um llama-server (/v1/systemone) e mede acerto/latência.

Uso: python3 eval.py --port 18080 --name q4_k_s [--limit N]
Grava preds_<name>.jsonl e imprime um resumo por categoria.
"""
import argparse, json, statistics, sys, time, urllib.request

BENCH = "bench.jsonl"


def post(port, state, questions, timeout=120):
    body = json.dumps({"state": state, "questions": questions}, ensure_ascii=False).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/systemone",
        data=body, headers={"Content-Type": "application/json"}, method="POST")
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        data = json.loads(r.read())
    return data, (time.perf_counter() - t0) * 1000.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--name", required=True)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--out", default=None)
    ap.add_argument("--bench", default=BENCH)
    args = ap.parse_args()

    items = [json.loads(l) for l in open(args.bench)]
    if args.limit:
        items = items[: args.limit]

    out_path = args.out or f"preds_{args.name}.jsonl"
    preds = []
    lat = []
    errors = 0
    with open(out_path, "w") as f:
        for i, it in enumerate(items):
            try:
                resp, ms = post(args.port, it["state"], it["questions"])
                ans = resp["answers"]["q"]
                rec = {"id": it["id"], "cat": it["cat"], "lang": it.get("lang", "pt"),
                       "expected": it["expected"], "answer": ans, "ms": ms,
                       "usage": resp.get("usage", {})}
                lat.append(ms)
            except Exception as e:
                errors += 1
                rec = {"id": it["id"], "cat": it["cat"], "lang": it.get("lang", "pt"),
                       "expected": it["expected"], "error": str(e)}
            preds.append(rec)
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
            if (i + 1) % 50 == 0:
                print(f"  {i+1}/{len(items)}", file=sys.stderr)

    by_cat = {}
    by_lang = {}
    for p in preds:
        c = by_cat.setdefault(p["cat"], {"n": 0, "ok": 0, "err": 0, "lat": [], "conf": []})
        l = by_lang.setdefault(p.get("lang", "pt"), {"n": 0, "ok": 0})
        c["n"] += 1; l["n"] += 1
        if "error" in p:
            c["err"] += 1
            continue
        c["lat"].append(p["ms"])
        a = p["answer"]
        if a["type"] == "choice":
            got = a["choice"]
            c["conf"].append(a["confidence"])
        elif a["type"] == "noul":
            got = a["noul"] >= 0.5
            c["conf"].append(max(a["noul"], 1 - a["noul"]))
        else:
            got = a["score"]
        c["ok"] += int(got == p["expected"])
        l["ok"] += int(got == p["expected"])

    print(f"\n== {args.name}  ({len(preds)} itens, {errors} erros) ==")
    total_ok = total_n = 0
    for cat, c in sorted(by_cat.items()):
        acc = c["ok"] / c["n"] if c["n"] else 0
        total_ok += c["ok"]; total_n += c["n"]
        p50 = statistics.median(c["lat"]) if c["lat"] else 0
        conf = statistics.mean(c["conf"]) if c["conf"] else 0
        print(f"  {cat:8s} acc {c['ok']:3d}/{c['n']:3d} = {acc:6.1%}   lat p50 {p50:6.1f} ms   conf {conf:.2f}   err {c['err']}")
    print(f"  TOTAL    acc {total_ok}/{total_n} = {total_ok/total_n:.1%}")
    for lang, l in sorted(by_lang.items()):
        print(f"    {lang}: {l['ok']}/{l['n']} = {l['ok']/l['n']:.1%}")
    if lat:
        sl = sorted(lat)
        print(f"  latência: p50 {sl[len(sl)//2]:.1f} ms  p95 {sl[int(len(sl)*0.95)]:.1f} ms  max {sl[-1]:.1f} ms")
    print(f"  -> {out_path}")


if __name__ == "__main__":
    main()
