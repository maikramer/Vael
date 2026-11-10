#!/usr/bin/env python3
"""Lint do CONTEÚDO do Vael (JSON + referências cruzadas). Exit 1 em erro.

Apanha o que o `viber analyze` não vê, porque são regras do jogo (Lua):
  - recompensas, custos e produtos que apontam para itens inexistentes;
  - objetivos de quest com alvo desconhecido (criatura, item, marco);
  - `requires`/`next` para quests que não existem, e ciclos de cadeia;
  - dador (`npc`) sem entidade no mundo;
  - talentos com `requires`/`excludes` desconhecidos ou assimétricos;
  - protótipos `mob-*` pedidos pelos scripts sem `<Prototype>` no mundo;
  - chaves de locale presentes num idioma e ausentes no outro.

  python3 tools/lint_content.py        # a partir da raiz do jogo
"""
import glob, json, re, sys

errors, warns = [], []
def err(msg): errors.append(msg)
def warn(msg): warns.append(msg)

def load(path):
    try:
        return json.load(open(path, encoding='utf-8'))
    except Exception as e:  # noqa: BLE001 — o lint reporta, não rebenta
        err(f"{path}: JSON inválido ({e})")
        return None

def read_all(patterns):
    out = ''
    for pat in patterns:
        for path in glob.glob(pat, recursive=True):
            out += open(path, encoding='utf-8', errors='replace').read()
    return out

items_doc = load('data/items.json') or {}
items = {i['id']: i for i in items_doc.get('items', []) if isinstance(i, dict) and i.get('id')}
SLOTS = {'weapon', 'armor', 'charm'}
for iid, item in items.items():
    slot = item.get('slot')
    if slot is not None and slot not in SLOTS:
        err(f"items: {iid}: slot desconhecido '{slot}'")
    if slot == 'weapon' and not isinstance(item.get('weapon'), dict):
        err(f"items: {iid}: slot weapon sem bloco `weapon`")
    for key, value in (item.get('effects') or {}).items():
        if not isinstance(value, (int, float)):
            err(f"items: {iid}: efeito '{key}' não numérico")
for iid in (items_doc.get('starting') or {}):
    if iid not in items:
        err(f"items: kit inicial com item desconhecido '{iid}'")

recipes = (load('data/recipes.json') or {}).get('recipes', [])
for r in recipes:
    rid = r.get('id', '?')
    if r.get('out') not in items:
        err(f"recipes: {rid}: produto desconhecido '{r.get('out')}'")
    for iid, n in (r.get('cost') or {}).items():
        if iid not in items:
            err(f"recipes: {rid}: custo com item desconhecido '{iid}'")
        if not isinstance(n, (int, float)) or n <= 0:
            err(f"recipes: {rid}: quantidade inválida para '{iid}'")
craftable = {r.get('out') for r in recipes}

prog = load('data/progression.json') or {}
skills = {s['id']: s for s in prog.get('skills', []) if isinstance(s, dict) and s.get('id')}
for sid, s in skills.items():
    req, exc = s.get('requires') or '', s.get('excludes') or ''
    if req and req not in skills:
        err(f"progression: {sid}: requires desconhecido '{req}'")
    if exc:
        if exc not in skills:
            err(f"progression: {sid}: excludes desconhecido '{exc}'")
        elif (skills[exc].get('excludes') or '') != sid:
            err(f"progression: {sid} exclui {exc}, mas {exc} não exclui {sid}")

# Criaturas: o `kind` de cada script de inimigo/chefe.
scripts = {p: open(p, encoding='utf-8').read() for p in glob.glob('scripts/**/*.lua', recursive=True)}
kinds = set()
for path, src in scripts.items():
    if '/enemies/' in path or '/bosses/' in path:
        kinds.update(re.findall(r'\bkind\s*=\s*"([^"]+)"', src))

world_xml = read_all(['world.xml', 'world/**/*.xml', 'scenes/**/*.xml'])
entity_names = set(re.findall(r'\bname="([^"]+)"', world_xml))
prototypes = set(re.findall(r'<Prototype\s+id="([^"]+)"', world_xml))
for path, src in scripts.items():
    for proto in set(re.findall(r'"(mob-[a-z0-9-]+)"', src)):
        if proto not in prototypes:
            err(f"{path}: protótipo '{proto}' sem <Prototype> no mundo")

