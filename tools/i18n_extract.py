#!/usr/bin/env python3
"""Extrai as frases PT mostradas ao jogador pelos scripts e diz o que falta
traduzir em locale/strings_en.json (lido por scripts/lib/i18n.lua).

Apanha os literais de `viber.toast`, `viber.say`, `viber.set_interaction`,
`viber.prompt` e `viber.damage_number`. Uma frase montada com `..` aparece
partida: essas traduzem-se com uma entrada em `patterns` (padrão Lua).

  python3 tools/i18n_extract.py            # lista o que falta (exit 1 se faltar)
  python3 tools/i18n_extract.py --all      # lista tudo o que foi encontrado
"""
import glob, json, re, sys

CALL = re.compile(r'viber\.(toast|say|set_interaction|prompt|damage_number)\s*\(')
LIT = re.compile(r'"((?:[^"\\]|\\.)*)"')

def call_args(src, start):
    depth, i = 1, start
    while i < len(src) and depth:
        c = src[i]
        if c == '"':
            m = LIT.match(src, i)
            i = m.end() if m else i + 1
            continue
        depth += c == '('
        depth -= c == ')'
        i += 1
    return src[start:i - 1]

def phrases():
    found = {}
    for path in sorted(glob.glob('scripts/**/*.lua', recursive=True)):
        src = open(path, encoding='utf-8').read()
        for m in CALL.finditer(src):
            args = call_args(src, m.end())
            lits = [l for l in LIT.findall(args) if re.search(r'[A-Za-zÀ-ÿ]{3}', l)]
            # cores e nomes de clip não são texto para o jogador
            lits = [l for l in lits if not l.startswith('#') and ' ' in l or len(l) > 12]
            if not lits:
                continue
            key = lits[0] if ('..' not in args and len(lits) == 1) else None
            found.setdefault(path, []).append((key, lits))
    return found

def main():
    data = json.load(open('locale/strings_en.json', encoding='utf-8'))
    exact = data.get('exact', {})
    pats = [re.compile(lua_to_re(p)) for p, _ in data.get('patterns', [])]
    plains = [re.sub(r'%(.)', r'\1', p).lstrip('^') for p, _ in data.get('patterns', [])]
    missing = 0
    for path, items in phrases().items():
        for key, lits in items:
            if key is not None:
                ok = key in exact or any(p.fullmatch(key) for p in pats)
            else:
                # frase composta: basta o início de um padrão coincidir com o
                # 1.º fragmento (heurística — é um lint, não uma prova)
                frag = lits[0].strip()[:10]
                ok = any(plain.startswith(frag) or (frag and frag in plain) for plain in plains) \
                    or all(l in exact for l in lits)
            if '--all' in sys.argv or not ok:
                print(f"{'  ' if ok else '! '}{path}: {key if key is not None else ' .. '.join(lits)}")
            missing += not ok
    print(f"{missing} frase(s) sem tradução EN")
    return 1 if missing else 0

def lua_to_re(p):
    """Padrão Lua (subconjunto usado: ^ $ %d %s %a . + - * ( ) e escapes %x)."""
    out, i = '', 0
    while i < len(p):
        c = p[i]
        if c == '%' and i + 1 < len(p):
            n = p[i + 1]
            out += {'d': r'\d', 's': r'\s', 'a': r'[A-Za-zÀ-ÿ]', 'w': r'\w'}.get(n, re.escape(n))
            i += 2
            continue
        if c == '-':
            out += '*?'
        elif c in '^$.+*()[]':
            out += c
        else:
            out += re.escape(c)
        i += 1
    return out

if __name__ == '__main__':
    sys.exit(main())
