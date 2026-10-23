#!/usr/bin/env python3
"""Poda o vocabulário de um GGUF d1-omni (F16) e o embedding correspondente.

Mantém: especiais, 256 bytes, tokens do corpus do Vael, o fecho de merges BPE
e, opcionalmente, os primeiros N tokens base (para robustez a texto novo).
Reescreve o GGUF com o tokenizer e o token_embd.weight fatiados.

Uso: python3 prune_vocab.py d1-omni.f16.gguf saida.gguf --keep-base 8192
"""
import argparse, glob, json, os
from collections import Counter

from tokenizers import Tokenizer, models

from gguf_patch import read_gguf, w_str, w_val

ROOT = os.environ.get("VAEL_ROOT", os.path.abspath(os.path.join(os.path.dirname(__file__), "../..")))
LAB = os.path.dirname(os.path.abspath(__file__))


def build_keep(keep_base):
    tj = json.load(open(f"{LAB}/tokenizer.json"))
    orig = Tokenizer.from_file(f"{LAB}/tokenizer.json")
    vocab = tj["model"]["vocab"]
    merges = tj["model"]["merges"]
    id2s = {i: s for s, i in vocab.items()}
    specials = {a["content"] for a in tj["added_tokens"]}
    for a in tj["added_tokens"]:
        id2s[a["id"]] = a["content"]

    texts = []
    for line in open(f"{LAB}/calib.txt", encoding="utf-8"):
        texts.append(line.rstrip("\n"))
    for path in ("bench.jsonl", "bench2.jsonl"):
        p = f"{LAB}/{path}"
        if not os.path.exists(p):
            continue
        for line in open(p, encoding="utf-8"):
            it = json.loads(line)
            texts.append(json.dumps(it["state"], ensure_ascii=False))
            for q in it["questions"].values():
                texts.append(json.dumps(q, ensure_ascii=False))
    for pat in ("dialogues/*.json", "quests/*.json", "data/*.json", "locale/*.json"):
        for p in glob.glob(f"{ROOT}/{pat}"):
            texts.append(open(p, encoding="utf-8").read())

    cnt = Counter()
    for t in texts:
        cnt.update(orig.encode(t, add_special_tokens=False).ids)
    corpus = {id2s[i] for i in cnt if i in id2s}

    def parts(m):
        return m.split(" ") if isinstance(m, str) else m

    merge_of = {}
    for m in merges:
        a, b = parts(m)
        merge_of[a + b] = (a, b)

    def closure(seed):
        keep = set(seed)
        stack = list(seed)
        while stack:
            t = stack.pop()
            if t in merge_of:
                for x in merge_of[t]:
                    if x not in keep:
                        keep.add(x)
                        stack.append(x)
        return keep

    singles = {s for s in vocab if len(s) == 1}
    # todo o bloco 0..508 fica (mesmo os type 1, como <|reserved_4|>), para
    # que bos/eos/pad/mask e os delimitadores do template não mudem de id
    block = {id2s[i] for i in range(509) if i in id2s}
    seeds = set(corpus) | specials | singles | block
    if keep_base > 0:
        for i in range(509, min(509 + keep_base, 64402)):
            seeds.add(id2s[i])
    keep = closure(seeds)
    keep = {s for s in keep if s in vocab or s in specials}

    # ids antigos mantidos, em ordem de id
    kept_ids = sorted(i for i, s in id2s.items() if s in keep)
    return tj, orig, texts, id2s, kept_ids, keep, merges, specials


