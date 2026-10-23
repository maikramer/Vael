#!/usr/bin/env python3
"""Benchmark de decisão do Vael + corpus de calibração para o d1-omni.

As tarefas imitam decisões que o jogo precisa de tomar:
  intent   — a frase do jogador -> uma de 9 intenções (choice)
  riddle   — a resposta satisfaz o enigma? (noul)
  action   — a frase de ação cumpre o requisito? (noul)
  combat   — política tática sobre números (choice)
  tone     — a fala ameaça o NPC? (noul)
  shop     — o mercador vende, negocia ou recusa? (choice)

Uso: python3 make_bench.py
Saída: bench.jsonl, calib.txt
"""
import json, os, random, re, glob

ROOT = os.environ.get("VAEL_ROOT", os.path.abspath(os.path.join(os.path.dirname(__file__), "../..")))
OUT = os.path.dirname(os.path.abspath(__file__))
random.seed(7)

items = []


def add(cat, state, questions, expected, note=""):
    items.append({
        "id": f"{cat}_{len(items):04d}",
        "cat": cat,
        "state": state,
        "questions": questions,
        "expected": expected,
        "note": note,
    })


# ------------------------------------------------------------------ intent
INTENTS = {
    "comprar": "comprar um item ao mercador",
    "vender": "vender um item ao mercador",
    "falar": "conversar ou saber novidades",
    "atacar": "atacar alguém",
    "ir": "ir para outro lugar",
    "usar": "usar ou equipar um item",
    "entregar": "entregar um item ou encomenda",
    "perguntar": "fazer uma pergunta sobre o mundo",
    "nenhuma": "nenhuma ação clara",
}
INTENT_Q = {
    "q": {
        "type": "choice",
        "instructions": "Qual é a intenção do jogador nesta fala?",
        "criteria": INTENTS,
    }
}

PT = {
    "comprar": [
        "quanto custa essa poção?", "quero comprar uma tocha", "vende-me a espada",
        "levo duas poções", "tem alguma coisa barata?", "aceitas ouro por essa pele?",
        "dá-me três maçãs, pago agora", "reserva esse machado para mim",
        "troco dez moedas pela corda", "quero levar tudo o que está na banca",
        "não quero comprar nada, só estou a ver",  # hard: negated
        "quanto é que fica o conjunto todo?",
    ],
    "vender": [
        "quero vender esta pele", "compras peles de lobo?", "vendo isto por quanto?",
        "interessa-te uma espada velha?", "trouxe musgo para vender",
        "dou-te a relíquia por cinquenta moedas", "ficarias com estas bagas?",
        "quero despachar este lixo", "alguém aqui compra dentes de goblin?",
        "pago-te em peles, não em ouro",  # hard: 'pago' but selling
        "troco este amuleto por uma poção",
    ],
    "falar": [
        "olá", "quem és tu?", "tens novidades?", "fala-me da vila",
        "bom dia, como tem passado?", "ouvi dizer que o forno abriu",
        "o que achas dos lobos?", "conta-me uma história",
        "andas cansada, Greta?", "até logo", "obrigado pela conversa",
        "as pessoas falam de ti na praça",
    ],
    "atacar": [
        "morre!", "vou cortar-te em dois", "prepara-te para lutar",
        "ataca o goblin", "dispara contra o lobo", "acerta-lhe com a espada",
        "vou matar-te", "defende-te se puderes", "corta a corda e deixa-o cair",
        "avança e esmaga o escorpião", "não te aproximes ou ataco",
    ],
    "ir": [
        "onde fica a floresta?", "como chego ao poço?", "vamos para a cidadela",
        "quero sair daqui", "que caminho leva ao pantano?", "leva-me ao portão",
        "volto para a estalagem", "segue para norte", "atravessa a ponte",
        "quero ir à montanha", "afasta-te da praça", "encontramo-nos na torre",
    ],
    "usar": [
        "bebo a poção", "acendo a tocha", "uso a chave na porta",
        "equipo a espada", "como a fruta", "acendo o fogo com a madeira",
        "lanço a poção ao chão", "visto a armadura", "toco a vara no altar",
        "parto a pedra com o machado", "acendo a lanterna",
    ],
    "entregar": [
        "aqui está a encomenda", "entrego a carta", "toma isto",
        "traz-te a pele do lobo", "deixo as oferendas no altar",
        "devolvo-te a chave", "entrego o musgo ao guardião",
        "toma as três pedras", "aqui tens o dinheiro que devia",
        "deposito a relíquia no baú", "dou-te a poção que pediste",
    ],
    "perguntar": [
        "o que é isto?", "porque é que a ponte está fechada?", "quem matou o ferreiro?",
        "onde está o tesouro?", "quantos lobos há na floresta?", "o que aconteceu aqui?",
        "porque choras?", "quem é o ancião?", "que horas são?",
        "explica-me o juramento", "de onde vem este som?",
    ],
    "nenhuma": [
        "hmm", "não sei", "deixa-me em paz", "...", "talvez",
        "estou só a passar", "não respondo", "espera", "nada",
        "não é nada contigo", "fico aqui calado",
    ],
}

