#!/usr/bin/env python3
"""Sondas de prompt: compara Q4 e Q8 em tarefas bem formuladas."""
import json, urllib.request, sys

PORTS = {"q4": 18080, "q8": 18081}

def ask(port, state, q):
    body = json.dumps({"state": state, "questions": {"q": q}}, ensure_ascii=False).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}/v1/systemone", data=body,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())["answers"]["q"]

def show(name, state, q, expected):
    print(f"\n### {name}  (esperado: {expected})")
    for tag, port in PORTS.items():
        a = ask(port, state, q)
        if a["type"] == "choice":
            print(f"  {tag}: {a['choice']} conf={a['confidence']:.2f} {a['probabilities']}")
        elif a["type"] == "noul":
            print(f"  {tag}: p(yes)={a['noul']:.3f}")
        else:
            print(f"  {tag}: score={a['score']:.2f} {a['probabilities']}")

# 1. action entailment
show("action yes",
     {"action_log": "o herói matou lobo em Floresta Sombria", "requirement": "kill a wolf"},
     {"type": "noul", "instructions": "Does the action log satisfy the requirement?",
      "criteria": {"false": "the log does not show the required action", "true": "the log shows the required action"}},
     True)
show("action no",
     {"action_log": "o herói visitou o poço da encruzilhada", "requirement": "kill a wolf"},
     {"type": "noul", "instructions": "Does the action log satisfy the requirement?",
      "criteria": {"false": "the log does not show the required action", "true": "the log shows the required action"}},
     False)

# 2. tone / moderation
TONE_Q = {"type": "noul", "instructions": "Is the player threatening the merchant? A threat is a promise of violence, an insult, or an attempt to intimidate. Requests, complaints and polite talk are not threats.",
          "criteria": {"false": "not a threat", "true": "a threat"}}
show("tone threat", {"utterance": "Vou partir-te os dentes se não te calas.", "npc": "Greta, merchant"}, TONE_Q, True)
show("tone polite", {"utterance": "Bom dia, tens quartos livres?", "npc": "Greta, merchant"}, TONE_Q, False)
show("tone insult", {"utterance": "Aquele guarda é um covarde, e tu também.", "npc": "Greta, merchant"}, TONE_Q, True)

# 3. riddle semantic match
RID_Q = {"type": "noul", "instructions": "Does the answer name the creature described by the clue?",
         "criteria": {"false": "the answer names a different creature or is unrelated", "true": "the answer names the creature in the clue"}}
show("riddle wolf yes", {"clue": "A deep howl echoes three times. Large paw prints mark the mud.", "answer": "alcateia"}, RID_Q, True)
show("riddle wolf no", {"clue": "A deep howl echoes three times. Large paw prints mark the mud.", "answer": "sombra"}, RID_Q, False)
show("riddle gifts yes", {"clue": "One offering is heavy, one burns, and one feeds.", "answer": "pedra, madeira e fruta"}, RID_Q, True)
show("riddle gifts no", {"clue": "One offering is heavy, one burns, and one feeds.", "answer": "pedra e madeira"}, RID_Q, False)

# 4. dialogue routing
DIAL_Q = {"type": "choice", "instructions": "Which reply best fits the player's utterance?",
          "criteria": {
              "quartos": "Dois quartos livres. O do fundo tem a janela que range.",
              "viajantes": "Passam cartógrafos, mercadores e um homem dos mojões.",
              "novidades": "O forno da Alda abriu e o Hald anda atrás dos lobos.",
              "adeus": "A porta não tem fechadura. Até à próxima."}}
show("dialogue rooms", {"npc": "Nela, innkeeper", "player_utterance": "há camas para dormir esta noite?"}, DIAL_Q, "quartos")
show("dialogue travelers", {"npc": "Nela, innkeeper", "player_utterance": "quem é que passa por aqui?"}, DIAL_Q, "viajantes")
show("dialogue news", {"npc": "Nela, innkeeper", "player_utterance": "alguma coisa nova na vila?"}, DIAL_Q, "novidades")
show("dialogue bye", {"npc": "Nela, innkeeper", "player_utterance": "obrigado, até logo"}, DIAL_Q, "adeus")

# 5. intent with context and English criteria
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
show("intent buy price", {"context": "The player is at Greta's stall in the village square.", "utterance": "quanto custa essa poção?"}, INT_Q, "buy")
show("intent buy", {"context": "The player is at Greta's stall in the village square.", "utterance": "quero comprar uma tocha"}, INT_Q, "buy")
show("intent sell", {"context": "The player is at Greta's stall in the village square.", "utterance": "compras peles de lobo?"}, INT_Q, "sell")
show("intent chat", {"context": "The player is at Greta's stall in the village square.", "utterance": "bom dia, como tem passado?"}, INT_Q, "chat")
show("intent attack", {"context": "The player is at Greta's stall in the village square.", "utterance": "vou cortar-te em dois"}, INT_Q, "attack")

# 6. explicit policy
SHOP_Q = {"type": "choice", "instructions": "Apply the merchant policy to this offer.",
          "criteria": {"sell": "sell at the asking price", "negotiate": "negotiate a middle price", "refuse": "refuse the sale"}}
policy = ("Policy: sell at the asking price if reputation >= 60 and player gold >= 80% of price and demand >= 40; "
          "negotiate if reputation >= 30 and player gold >= 40% of price; otherwise refuse.")
show("shop sell", {"policy": policy, "price": 50, "player_gold": 60, "reputation": 70, "demand": 60}, SHOP_Q, "sell")
show("shop refuse", {"policy": policy, "price": 50, "player_gold": 20, "reputation": 20, "demand": 80}, SHOP_Q, "refuse")

COMBAT_Q = {"type": "choice", "instructions": "Apply the combat policy.",
            "criteria": {"attack": "attack now", "dodge": "dodge and wait", "heal": "drink a potion", "flee": "flee"}}
cpolicy = ("Heal if hp <= 25 and potions > 0. Flee if hp <= 20. Attack if distance <= 3 and stamina >= 25. "
           "Dodge if distance <= 3. Flee if hp <= 35 and enemy_hp > 60. Attack if distance <= 8 and stamina >= 15. Otherwise dodge.")
show("combat heal", {"policy": cpolicy, "hp": 18, "max_hp": 100, "potions": 1, "enemy_distance_m": 2.0, "stamina": 40, "enemy_hp_pct": 60}, COMBAT_Q, "heal")
show("combat dodge", {"policy": cpolicy, "hp": 48, "max_hp": 100, "potions": 0, "enemy_distance_m": 2.2, "stamina": 10, "enemy_hp_pct": 55}, COMBAT_Q, "dodge")
