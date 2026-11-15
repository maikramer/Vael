#!/usr/bin/env python3
"""Lint do CONTEÚDO do Vael (JSON + referências cruzadas). Exit 1 em erro.

Apanha o que o `vael-tools analyze` não vê, porque são regras do jogo (Lua):
  - recompensas, custos e produtos que apontam para itens inexistentes;
  - objetivos de quest com alvo desconhecido (criatura, item, marco);
  - `requires`/`next` para quests que não existem, e ciclos de cadeia;
  - dador (`npc`) sem entidade no mundo;
  - talentos com `requires`/`excludes` desconhecidos ou assimétricos;
  - protótipos `mob-*` pedidos pelos scripts sem `<Prototype>` no mundo;
  - chaves de locale presentes num idioma e ausentes no outro.

  python3 tools/lint_content.py        # a partir da raiz do jogo
"""
import glob, json, os, re, shutil, subprocess, sys

errors, warns = [], []
def err(msg): errors.append(msg)
def warn(msg): warns.append(msg)

def load(path):
    try:
        return json.load(open(path, encoding='utf-8'))
    except Exception as e:  # noqa: BLE001 — o lint reporta, não rebenta
        err(f"{path}: JSON inválido ({e})")
        return None

items_doc = load('data/items.json') or {}
items = {i['id']: i for i in items_doc.get('items', []) if isinstance(i, dict) and i.get('id')}
stack_max = items_doc.get('stack_max', 99)
resource_cap = items_doc.get('resource_cap', 1000000)
seen_item_ids = set()
for raw in items_doc.get('items', []):
    if not isinstance(raw, dict):
        continue
    iid = raw.get('id')
    if not iid:
        continue
    if iid in seen_item_ids:
        err(f"items: id repetido '{iid}'")
    seen_item_ids.add(iid)
SLOTS = {'weapon', 'armor', 'charm'}

def cap_of(iid):
    """Tecto da pilha como `src/vault.rs`/`economy.rs` (nunca negativo)."""
    item = items.get(iid) or {}
    if item.get('kind') == 'resource':
        return max(0, resource_cap)
    stack = item.get('stack')
    if isinstance(stack, int) and stack > 0:
        return stack
    return max(0, stack_max)

for iid, item in items.items():
    slot = item.get('slot')
    if slot is not None and slot not in SLOTS:
        err(f"items: {iid}: slot desconhecido '{slot}'")
    if slot == 'weapon' and not isinstance(item.get('weapon'), dict):
        err(f"items: {iid}: slot weapon sem bloco `weapon`")
    if item.get('weapon') is not None and slot != 'weapon':
        err(f"items: {iid}: bloco `weapon` sem slot weapon")
    for key, value in (item.get('effects') or {}).items():
        if not isinstance(value, (int, float)):
            err(f"items: {iid}: efeito '{key}' não numérico")
    shop = item.get('shop') or {}
    for key in ('buy', 'sell'):
        price = shop.get(key)
        if price is not None and (not isinstance(price, int) or price < 0):
            err(f"items: {iid}: preço `{key}` inválido ({price})")
    stack = item.get('stack')
    if stack is not None and (not isinstance(stack, int) or stack < 1
                              or (stack_max > 0 and stack > stack_max)):
        err(f"items: {iid}: `stack` inválido ({stack}); teto global {stack_max}")
    heal = (item.get('use') or {}).get('heal')
    if heal is not None and (not isinstance(heal, (int, float)) or heal < 0):
        err(f"items: {iid}: `use.heal` inválido ({heal})")
for iid in (items_doc.get('starting') or {}):
    if iid not in items:
        err(f"items: kit inicial com item desconhecido '{iid}'")

