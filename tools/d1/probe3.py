#!/usr/bin/env python3
"""Sondas 3: instruções em PT vs EN, estado PT."""
import json, urllib.request

PORT = 18081

def ask(state, q):
    body = json.dumps({"state": state, "questions": {"q": q}}, ensure_ascii=False).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}/v1/systemone", data=body,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())["answers"]["q"]

def show(name, state, q, expected):
    a = ask(state, q)
    val = a.get("noul") if a["type"] == "noul" else a
    print(f"{name:55s} exp={str(expected):5s} -> {val}")

TONE_STATE = {"utterance": "Vou queimar a tua loja.", "npc": "Greta, mercadora"}
TONE_EN = {"type": "noul",
           "instructions": "Is the player threatening or insulting the merchant? A threat is a promise of violence, an insult, or an attempt to intimidate. Requests, complaints and polite talk are not threats.",
           "criteria": {"false": "not a threat or insult", "true": "a threat or insult"}}
TONE_PT = {"type": "noul",
           "instructions": "O jogador está a ameaçar ou a insultar o mercador? Uma ameaça é uma promessa de violência, um insulto ou uma tentativa de intimidar. Pedidos, queixas e conversa educada não são ameaças.",
           "criteria": {"false": "não é ameaça nem insulto", "true": "é ameaça ou insulto"}}
TONE_PT_SHORT = {"type": "noul", "instructions": "O jogador está a ameaçar o mercador?",
                 "criteria": {"false": "não", "true": "sim"}}
show("tone PT state / EN instr+crit", TONE_STATE, TONE_EN, True)
show("tone PT state / PT instr+crit", TONE_STATE, TONE_PT, True)
show("tone PT state / PT short", TONE_STATE, TONE_PT_SHORT, True)

INT_STATE = {"context": "O jogador está na banca de Greta, na praça.", "utterance": "quero comprar uma poção"}
INT_EN = {"type": "choice", "instructions": "What does the player want to do?",
          "criteria": {"buy": "purchase an item from the merchant", "sell": "sell an item to the merchant",
                       "talk": "chat, greet someone or hear news", "attack": "attack or threaten violence",
                       "travel": "go somewhere or ask for directions", "use": "use or equip an item",
                       "deliver": "hand over an item", "ask": "ask a factual question about the world",
                       "none": "no clear request"}}
INT_PT = {"type": "choice", "instructions": "O que quer o jogador fazer?",
          "criteria": {"buy": "comprar um item ao mercador", "sell": "vender um item ao mercador",
                       "talk": "conversar, cumprimentar ou ouvir novidades", "attack": "atacar ou ameaçar",
                       "travel": "ir a algum lugar ou pedir direções", "use": "usar ou equipar um item",
                       "deliver": "entregar um item", "ask": "fazer uma pergunta factual sobre o mundo",
                       "none": "nenhum pedido claro"}}
show("intent PT state / EN instr+crit", INT_STATE, INT_EN, "buy")
show("intent PT state / PT instr+crit", INT_STATE, INT_PT, "buy")
for utt, exp in [("vou matar-te", "attack"), ("equipo a espada", "use"), ("aqui está a encomenda", "deliver"),
                 ("onde fica a floresta?", "travel"), ("compras peles de lobo?", "sell")]:
    show(f"intent PT {utt[:28]} / PT", {**INT_STATE, "utterance": utt}, INT_PT, exp)
    show(f"intent PT {utt[:28]} / EN", {**INT_STATE, "utterance": utt}, INT_EN, exp)

RID_STATE = {"clue": "Um uivo grave ecoa três vezes. Pegadas grandes marcam a lama.", "answer": "alcateia"}
RID_EN = {"type": "noul", "instructions": "Does the answer name the creature described by the clue?",
          "criteria": {"false": "the answer names something else", "true": "the answer names the creature"}}
RID_PT = {"type": "noul", "instructions": "A resposta nomeia a criatura descrita pela pista?",
          "criteria": {"false": "a resposta nomeia outra coisa", "true": "a resposta nomeia a criatura"}}
show("riddle PT state / EN instr+crit", RID_STATE, RID_EN, True)
show("riddle PT state / PT instr+crit", RID_STATE, RID_PT, True)
show("riddle PT state / PT short", RID_STATE, {"type": "noul", "instructions": "A resposta nomeia a criatura descrita pela pista?"}, True)