EN = {
    "comprar": ["how much is this potion?", "I want to buy a torch", "sell me the sword"],
    "vender": ["I want to sell this pelt", "do you buy wolf pelts?", "I'll sell it for ten coins"],
    "falar": ["hello", "who are you?", "any news?", "tell me about the village"],
    "atacar": ["die!", "I'll cut you in two", "attack the goblin"],
    "ir": ["where is the forest?", "how do I get to the well?", "let's go to the citadel"],
    "usar": ["I drink the potion", "I light the torch", "I equip the sword"],
    "entregar": ["here is the package", "I deliver the letter", "take this"],
    "perguntar": ["what is this?", "why is the bridge closed?", "who killed the smith?"],
    "nenhuma": ["hmm", "I don't know", "leave me alone"],
}

for cat, phrases in PT.items():
    for p in phrases:
        add("intent", {"utterance": p, "npc": "Greta, mercadora", "place": "praça da vila"}, INTENT_Q, cat)
for cat, phrases in EN.items():
    for p in phrases:
        add("intent", {"utterance": p, "npc": "Greta, merchant", "place": "village square"}, INTENT_Q, cat)

# ------------------------------------------------------------------ riddle
riddles = json.load(open(f"{ROOT}/data/riddles.json"))["riddles"]
answers = {
    "echo_wolf": [
        ("lobos", True), ("um lobo", True), ("alcateia", True), ("lobo negro", True),
        ("sombra", False), ("urso", False), ("goblins", False), ("o vento", False),
        ("lobos e corvos", True), ("não sei", False),
    ],
    "three_gifts": [
        ("pedra, madeira e fruta", True), ("pedra e madeira e fruta", True),
        ("fruta, pedra, madeira", True), ("pedra, madeira", False),
        ("pedra, madeira e pão", False), ("madeira e fruta", False),
        ("pedra, madeira, fruta e pão", True), ("ouro, prata e bronze", False),
        ("as três oferendas", False), ("pedra madeira fruta", True),
    ],
    "odd_statue": [
        ("sombra", True), ("a sombra", True), ("espectro", True), ("fantasma", True),
        ("lobos", False), ("o lobo", False), ("a terceira estátua", False),
        ("pedra", False), ("sombra e lobo", True), ("não pertence", False),
    ],
}
for rid, cases in answers.items():
    r = next(x for x in riddles if x["id"] == rid)
    for ans, ok in cases:
        add("riddle", {"riddle_prompt": r["prompt"], "player_answer": ans},
            {"q": {"type": "noul", "instructions": f'A resposta satisfaz o enigma "{r["prompt"]}"?',
                   "criteria": {"false": "não satisfaz", "true": "satisfaz"}}}, ok)