recipes = (load('data/recipes.json') or {}).get('recipes', [])
seen_recipes = set()
for r in recipes:
    rid = r.get('id', '?')
    if not r.get('id'):
        err(f"recipes: receita sem id (out='{r.get('out')}')")
    elif r.get('id') in seen_recipes:
        err(f"recipes: id repetido '{rid}'")
    seen_recipes.add(r.get('id'))
    if r.get('out') not in items:
        err(f"recipes: {rid}: produto desconhecido '{r.get('out')}'")
    count = r.get('count', 1)
    if not isinstance(count, int) or count < 1:
        err(f"recipes: {rid}: quantidade produzida inválida ({count})")
    elif r.get('out') in items and count > cap_of(r.get('out')):
        err(f"recipes: {rid}: produz {count}× acima do tecto de '{r.get('out')}'")
    for iid, n in (r.get('cost') or {}).items():
        if iid not in items:
            err(f"recipes: {rid}: custo com item desconhecido '{iid}'")
        if not isinstance(n, (int, float)) or n <= 0:
            err(f"recipes: {rid}: quantidade inválida para '{iid}'")
        elif iid in items and n > cap_of(iid):
            err(f"recipes: {rid}: custo {n}× acima do tecto de '{iid}'")
craftable = {r.get('out') for r in recipes}

prog = load('data/progression.json') or {}
skills = {s['id']: s for s in prog.get('skills', []) if isinstance(s, dict) and s.get('id')}
seen_skills = set()
for raw in prog.get('skills', []):
    if not isinstance(raw, dict) or not raw.get('id'):
        continue
    sid = raw['id']
    if sid in seen_skills:
        err(f"progression: id de talento repetido '{sid}'")
    seen_skills.add(sid)
    for key, value in (raw.get('effects') or {}).items():
        if not isinstance(value, (int, float)):
            err(f"progression: {sid}: efeito '{key}' não numérico")
xp = prog.get('xp') or {}
if not isinstance(xp.get('base'), (int, float)) or xp.get('base', 0) < 1:
    err(f"progression: xp.base tem de ser >= 1 ({xp.get('base')})")
if not isinstance(xp.get('growth'), (int, float)) or xp.get('growth', 0) <= 1:
    err(f"progression: xp.growth tem de ser > 1 ({xp.get('growth')})")
for kind, value in (prog.get('kill_xp') or {}).items():
    if not isinstance(value, (int, float)) or value < 0:
        err(f"progression: kill_xp.{kind} inválido ({value})")
for sid, s in skills.items():
    req, exc = s.get('requires') or '', s.get('excludes') or ''
    if req and req not in skills:
        err(f"progression: {sid}: requires desconhecido '{req}'")
    if exc:
        if exc not in skills:
            err(f"progression: {sid}: excludes desconhecido '{exc}'")
        elif (skills[exc].get('excludes') or '') != sid:
            err(f"progression: {sid} exclui {exc}, mas {exc} não exclui {sid}")

landmarks_doc = load('data/landmarks.json') or {}
landmark_names = {
    m['name'] for m in landmarks_doc.get('landmarks', [])
    if isinstance(m, dict) and m.get('name')
}
aliases_doc = load('data/nlu_aliases.json') or {}
creature_aliases = set((aliases_doc.get('creatures') or {}).keys())

# Criaturas: o `kind` de cada script de inimigo/chefe.
scripts = {p: open(p, encoding='utf-8').read() for p in glob.glob('scripts/**/*.lua', recursive=True)}
kinds = set()
for path, src in scripts.items():
    if '/enemies/' in path or '/bosses/' in path:
        kinds.update(re.findall(r'\bkind\s*=\s*"([^"]+)"', src))

# Aliases autorais têm de apontar para conteúdo que existe: um id morto no
# `nlu_aliases.json` nunca resolve e a criatura/item/local fica inominável.
PSEUDO_ITEM_IDS = {'torch', 'sword'}  # kinds de game/equipment.lua
for cid in sorted(creature_aliases - kinds):
    err(f"nlu_aliases: criatura '{cid}' sem script de inimigo/chefe")
for kind in sorted(kinds - creature_aliases):
    err(f"nlu_aliases: criatura '{kind}' sem aliases (o NLU não a reconhece)")
for iid in sorted(set((aliases_doc.get('items') or {})) - set(items) - PSEUDO_ITEM_IDS):
    err(f"nlu_aliases: item '{iid}' não existe em items.json")
