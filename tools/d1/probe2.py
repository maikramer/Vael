#!/usr/bin/env python3
"""Sondas 2: língua, formato de estado e formulação do noul."""
import json, urllib.request

PORTS = {"q4": 18080, "q8": 18081}

def ask(port, state, q):
    body = json.dumps({"state": state, "questions": {"q": q}}, ensure_ascii=False).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}/v1/systemone", data=body,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())["answers"]["q"]

def show(name, state, q, expected):
    print(f"\n### {name}  (esperado {expected})")
    for tag, port in PORTS.items():
        a = ask(port, state, q)
        print(f"  {tag}: " + (f"p(yes)={a['noul']:.3f}" if a['type']=='noul' else str(a)))

NOUL = lambda instr, crit=None: {"type": "noul", "instructions": instr, **({"criteria": crit} if crit else {})}

# ---- tone: língua x formato
pt = "Vou partir-te os dentes se não te calas."
en = "I will knock your teeth out if you do not shut up."
for lang, utt in [("PT", pt), ("EN", en)]:
    show(f"tone {lang} statement/string", f"The player says to the merchant: \"{utt}\"",
         NOUL("The player is threatening the merchant."), True)
    show(f"tone {lang} question/string", f"The player says to the merchant: \"{utt}\"",
         NOUL("Is the player threatening the merchant?"), True)
    show(f"tone {lang} question/json+crit", {"utterance": utt, "npc": "Greta, merchant"},
         NOUL("Is the player threatening the merchant?",
              {"false": "no threat", "true": "a threat"}), True)

# ---- action: língua x formato
for lang, log, req in [
    ("PT", "o herói matou lobo em Floresta Sombria", "matar um lobo"),
    ("EN", "the hero killed a wolf in the Dark Forest", "kill a wolf"),
]:
    show(f"action {lang} json", {"action_log": log, "requirement": req},
         NOUL("Does the action log satisfy the requirement?"), True)
    show(f"action {lang} string", f"Action log: {log}. Requirement: {req}.",
         NOUL("Does the action log satisfy the requirement?"), True)
    show(f"action {lang} statement", f"Action log: {log}. Requirement: {req}.",
         NOUL(f"The action log shows that the hero {req}."), True)

# ---- riddle
for lang, clue, ans in [
    ("PT", "Um uivo grave ecoa três vezes. Pegadas grandes marcam a lama.", "alcateia"),
    ("EN", "A deep howl echoes three times. Large paw prints mark the mud.", "a pack of wolves"),
]:
    show(f"riddle {lang} string", f"Clue: {clue} Answer: {ans}",
         NOUL("Does the answer name the creature described by the clue?"), True)
    show(f"riddle {lang} json", {"clue": clue, "answer": ans},
         NOUL("Does the answer name the creature described by the clue?"), True)
    show(f"riddle {lang} statement", f"Clue: {clue} Answer: {ans}",
         NOUL("The answer names the creature described by the clue."), True)

# ---- intent clear vs ambiguous, PT/EN
INT_Q = {"type": "choice", "instructions": "What does the player want to do?",
         "criteria": {
             "buy": "purchase an item from the merchant",
             "sell": "sell an item to the merchant",
             "chat": "just talk or hear news",
             "attack": "attack the merchant",
             "travel": "ask for directions or go somewhere",
             "use": "use or equip an item",
             "deliver": "hand over an item",
             "ask": "ask a factual question about the world",
             "none": "no clear request"}}
for utt, exp in [("quanto custa essa poção?", "buy"), ("how much is this potion?", "buy"),
                 ("compras peles de lobo?", "sell"), ("do you buy wolf pelts?", "sell"),
                 ("bom dia, como tem passado?", "chat"), ("good morning, how have you been?", "chat"),
                 ("vou cortar-te em dois", "attack"), ("I will cut you in two", "attack")]:
    show(f"intent {exp} / {utt[:24]}", {"context": "The player is at Greta's stall in the village square.", "utterance": utt}, INT_Q, exp)

# ---- dialogue routing EN
DIAL = {"type": "choice", "instructions": "Which reply best fits the player's utterance?",
        "criteria": {
            "quartos": "Two rooms are free. The back one has a creaking window.",
            "viajantes": "Cartographers, merchants and a silent man from the moors pass through.",
            "novidades": "Alda's oven opened and Hald is hunting the wolves.",
            "adeus": "The door has no lock. Until next time."}}
for utt, exp in [("do you have beds for the night?", "quartos"),
                 ("who passes through here?", "viajantes"),
                 ("what is new in the village?", "novidades"),
                 ("thanks, goodbye", "adeus")]:
    show(f"dialogue {exp}", {"npc": "Nela, innkeeper", "player_utterance": utt}, DIAL, exp)