# ------------------------------------------------------------------ action
actions = [
    ("o herói matou lobo em Floresta Sombria", "O herói matou um lobo?", True),
    ("o herói matou lobo em Floresta Sombria", "O herói visitou o poço?", False),
    ("o herói visitou o poço da encruzilhada", "O herói visitou o poço?", True),
    ("o herói visitou o poço da encruzilhada", "O herói matou um lobo?", False),
    ("o herói equipou tocha em Floresta Sombria", "O herói equipou uma tocha?", True),
    ("o herói equipou tocha em Floresta Sombria", "O herói comprou uma tocha?", False),
    ("o herói comprou poção em Vila de Discordia", "O herói comprou uma poção?", True),
    ("o herói vendeu pele de lobo em Vila de Discordia", "O herói vendeu uma pele de lobo?", True),
    ("o herói usou machado de ferro na floresta", "O herói usou um machado?", True),
    ("o herói usou machado de ferro na floresta", "O herói equipou um machado?", False),
    ("o herói matou bandido na estrada", "O herói matou um bandido?", True),
    ("o herói matou bandido na estrada", "O herói matou um goblin?", False),
    ("o herói visitou a torre do posto avançado", "O herói visitou a torre?", True),
    ("o herói visitou a torre do posto avançado", "O herói visitou o círculo de pedras?", False),
    ("o herói equipou espada na cidadela", "O herói equipou uma espada?", True),
    ("o herói equipou espada na cidadela", "O herói usou uma espada?", False),
    ("o herói comprou fruta no mercado", "O herói comprou fruta?", True),
    ("o herói comprou fruta no mercado", "O herói vendeu fruta?", False),
    ("o herói usou a chave na porta", "O herói usou a chave?", True),
    ("o herói usou a chave na porta", "O herói usou a poção?", False),
]
for sent, q, ok in actions:
    add("action", {"action_log": sent, "question": q},
        {"q": {"type": "noul", "instructions": q, "criteria": {"false": "não", "true": "sim"}}}, ok)

# ------------------------------------------------------------------ combat
COMBAT = {
    "atacar": "avançar e atacar",
    "esquivar": "esquivar e esperar",
    "curar": "beber uma poção",
    "fugir": "fugir do combate",
}
COMBAT_Q = {"q": {"type": "choice", "instructions": "Qual é a melhor ação agora?",
                  "criteria": COMBAT}}


def combat_policy(hp, potions, dist, stamina, enemy_hp):
    if hp <= 25 and potions > 0:
        return "curar"
    if hp <= 20:
        return "fugir"
    if dist <= 3.0 and stamina >= 25:
        return "atacar"
    if dist <= 3.0:
        return "esquivar"
    if hp <= 35 and enemy_hp > 60:
        return "fugir"
    if dist <= 8.0 and stamina >= 15:
        return "atacar"
    return "esquivar"


combat_cases = [
    (18, 1, 2.0, 40, 60), (12, 0, 2.5, 30, 70), (55, 2, 2.0, 35, 40),
    (48, 0, 2.2, 10, 55), (72, 1, 5.0, 40, 30), (30, 0, 12.0, 50, 80),
    (22, 2, 20.0, 60, 90), (90, 0, 2.0, 20, 75), (40, 0, 6.0, 10, 50),
    (65, 1, 4.0, 18, 45), (15, 0, 1.5, 50, 95), (80, 3, 2.8, 60, 20),
    (50, 0, 2.9, 24, 55), (35, 1, 9.0, 30, 65), (28, 3, 3.5, 20, 70),
    (18, 0, 3.0, 40, 30), (60, 0, 3.0, 25, 40), (45, 0, 2.0, 12, 60),
    (25, 1, 15.0, 50, 85), (70, 0, 7.5, 14, 50), (10, 0, 2.0, 30, 20),
    (55, 0, 11.0, 40, 45), (33, 2, 2.0, 35, 80), (88, 0, 3.2, 22, 30),
    (42, 0, 4.5, 26, 60), (20, 1, 6.0, 35, 90), (62, 0, 1.8, 40, 50),
    (75, 2, 2.0, 15, 25), (38, 0, 8.5, 12, 70), (29, 0, 2.5, 28, 65),
]
for hp, pot, dist, st, ehp in combat_cases:
    add("combat", {"hp": hp, "max_hp": 100, "potions": pot, "enemy_distance_m": dist,
                   "stamina": st, "enemy_hp_pct": ehp}, COMBAT_Q,
        combat_policy(hp, pot, dist, st, ehp))