for pid in sorted(set((aliases_doc.get('places') or {})) - landmark_names):
    err(f"nlu_aliases: local '{pid}' não é um marco de landmarks.json")

def tools_bin():
    """`vael-tools`: env, `target/{release,debug}` ou `cargo run`."""
    exe = os.environ.get('VAEL_TOOLS_BIN')
    if exe:
        return [exe]
    for profile in ('release', 'debug'):
        path = os.path.join('target', profile, 'vael-tools')
        if os.path.isfile(path):
            return [path]
    if shutil.which('vael-tools'):
        return ['vael-tools']
    return ['cargo', 'run', '-q', '-p', 'vael-tools', '--']


def analyze_world():
    """Nomes/protótipos do mundo pelo `vael-tools analyze --json`."""
    cmd = [*tools_bin(), 'analyze', 'world.bsn', '--json']
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except FileNotFoundError:
        warn(f'{cmd[0]} ausente; referências ao mundo não verificadas')
        return None
    except Exception as e:  # noqa: BLE001 — o lint reporta, não rebenta
        err(f'analyze do mundo: {e}')
        return None
    if out.returncode != 0:
        err(f'analyze do mundo: exit {out.returncode}: {out.stderr.strip()[:300]}')
        return None
    try:
        return json.loads(out.stdout)
    except Exception as e:  # noqa: BLE001
        err(f'analyze do mundo: JSON inválido ({e})')
        return None

world = analyze_world()
entity_names, prototypes = set(), set()
if world is not None:
    entity_names = set(world.get('names') or [])
    prototypes = set(world.get('prototype_ids') or [])
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
for name in index:
    if not isinstance(name, str) or not name:
        err("quests/index.json: entrada vazia")
    elif not os.path.isfile(os.path.join('quests', name)):
        err(f"quests/index.json: '{name}' não existe em quests/")

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
    elif kind == 'visit' and count > len(target.split()):
        # O `scan_visits` conta no máximo um marco por token: uma meta acima
        # dos locais listados nunca fecha.
        err(f"quest {qid}: {where}: count {count} > {len(target.split())} marcos listados")
    elif kind == 'collect' and target in items and count > cap_of(target):
        # O cofre para no tecto da pilha: recolher mais do que isso é impossível.
        err(f"quest {qid}: {where}: count {count} acima do tecto de '{target}'")

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

def requires_of(q):
    req = q.get('requires')
    if isinstance(req, str):
        return [req] if req else []
    return req if isinstance(req, list) else []

def has_requires_cycle(start):
    def walk(node, path):
        if node in path:
            return True
        for other in requires_of(quests.get(node) or {}):
            if other in quests and walk(other, path | {node}):
                return True
        return False
    return walk(start, set())

for qid, q in quests.items():
    if has_requires_cycle(qid):
        err(f"quest {qid}: ciclo em `requires` (a quest nunca fica disponível)")
    rewards = q.get('rewards')
    if not isinstance(rewards, dict):
        err(f"quest {qid}: rewards malformado")
        rewards = {}
    for key in ('gold', 'xp'):
        value = rewards.get(key, 0)
        if not isinstance(value, (int, float)) or value < 0:
            err(f"quest {qid}: recompensa `{key}` inválida ({value})")
    parley = q.get('parley')
    if parley is not None:
        report = parley.get('report') if isinstance(parley, dict) else None
        if not isinstance(report, dict):
            err(f"quest {qid}: parley sem report")
            continue
        expect = report.get('expect')
        if not isinstance(expect, list) or not expect:
            err(f"quest {qid}: parley.report.expect vazio")
        for e in expect or []:
            if not isinstance(e, dict):
                err(f"quest {qid}: parley.report.expect malformado")
                continue
            kind, eid = e.get('kind'), e.get('id')
            if kind not in ('criatura', 'item', 'local'):
                err(f"quest {qid}: parley com kind inválido '{kind}'")
            elif kind == 'item' and eid not in items:
                err(f"quest {qid}: parley espera item desconhecido '{eid}'")
            elif kind == 'criatura' and eid not in creature_aliases and eid not in kinds:
                err(f"quest {qid}: parley espera criatura sem alias '{eid}'")
            elif kind == 'local' and eid != '*' and eid not in landmark_names:
                err(f"quest {qid}: parley espera local desconhecido '{eid}'")