def check_tokenizer(orig, texts, kept_ids, id2s, keep, merges):
    def parts(m):
        return m.split(" ") if isinstance(m, str) else m

    keep = set(keep)
    new_merges = [tuple(parts(m)) for m in merges if parts(m)[0] + parts(m)[1] in keep]
    new_vocab = {id2s[i]: n for n, i in enumerate(kept_ids)}
    if len(new_vocab) != len(kept_ids):
        raise ValueError("ids duplicados no vocabulário podado")
    tok = Tokenizer(models.BPE(vocab=new_vocab, merges=new_merges, unk_token=None,
                               fuse_unk=False, byte_fallback=False, ignore_merges=False))
    tok.pre_tokenizer = orig.pre_tokenizer
    tok.normalizer = orig.normalizer
    tok.decoder = orig.decoder
    total = same = diff_texts = 0
    for t in texts:
        a = orig.encode(t, add_special_tokens=False).tokens
        b = tok.encode(t, add_special_tokens=False).tokens
        total += len(a)
        same += sum(1 for x, y in zip(a, b) if x == y)
        if a != b:
            diff_texts += 1
    return same / total if total else 0, diff_texts, len(texts), tok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("input")
    ap.add_argument("output")
    ap.add_argument("--keep-base", type=int, default=0)
    args = ap.parse_args()

    tj, orig, texts, id2s, kept_ids, keep, merges, specials = build_keep(args.keep_base)
    ratio, diff_texts, n_texts, tok = check_tokenizer(orig, texts, kept_ids, id2s, keep, merges)
    print(f"vocab: 65536 -> {len(kept_ids)}  (keep_base={args.keep_base})")
    print(f"tokenização: {ratio:.4%} posições iguais, {diff_texts}/{n_texts} textos diferentes")
    if ratio < 0.999:
        print("AVISO: tokenização mudou; considera --keep-base maior")

    ver, kvs, tensors, align, data_start, d = read_gguf(args.input)
    _, toks_et, old_tokens = next(v for k, t, v in kvs if k == "tokenizer.ggml.tokens")
    _, types_et, old_types = next(v for k, t, v in kvs if k == "tokenizer.ggml.token_type")
    _, merges_et, old_merges = next(v for k, t, v in kvs if k == "tokenizer.ggml.merges")

    new_tokens = [old_tokens[i] for i in kept_ids]
    new_types = [old_types[i] for i in kept_ids]
    kept_set = {old_tokens[i] for i in kept_ids}

    def parts(m):
        return m.split(" ") if isinstance(m, str) else m

    new_merges = [m for m in old_merges if parts(m)[0] + parts(m)[1] in kept_set]
    print(f"merges: {len(old_merges)} -> {len(new_merges)}")

    old2new = {old: new for new, old in enumerate(kept_ids)}
    # metadata
    new_kvs = []
    for k, t, v in kvs:
        if k in ("tokenizer.ggml.bos_token_id", "tokenizer.ggml.eos_token_id",
                 "tokenizer.ggml.padding_token_id", "tokenizer.ggml.mask_token_id",
                 "tokenizer.ggml.prefix_token_id", "tokenizer.ggml.suffix_token_id",
                 "tokenizer.ggml.middle_token_id") and isinstance(v, int) and v in old2new:
            new_kvs.append((k, t, old2new[v]))
            continue
        if k == "tokenizer.ggml.tokens":
            v = ("array", toks_et, new_tokens)
        elif k == "tokenizer.ggml.token_type":
            v = ("array", types_et, new_types)
        elif k == "tokenizer.ggml.merges":
            v = ("array", merges_et, new_merges)
        elif k == "lfm2.vocab_size":
            v = len(kept_ids)
        new_kvs.append((k, t, v))

    # embedding fatiado
    emb = next(x for x in tensors if x["name"] == "token_embd.weight")
    assert emb["dims"] == [1024, 65536], emb["dims"]
    row = 1024 * 2  # f16
    emb_old = d[data_start + emb["off"]: data_start + emb["off"] + 65536 * row]
    emb_new = b"".join(emb_old[i * row:(i + 1) * row] for i in kept_ids)

    head = bytearray(b"GGUF")
    head += __import__("struct").pack("<IQQ", ver, len(tensors), len(new_kvs))
    for k, t, v in new_kvs:
        w_str(head, k)
        head += __import__("struct").pack("<I", t)
        head += w_val(None, t, v)

    old_off = [x["off"] for x in tensors]
    old_len = []
    for i, x in enumerate(tensors):
        end = old_off[i + 1] if i + 1 < len(tensors) else len(d) - data_start
        old_len.append(end - x["off"])

    infos = bytearray()
    cur = 0
    new_off = []
    for x, ln in zip(tensors, old_len):
        cur = (cur + align - 1) // align * align
        new_off.append(cur)
        dims = [1024, len(kept_ids)] if x["name"] == "token_embd.weight" else x["dims"]
        w_str(infos, x["name"])
        infos += __import__("struct").pack("<I", len(dims))
        for dim in dims:
            infos += __import__("struct").pack("<Q", dim)
        infos += __import__("struct").pack("<IQ", x["type"], cur)
        cur += len(emb_new) if x["name"] == "token_embd.weight" else ln

    pre = bytes(head) + bytes(infos)
    pre += b"\0" * ((-len(pre)) % align)
    data = bytearray()
    for x, ln, off in zip(tensors, old_len, new_off):
        seg = emb_new if x["name"] == "token_embd.weight" else d[data_start + x["off"]: data_start + x["off"] + ln]
        if len(data) < off:
            data += b"\0" * (off - len(data))
        data += seg
    if len(data) < cur:
        data += b"\0" * (cur - len(data))

    with open(args.output, "wb") as f:
        f.write(pre)
        f.write(data)
    print(f"{args.input} -> {args.output}: {len(d)/1e6:.1f} MB -> {(len(pre)+len(data))/1e6:.1f} MB")


if __name__ == "__main__":
    main()