def kill_ok(target):
    return target == 'elite' or target in kinds or any(k.startswith(target + '-') for k in kinds)

quests = {}
index = load('quests/index.json') or []
for name in index:
    doc = load('quests/' + name)
    for q in doc or []:
        if not isinstance(q, dict) or not q.get('id'):
            err(f"quests/{name}: entrada sem id")
            continue
        if q['id'] in quests:
            err(f"quests/{name}: id repetido '{q['id']}'")
        quests[q['id']] = q
for path in glob.glob('quests/*_quests.json'):
    if path.split('/')[-1] not in index:
        warn(f"{path}: fora do quests/index.json (não é carregado)")

def check_objective(qid, obj, where):
    kind, target = obj.get('type'), str(obj.get('target') or '')
    if kind == 'kill':
        if not kill_ok(target):
            err(f"quest {qid}: {where}: criatura desconhecida '{target}'")
    elif kind in ('collect', 'craft'):
        if target not in items:
            err(f"quest {qid}: {where}: item desconhecido '{target}'")
        if kind == 'craft' and target not in craftable:
            err(f"quest {qid}: {where}: '{target}' não tem receita")
    elif kind == 'visit':
        for token in target.split():
            if token not in entity_names:
                err(f"quest {qid}: {where}: marco '{token}' sem entidade no mundo")
    elif kind != 'decision':
        err(f"quest {qid}: {where}: tipo desconhecido '{kind}'")
    count = obj.get('count', 1)
    if not isinstance(count, (int, float)) or count < 1:
        err(f"quest {qid}: {where}: count inválido")

for qid, q in quests.items():
    stages = q.get('stages')
    if stages is not None:
        if not isinstance(stages, list) or not stages:
            err(f"quest {qid}: stages vazio")
        for i, stage in enumerate(stages or []):
            check_objective(qid, stage, f"etapa {i + 1}")
    elif isinstance(q.get('objective'), dict):
        check_objective(qid, q['objective'], 'objetivo')
    else:
        err(f"quest {qid}: sem objective nem stages")
    rewards = q.get('rewards') or {}
    for entry in rewards.get('items') or []:
        name = str(entry).split(':')[0].strip()
        if name not in items:
            err(f"quest {qid}: recompensa com item desconhecido '{name}'")
    req = q.get('requires')
    for other in ([req] if isinstance(req, str) and req else (req or [])):
        if other not in quests:
            err(f"quest {qid}: requires desconhecido '{other}'")
    nxt = q.get('next')
    if nxt:
        if nxt not in quests:
            err(f"quest {qid}: next desconhecido '{nxt}'")
        elif quests[nxt].get('npc') != q.get('npc'):
            err(f"quest {qid}: next '{nxt}' tem outro dador (a cadeia é do mesmo NPC)")
    npc = q.get('npc')
    if npc and npc != 'notice_board' and npc not in entity_names:
        err(f"quest {qid}: dador '{npc}' sem entidade no mundo")
for qid in quests:
    seen, cur = set(), qid
    while cur and cur in quests:
        if cur in seen:
            err(f"quest {qid}: ciclo na cadeia `next`")
            break
        seen.add(cur)
        cur = quests[cur].get('next')

pt, en = load('locale/pt.json') or {}, load('locale/en.json') or {}
for key in sorted(set(pt) - set(en)):
    err(f"locale: '{key}' existe em pt.json e falta em en.json")
for key in sorted(set(en) - set(pt)):
    err(f"locale: '{key}' existe em en.json e falta em pt.json")
ui_xml = read_all(['world/*.xml', 'world/autoload/*.xml'])
for key in sorted(set(re.findall(r'>@([A-Za-z0-9_.]+)<', ui_xml))):
    if key not in pt:
        err(f"UI: chave '@{key}' sem entrada em locale/pt.json")

for w in warns:
    print('aviso:', w)
for e in errors:
    print('ERRO:', e)
print(f"conteúdo: {len(items)} itens, {len(recipes)} receitas, {len(skills)} talentos, "
      f"{len(quests)} quests, {len(kinds)} criaturas — {len(errors)} erro(s), {len(warns)} aviso(s)")
sys.exit(1 if errors else 0)