# Marcos que uma quest de visita chega a registar no `actions.lua`
# (`report_visit` só corre para tokens de objetivos `visit` ativos). É o
# único caminho que o juiz (`data/verdicts.json`) consegue observar.
visit_tokens = set()
for q in quests.values():
    for obj in [q.get('objective')] + list(q.get('stages') or []):
        if isinstance(obj, dict) and obj.get('type') == 'visit':
            visit_tokens.update(str(obj.get('target') or '').split())

riddles_doc = load('data/riddles.json') or {}
riddle_ids, riddle_sites = set(), set()
for r in riddles_doc.get('riddles', []):
    if not isinstance(r, dict):
        err("riddles: entrada que não é objeto")
        continue
    rid = r.get('id')
    if not rid:
        err("riddles: enigma sem id")
        continue
    if rid in riddle_ids:
        err(f"riddles: id repetido '{rid}'")
    riddle_ids.add(rid)
    site = r.get('site')
    if not site:
        err(f"riddles: {rid}: sem site")
    elif site not in entity_names:
        err(f"riddles: {rid}: site '{site}' sem entidade no mundo")
    elif site in riddle_sites:
        err(f"riddles: {rid}: site '{site}' já usado por outro enigma")
    else:
        riddle_sites.add(site)
    expect = r.get('expect')
    if not isinstance(expect, list) or not expect:
        err(f"riddles: {rid}: expect vazio")
    for e in expect or []:
        if not isinstance(e, dict):
            err(f"riddles: {rid}: expect malformado")
            continue
        kind, eid = e.get('kind'), e.get('id')
        if kind not in ('criatura', 'item', 'local'):
            err(f"riddles: {rid}: kind inválido '{kind}'")
        elif kind == 'item' and eid not in items:
            err(f"riddles: {rid}: espera item desconhecido '{eid}'")
        elif kind == 'criatura' and eid not in creature_aliases and eid not in kinds:
            err(f"riddles: {rid}: espera criatura sem alias '{eid}'")
        elif kind == 'local' and eid != '*' and eid not in landmark_names:
            err(f"riddles: {rid}: espera local desconhecido '{eid}'")
    for mark in r.get('require_marks') or []:
        if mark not in landmark_names:
            err(f"riddles: {rid}: require_marks com marco desconhecido '{mark}'")
    reward = r.get('reward')
    if not isinstance(reward, dict):
        err(f"riddles: {rid}: reward malformado")
        reward = {}
    for key in ('gold', 'xp'):
        value = reward.get(key, 0)
        if not isinstance(value, (int, float)) or value < 0:
            err(f"riddles: {rid}: reward.{key} inválido ({value})")
    for entry in reward.get('items') or []:
        name = str(entry).split(':')[0].strip()
        if name not in items:
            err(f"riddles: {rid}: recompensa com item desconhecido '{name}'")

verdicts_doc = load('data/verdicts.json') or {}
verdict_ids, verdict_sites = set(), set()

def check_verdict_requirement(vid, req, where, need_visit):
    if not isinstance(req, dict):
        err(f"verdicts: {vid}: {where} malformado")
        return None
    kind, eid, verb = req.get('kind'), req.get('id'), req.get('verb')
    if kind not in ('criatura', 'item', 'local'):
        err(f"verdicts: {vid}: {where} com kind inválido '{kind}'")
    elif kind == 'criatura' and eid not in creature_aliases and eid not in kinds:
        err(f"verdicts: {vid}: {where} espera criatura sem alias '{eid}'")
    elif kind == 'item' and eid not in items and eid not in PSEUDO_ITEM_IDS:
        err(f"verdicts: {vid}: {where} espera item desconhecido '{eid}'")
    elif kind == 'local':
        if eid not in landmark_names:
            err(f"verdicts: {vid}: {where} espera local desconhecido '{eid}'")
        elif need_visit and eid not in visit_tokens:
            # O juiz só vê `visitar` de tokens de quests de visita; sem isso o
            # requisito é impossível de cumprir.
            err(f"verdicts: {vid}: {where} '{eid}' nunca é registado por uma quest visit")
    return verb