# ------------------------------------------------------------------ tone
tone = [
    ("Vou partir-te os dentes se não te calas.", True),
    ("Bom dia, tens quartos livres?", False),
    ("Sabes o que acontece a quem mente aos mercadores?", True),
    ("Que cheiro horrível, andas a dormir com os porcos?", True),
    ("Quanto custa o pão?", False),
    ("Se me enganares, volto com a espada.", True),
    ("Obrigado pela ajuda, até logo.", False),
    ("Dá-me tudo o que tens ou arrependes-te.", True),
    ("A tua filha está bem? Ouvi dizer que adoeceu.", False),
    ("Não me toques, velho nojento.", True),
    ("Conte-me das estradas do norte.", False),
    ("A tua banca é uma vergonha para a vila.", True),
    ("Aceitas esta encomenda?", False),
    ("Vou contar a todos que roubas os clientes.", True),
    ("Bela manhã, não é?", False),
    ("Sai da frente antes que te empurre.", True),
    ("Tens alguma poção que cure feridas?", False),
    ("Aquele guarda é um covarde, e tu também.", True),
    ("Fico grato pela conversa.", False),
    ("Vou queimar a tua loja.", True),
]
for text, threat in tone:
    add("tone", {"utterance": text, "npc": "Greta, mercadora"},
        {"q": {"type": "noul", "instructions": "O jogador está a ameaçar o mercador?",
               "criteria": {"false": "não ameaça", "true": "ameaça"}}}, threat)

# ------------------------------------------------------------------ shop
SHOP = {
    "vender_barato": "vender pelo preço pedido",
    "negociar": "negociar um preço intermédio",
    "recusar": "recusar a venda",
}
SHOP_Q = {"q": {"type": "choice", "instructions": "O que deve o mercador fazer?",
                "criteria": SHOP}}


def shop_policy(price, gold, reputation, demand):
    if reputation >= 60 and gold >= price * 0.8 and demand >= 40:
        return "vender_barato"
    if reputation >= 30 and gold >= price * 0.4:
        return "negociar"
    if gold >= price:
        return "vender_barato"
    if gold >= price * 0.5 or reputation >= 50:
        return "negociar"
    return "recusar"


shop_cases = [
    (50, 60, 70, 60), (50, 20, 20, 80), (100, 30, 40, 50), (30, 25, 10, 30),
    (80, 10, 55, 90), (20, 100, 80, 20), (200, 50, 20, 70), (40, 15, 65, 40),
    (60, 60, 10, 10), (90, 20, 35, 85), (25, 5, 0, 50), (75, 75, 45, 30),
    (150, 60, 70, 60), (35, 35, 25, 40), (10, 2, 30, 90), (120, 100, 55, 50),
    (55, 10, 60, 80), (45, 22, 15, 20), (300, 200, 90, 95), (65, 30, 5, 60),
]
for price, gold, rep, dem in shop_cases:
    add("shop", {"price": price, "player_gold": gold, "reputation": rep, "demand": dem},
        SHOP_Q, shop_policy(price, gold, rep, dem))

# ------------------------------------------------------------------ write
with open(f"{OUT}/bench.jsonl", "w") as f:
    for it in items:
        f.write(json.dumps(it, ensure_ascii=False) + "\n")

# Corpus de calibração: texto do jogo + estados do benchmark (3x) + inglês.
chunks = []


def collect(obj):
    if isinstance(obj, str):
        s = re.sub(r"\s+", " ", obj).strip()
        if len(s) >= 8:
            chunks.append(s)
    elif isinstance(obj, dict):
        for v in obj.values():
            collect(v)
    elif isinstance(obj, list):
        for v in obj:
            collect(v)


for pattern in ["dialogues/*.json", "quests/*.json", "data/*.json", "locale/*.json"]:
    for path in glob.glob(f"{ROOT}/{pattern}"):
        try:
            collect(json.load(open(path)))
        except Exception:
            pass

bench_text = []
for it in items:
    bench_text.append(json.dumps(it["state"], ensure_ascii=False))
    for q in it["questions"].values():
        bench_text.append(q.get("instructions", ""))
        crit = q.get("criteria")
        if isinstance(crit, dict):
            bench_text.extend(f"{k}: {v}" for k, v in crit.items())
        elif isinstance(crit, list):
            bench_text.extend(str(x) for x in crit)

with open(f"{OUT}/calib.txt", "w") as f:
    for s in chunks:
        f.write(s + "\n")
    for _ in range(4):
        for s in bench_text:
            f.write(s + "\n")

print(f"{len(items)} itens -> bench.jsonl")
from collections import Counter
print(Counter(x["cat"] for x in items))
print(f"calib.txt: {os.path.getsize(OUT + '/calib.txt')/1e6:.2f} MB")
