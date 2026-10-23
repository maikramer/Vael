#!/usr/bin/env python3
"""Ajusta metadata de um GGUF podado: remove índices dos arrays por camada.

O `llama-quantize --prune-layers` renumera os tensores e atualiza block_count,
mas deixa `feed_forward_length` e `attention.head_count_kv` com o comprimento
antigo. Este script reescreve o GGUF com os arrays certos, copiando os tensores
tal como estão (quantizados) e recomputando os offsets.

Uso: python3 gguf_patch.py entrada.gguf saida.gguf 1,4,7,9
"""
import struct, sys

# ---- leitura

class Reader:
    def __init__(self, data):
        self.d = data
        self.o = 0

    def u(self, fmt):
        v = struct.unpack_from(fmt, self.d, self.o)[0]
        self.o += struct.calcsize(fmt)
        return v

    def s(self):
        n = self.u("<Q")
        b = self.d[self.o:self.o + n]
        self.o += n
        return b.decode("utf-8", "replace")

    def val(self, t):
        if t == 0: return self.u("<B")
        if t == 1: return self.u("<b")
        if t == 2: return self.u("<H")
        if t == 3: return self.u("<h")
        if t == 4: return self.u("<I")
        if t == 5: return self.u("<i")
        if t == 6: return self.u("<f")
        if t == 7: return self.u("<B") != 0
        if t == 8: return self.s()
        if t == 9:
            et = self.u("<I")
            n = self.u("<Q")
            return ("array", et, [self.val(et) for _ in range(n)])
        if t == 10: return self.u("<Q")
        if t == 11: return self.u("<q")
        if t == 12: return self.u("<d")
        raise ValueError(f"tipo GGUF desconhecido {t} em {self.o}")


def read_gguf(path):
    d = open(path, "rb").read()
    r = Reader(d)
    if d[:4] != b"GGUF":
        raise ValueError("não é GGUF")
    r.o = 4
    ver = r.u("<I")
    nt = r.u("<Q")
    nkv = r.u("<Q")
    kvs = []
    for _ in range(nkv):
        k = r.s()
        t = r.u("<I")
        kvs.append((k, t, r.val(t)))
    tensors = []
    for _ in range(nt):
        name = r.s()
        nd = r.u("<I")
        dims = [r.u("<Q") for _ in range(nd)]
        t = r.u("<I")
        off = r.u("<Q")
        tensors.append({"name": name, "dims": dims, "type": t, "off": off})
    align = 32
    for k, t, v in kvs:
        if k == "general.alignment" and isinstance(v, int):
            align = v
    data_start = (r.o + align - 1) // align * align
    return ver, kvs, tensors, align, data_start, d


# ---- escrita

def w_str(out, s):
    b = s.encode("utf-8")
    out += struct.pack("<Q", len(b)) + b


def w_val(out, t, v):
    if t == 0: return struct.pack("<B", v)
    if t == 1: return struct.pack("<b", v)
    if t == 2: return struct.pack("<H", v)
    if t == 3: return struct.pack("<h", v)
    if t == 4: return struct.pack("<I", v)
    if t == 5: return struct.pack("<i", v)
    if t == 6: return struct.pack("<f", v)
    if t == 7: return struct.pack("<B", 1 if v else 0)
    if t == 8:
        b = v.encode("utf-8")
        return struct.pack("<Q", len(b)) + b
    if t == 9:
        _, et, vals = v
        return struct.pack("<IQ", et, len(vals)) + b"".join(w_val(None, et, x) for x in vals)
    if t == 10: return struct.pack("<Q", v)
    if t == 11: return struct.pack("<q", v)
    if t == 12: return struct.pack("<d", v)
    raise ValueError(f"tipo GGUF desconhecido {t}")


def main(inp, outp, prune_csv):
    prune = sorted(int(x) for x in prune_csv.split(",") if x.strip())
    ver, kvs, tensors, align, data_start, d = read_gguf(inp)
    # remove os índices podados dos arrays por camada
    patched = []
    for k, t, v in kvs:
        if t == 9 and k in ("lfm2.feed_forward_length", "lfm2.attention.head_count_kv"):
            _, et, vals = v
            vals = [x for i, x in enumerate(vals) if i not in prune]
            v = ("array", et, vals)
            print(f"{k}: {len(vals)} entradas")
        patched.append((k, t, v))
    kvs = patched

    # header + metadata
    head = bytearray(b"GGUF")
    head += struct.pack("<IQQ", ver, len(tensors), len(kvs))
    for k, t, v in kvs:
        w_str(head, k)
        head += struct.pack("<I", t)
        head += w_val(None, t, v)

    # offsets novos: copia cada segmento antigo (inclui padding) alinhado
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
        w_str(infos, x["name"])
        infos += struct.pack("<I", len(x["dims"]))
        for dim in x["dims"]:
            infos += struct.pack("<Q", dim)
        infos += struct.pack("<IQ", x["type"], cur)
        cur += ln

    pre = bytes(head) + bytes(infos)
    pre += b"\0" * ((-len(pre)) % align)
    data = bytearray()
    for x, ln, off in zip(tensors, old_len, new_off):
        seg = d[data_start + x["off"]: data_start + x["off"] + ln]
        if len(data) < off:
            data += b"\0" * (off - len(data))
        data += seg
    if len(data) < cur:
        data += b"\0" * (cur - len(data))

    with open(outp, "wb") as f:
        f.write(pre)
        f.write(data)
    print(f"{inp} -> {outp}: {len(d)/1e6:.1f} MB -> {(len(pre)+len(data))/1e6:.1f} MB")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3])