for v in verdicts_doc.get('rules', []):
    if not isinstance(v, dict):
        err("verdicts: entrada que não é objeto")
        continue
    vid = v.get('id')
    if not vid:
        err("verdicts: regra sem id")
        continue
    if vid in verdict_ids:
        err(f"verdicts: id repetido '{vid}'")
    verdict_ids.add(vid)
    site = v.get('site')
    if not site:
        err(f"verdicts: {vid}: sem site")
    elif site not in entity_names:
        err(f"verdicts: {vid}: site '{site}' sem entidade no mundo")
    elif site in verdict_sites:
        err(f"verdicts: {vid}: site '{site}' já usado por outra regra")
    else:
        verdict_sites.add(site)
    require = v.get('require')
    if not isinstance(require, list) or not require:
        err(f"verdicts: {vid}: require vazio")
        require = []
    verbs = []
    for r in require:
        verbs.append(check_verdict_requirement(vid, r, 'require', True))
    for f in v.get('forbid') or []:
        check_verdict_requirement(vid, f, 'forbid', False)
    order = v.get('order')
    if not isinstance(order, list):
        err(f"verdicts: {vid}: order malformado")
        order = []
    if sorted(str(x) for x in order) != sorted(str(x) for x in verbs):
        err(f"verdicts: {vid}: order {order} não corresponde aos verbos {verbs}")
    texts = v.get('verdicts')
    if not isinstance(texts, dict):
        err(f"verdicts: {vid}: verdicts malformado")
        texts = {}
    for key in ('correct', 'partial', 'wrong'):
        if not str(texts.get(key) or '').strip():
            err(f"verdicts: {vid}: verdicts.{key} vazio")
    window = v.get('window', 0)
    if not isinstance(window, (int, float)) or window < 0:
        err(f"verdicts: {vid}: window inválido ({window})")
    reward = v.get('reward')
    if not isinstance(reward, dict):
        err(f"verdicts: {vid}: reward malformado")
        reward = {}
    for key in ('gold', 'xp'):
        value = reward.get(key, 0)
        if not isinstance(value, (int, float)) or value < 0:
            err(f"verdicts: {vid}: reward.{key} inválido ({value})")
    for entry in reward.get('items') or []:
        name = str(entry).split(':')[0].strip()
        if name not in items:
            err(f"verdicts: {vid}: recompensa com item desconhecido '{name}'")

for path in sorted(glob.glob('data/quest_puzzles/*.json')):
    base = os.path.basename(path)[:-5]
    puzzle = load(path)
    if not isinstance(puzzle, dict):
        continue
    quest = quests.get(base)
    if quest is None:
        err(f"{path}: puzzle sem quest '{base}' em quests/")
    clues = puzzle.get('clues')
    if not isinstance(clues, list) or not clues:
        err(f"{path}: clues vazio")
        clues = []
    keys = [c.get('key') if isinstance(c, dict) else None for c in clues]
    if any(not k for k in keys):
        err(f"{path}: pista sem key")
    if len(set(keys)) != len(keys):
        err(f"{path}: keys de pista repetidas")
    for c in clues:
        if not isinstance(c, dict):
            err(f"{path}: pista malformada")
            continue
        for key in ('description', 'reply'):
            if not str(c.get(key) or '').strip():
                err(f"{path}: pista '{c.get('key')}' sem {key}")
    required = puzzle.get('required')
    if not isinstance(required, int) or required < 1 or required > len(clues):
        err(f"{path}: required {required} fora de 1..{len(clues)}")
    options = puzzle.get('final_options')
    if not isinstance(options, list) or not options:
        err(f"{path}: final_options vazio")
        options = []
    option_keys = [o.get('key') if isinstance(o, dict) else None for o in options]
    if any(not k for k in option_keys):
        err(f"{path}: opção final sem key")
    if len(set(option_keys)) != len(option_keys):
        err(f"{path}: keys finais repetidas")
    for o in options:
        if isinstance(o, dict) and not str(o.get('description') or '').strip():
            err(f"{path}: opção '{o.get('key')}' sem description")
    answer = puzzle.get('final_answer')
    if answer not in option_keys:
        err(f"{path}: final_answer '{answer}' não é uma final_option")
    if quest is not None:
        objective = quest.get('objective') or {}
        if objective.get('type') != 'decision':
            err(f"{path}: a quest '{base}' não tem objetivo decision")
        if objective.get('target') != answer:
            err(f"{path}: objetivo target '{objective.get('target')}' != final_answer '{answer}'")
    for key in ('ask', 'final_prompt'):
        if not str(puzzle.get(key) or '').strip():
            err(f"{path}: {key} vazio")
    endings = puzzle.get('endings')
    if endings is None:
        endings = {}
    if not isinstance(endings, dict):
        err(f"{path}: endings malformado")
        endings = {}
    for key, ending in endings.items():
        if key not in option_keys:
            err(f"{path}: ending '{key}' não é uma final_option")
        for entry in (ending or {}).get('items') or []:
            name = str(entry).split(':')[0].strip()
            if name not in items:
                err(f"{path}: ending '{key}' com item desconhecido '{name}'")

pt, en = load('locale/pt.json') or {}, load('locale/en.json') or {}
for key in sorted(set(pt) - set(en)):
    err(f"locale: '{key}' existe em pt.json e falta em en.json")
for key in sorted(set(en) - set(pt)):
    err(f"locale: '{key}' existe em en.json e falta em pt.json")

for path in sorted(glob.glob('dialogues/*.json')):
    stem = os.path.basename(path)[:-5]
    graph = load(path)
    if not isinstance(graph, dict):
        continue
    nodes = graph.get('nodes')
    if not isinstance(nodes, dict) or not nodes:
        err(f"{path}: sem nodes")
        continue
    start = graph.get('start')
    if not start:
        err(f"{path}: sem start")
    elif start not in nodes:
        err(f"{path}: start '{start}' não existe")
    for nid, node in nodes.items():
        if not isinstance(node, dict):
            err(f"{path}: nó '{nid}' malformado")
            continue
        text = str(node.get('text') or '')
        if not text:
            err(f"{path}: nó '{nid}' sem texto")
        elif text.startswith('@'):
            key = text[1:]
            for lang, table in (('pt', pt), ('en', en)):
                if key not in table:
                    err(f"{path}: nó '{nid}': locale '{key}' falta em {lang}.json")
        for choice in node.get('choices') or []:
            if not isinstance(choice, dict):
                err(f"{path}: nó '{nid}': escolha malformada")
                continue
            if not str(choice.get('text') or '').strip():
                warn(f"{path}: nó '{nid}': escolha sem texto")
            goto = choice.get('goto')
            if goto and goto not in nodes:
                err(f"{path}: nó '{nid}': goto '{goto}' não existe")
            if choice.get('quest_accept') or choice.get('quest_turn_in'):
                if stem not in quests:
                    err(f"{path}: nó '{nid}': flag de quest sem quest '{stem}'")
    reached, stack = set(), [start]
    while stack:
        nid = stack.pop()
        if nid in reached or nid not in nodes or not isinstance(nodes[nid], dict):
            continue
        reached.add(nid)
        for choice in nodes[nid].get('choices') or []:
            if isinstance(choice, dict) and choice.get('goto'):
                stack.append(choice['goto'])
    for nid in sorted(set(nodes) - reached):
        warn(f"{path}: nó '{nid}' inalcançável a partir de '{start}'")

for w in warns:
    print('aviso:', w)
for e in errors:
    print('ERRO:', e)
print(f"conteúdo: {len(items)} itens, {len(recipes)} receitas, {len(skills)} talentos, "
      f"{len(quests)} quests, {len(kinds)} criaturas — {len(errors)} erro(s), {len(warns)} aviso(s)")
sys.exit(1 if errors else 0)
