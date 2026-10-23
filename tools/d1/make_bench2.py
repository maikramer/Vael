#!/usr/bin/env python3
"""Benchmark v2: tarefas no formato que o d1-omni foi treinado.

Instruções em inglês, opções com descrições ricas, estado PT/EN separado para
medir o fosso de língua. Saída: bench2.jsonl
"""
import json, os

OUT = os.path.dirname(os.path.abspath(__file__))
items = []


def add(cat, lang, state, questions, expected, note=""):
    items.append({"id": f"{cat}_{len(items):04d}", "cat": cat, "lang": lang,
                  "state": state, "questions": questions, "expected": expected, "note": note})


# ------------------------------------------------------------------ dialogue routing
DIALOGUES = {
    "innkeeper": {
        "npc": "Nela, innkeeper at the House A inn",
        "criteria": {
            "quartos": "rooms for the night. 'Dois quartos livres; um tem a janela que range.'",
            "viajantes": "travelers passing through. 'Cartógrafos, mercadores, um homem dos mojões.'",
            "novidades": "news from the village. 'O forno da Alda abriu; o Hald caça lobos.'",
            "adeus": "a farewell that ends the conversation. 'A porta não tem fechadura.'",
        },
        "cases": [
            ("Tens quartos livres?", "quartos"), ("Há camas para dormir esta noite?", "quartos"),
            ("Dá para ficar num quarto?", "quartos"), ("Do you have rooms for the night?", "quartos"),
            ("Que viajantes passam por aqui?", "viajantes"), ("Quem é que passa nesta estrada?", "viajantes"),
            ("Vê-se muita gente de fora?", "viajantes"), ("Who travels through here?", "viajantes"),
            ("O que há de novo na vila?", "novidades"), ("Alguma novidade por aqui?", "novidades"),
            ("O que se passa de novo?", "novidades"), ("What is new in the village?", "novidades"),
            ("Só vim ver. Obrigado.", "adeus"), ("Adeus, boa noite.", "adeus"),
            ("Obrigado, até logo.", "adeus"), ("Thanks, goodbye.", "adeus"),
        ],
    },
    "chapel": {
        "npc": "Padre do santuário de Vael",
        "criteria": {
            "coro": "the Choir of Vael and its broken Notes. 'As quatro Notas partiram-se.'",
            "sinos": "the bells and deaths in the forest. 'Tocam só quando alguém morre.'",
            "ajuda": "the priest asks for help with wounded travelers. 'Manda os feridos para cá.'",
            "adeus": "a farewell blessing. 'Que a luz te guarde.'",
        },
        "cases": [
            ("Falas do Coro de Vael?", "coro"), ("O que é o Coro?", "coro"), ("E as Notas do Coro?", "coro"),
            ("Que se passa com os sinos?", "sinos"), ("Porque tocam os sinos?", "sinos"),
            ("Os sinos ainda tocam?", "sinos"),
            ("Precisas de alguma coisa?", "ajuda"), ("Posso ajudar em algo?", "ajuda"),
            ("Há alguém ferido?", "ajuda"),
            ("Até à vista.", "adeus"), ("Adeus, padre.", "adeus"), ("Que a luz te guarde.", "adeus"),
        ],
    },
    "cook": {
        "npc": "Cozinheiro da casa A",
        "criteria": {
            "javali": "where the boar came from. 'No bosque, antes do sol nascer.'",
            "lobos": "wolves causing trouble. 'Comem o gado do Hald.'",
            "comensais": "who eats here. 'Viajantes, pastores, o guarda da praça.'",
            "adeus": "a farewell. 'Bom apetite.'",
        },
        "cases": [
            ("Onde apanhaste o javali?", "javali"), ("De onde veio a carne?", "javali"),
            ("Caçaste o javali onde?", "javali"),
            ("Os lobos dão trabalho?", "lobos"), ("Há problemas com lobos?", "lobos"),
            ("Os lobos atacam o gado?", "lobos"),
            ("Quem é que come aqui?", "comensais"), ("Quem vem à tua mesa?", "comensais"),
            ("Que gente come aqui?", "comensais"),
            ("Bom apetite.", "adeus"), ("Até logo, cozinheiro.", "adeus"), ("Obrigado, até logo.", "adeus"),
        ],
    },
    "scholar": {
        "npc": "Mestre Orin, scholar",
        "criteria": {
            "mapas": "old maps and books. 'Mapas antigos que não concordam.'",
            "cidadela": "the citadel to the west. 'As muralhas caíram por dentro.'",
            "cristais": "the crystals to the east. 'Brilham à noite e cantam.'",
            "adeus": "a farewell. 'Fecha a porta devagar.'",
        },
        "cases": [
            ("O que andas a ler?", "mapas"), ("Que mapas são esses?", "mapas"),
            ("Estás a estudar o quê?", "mapas"),
            ("Falas da cidadela a oeste?", "cidadela"), ("O que há a oeste?", "cidadela"),
            ("A cidadela ainda está de pé?", "cidadela"),
            ("E os cristais do leste?", "cristais"), ("Os cristais brilham?", "cristais"),
            ("O que sabes dos cristais?", "cristais"),
            ("Desculpa. Até logo.", "adeus"), ("Adeus, mestre.", "adeus"), ("Vou-me embora.", "adeus"),
        ],
    },
    "baker": {
        "npc": "Alda, baker",
        "criteria": {
            "pao": "the bread she bakes. 'Centeio com uma crosta dura.'",
            "lenha": "where the firewood comes from. 'Do bosque, cortada com machado.'",
            "forno": "the old oven. 'Antigo como a vila.'",
            "adeus": "a farewell. 'Volta quando o pão estiver quente.'",
        },
        "cases": [
            ("Que pão fazes?", "pao"), ("Que pão é esse?", "pao"), ("Fazes pão de quê?", "pao"),
            ("De onde vem a lenha?", "lenha"), ("Quem corta a lenha?", "lenha"),
            ("Onde arranjas lenha?", "lenha"),
            ("O forno é antigo?", "forno"), ("Há quanto tempo existe o forno?", "forno"),
            ("Esse forno é novo?", "forno"),
            ("Fico para depois. Até logo.", "adeus"), ("Adeus, Alda.", "adeus"),
            ("Volto mais tarde.", "adeus"),
        ],
    },
    "longhouse": {
        "npc": "Estalajadeiro do longhouse",
        "criteria": {
            "historias": "stories of the Heights and the Choir. 'Quem sobe aos picos ouve as Notas.'",
            "comida": "food and drink. 'Guisado de javali, pão e queijo.'",
            "ajuda": "someone who needs help. 'O Hald perdeu gado; o Bram paga por minério.'",
            "adeus": "a farewell. 'O lume fica aceso até tarde.'",
        },
        "cases": [
            ("Que histórias se contam?", "historias"), ("Conta-me uma história.", "historias"),
            ("O que dizem do Alto?", "historias"),
            ("O que há para comer?", "comida"), ("Que comida tens?", "comida"),
            ("Tens algo quente para beber?", "comida"),
            ("Alguém precisa de ajuda?", "ajuda"), ("Quem precisa de ajuda?", "ajuda"),
            ("Há trabalho por aqui?", "ajuda"),
            ("Só vim aquecer. Obrigado.", "adeus"), ("Adeus, até amanhã.", "adeus"),
            ("Obrigado, vou ficar aqui.", "adeus"),
        ],
    },
}

for key, d in DIALOGUES.items():
    q = {"q": {"type": "choice", "instructions": "Which reply best fits the player's utterance?",
               "criteria": d["criteria"]}}
    for utt, exp in d["cases"]:
        lang = "en" if utt[0].isupper() and utt.split()[0].lower() in ("do", "who", "what", "thanks") else "pt"
        add("dialogue", lang, {"npc": d["npc"], "player_utterance": utt}, q, exp)

# ------------------------------------------------------------------ tone / moderation
TONE_Q = {"q": {"type": "noul",
                "instructions": ("Is the player threatening or insulting the merchant? A threat is a promise of "
                                 "violence, an insult, or an attempt to intimidate. Requests, complaints and polite "
                                 "talk are not threats."),
                "criteria": {"false": "not a threat or insult", "true": "a threat or insult"}}}

TONE = [
    ("pt", "Vou matar-te se não te calas.", True),
    ("pt", "Dá-me tudo o que tens ou morres.", True),
    ("pt", "Vou queimar a tua loja.", True),
    ("pt", "Vou partir-te os dentes.", True),
    ("pt", "Sai da frente ou empurro-te.", True),
    ("pt", "És um ladrão.", True),
    ("pt", "A tua banca é uma vergonha.", True),
    ("pt", "Aquele guarda é um covarde, e tu também.", True),
    ("pt", "Andas a dormir com os porcos?", True),
    ("pt", "Nunca devia ter comprado nada aqui.", False),
    ("pt", "O pão está caro.", False),
    ("pt", "Isto está frio.", False),
    ("pt", "Bom dia, tens quartos livres?", False),
    ("pt", "Quanto custa a poção?", False),
    ("pt", "Obrigado pela ajuda.", False),
    ("pt", "Aceitas esta encomenda?", False),
    ("pt", "Conte-me das estradas do norte.", False),
    ("pt", "Fico grato pela conversa.", False),
    ("en", "I will kill you if you do not shut up.", True),
    ("en", "Give me everything you have or die.", True),
    ("en", "I will burn your shop down.", True),
    ("en", "You are a thief.", True),
    ("en", "That guard is a coward, and so are you.", True),
    ("en", "Get out of my way or I will push you.", True),
    ("en", "The bread is expensive.", False),
    ("en", "This is cold.", False),
    ("en", "Good morning, do you have rooms?", False),
    ("en", "How much is the potion?", False),
    ("en", "Thank you for your help.", False),
    ("en", "Tell me about the northern roads.", False),
]
for lang, utt, threat in TONE:
    add("tone", lang, {"utterance": utt, "npc": "Greta, merchant"}, TONE_Q, threat)

# ------------------------------------------------------------------ action verification
ACTION_Q = {"q": {"type": "noul", "instructions": "Does the action log satisfy the requirement?",
                  "criteria": {"false": "the log does not show the required action",
                               "true": "the log shows the required action"}}}
ACTION = [
    ("pt", "o herói matou lobo em Floresta Sombria", "matar um lobo", True),
    ("pt", "o herói visitou o poço da encruzilhada", "matar um lobo", False),
    ("pt", "o herói visitou o poço da encruzilhada", "visitar o poço", True),
    ("pt", "o herói equipou tocha na floresta", "equipar uma tocha", True),
    ("pt", "o herói equipou tocha na floresta", "comprar uma tocha", False),
    ("pt", "o herói comprou poção na vila", "comprar uma poção", True),
    ("pt", "o herói vendeu pele de lobo na vila", "vender uma pele de lobo", True),
    ("pt", "o herói usou machado na floresta", "usar um machado", True),
    ("pt", "o herói usou machado na floresta", "equipar um machado", False),
    ("pt", "o herói matou bandido na estrada", "matar um bandido", True),
    ("pt", "o herói matou bandido na estrada", "matar um goblin", False),
    ("pt", "o herói visitou a torre do posto", "visitar o círculo de pedras", False),
    ("en", "the hero killed a wolf in the Dark Forest", "kill a wolf", True),
    ("en", "the hero visited the crossroads well", "kill a wolf", False),
    ("en", "the hero visited the crossroads well", "visit the well", True),
    ("en", "the hero equipped a torch in the forest", "equip a torch", True),
    ("en", "the hero equipped a torch in the forest", "buy a torch", False),
    ("en", "the hero bought a potion in the village", "buy a potion", True),
    ("en", "the hero sold a wolf pelt in the village", "sell a wolf pelt", True),
    ("en", "the hero used an axe in the forest", "use an axe", True),
    ("en", "the hero used an axe in the forest", "equip an axe", False),
    ("en", "the hero killed a bandit on the road", "kill a bandit", True),
    ("en", "the hero killed a bandit on the road", "kill a goblin", False),
    ("en", "the hero visited the outpost tower", "visit the stone circle", False),
]
for lang, log, req, ok in ACTION:
    add("action", lang, {"action_log": log, "requirement": req}, ACTION_Q, ok)

# ------------------------------------------------------------------ riddle match
RIDDLE_Q = {"q": {"type": "noul", "instructions": "Does the answer name the creature or thing described by the clue?",
                  "criteria": {"false": "the answer names something else or is unrelated",
                               "true": "the answer names what the clue describes"}}}
RIDDLES = [
    ("en", "A deep howl echoes three times. Large paw prints mark the mud.", "a pack of wolves", True),
    ("en", "A deep howl echoes three times. Large paw prints mark the mud.", "a shadow", False),
    ("en", "A deep howl echoes three times. Large paw prints mark the mud.", "goblins", False),
    ("en", "A deep howl echoes three times. Large paw prints mark the mud.", "a bear", False),
    ("en", "One offering is heavy, one burns, and one feeds.", "stone, wood and fruit", True),
    ("en", "One offering is heavy, one burns, and one feeds.", "stone and wood", False),
    ("en", "One offering is heavy, one burns, and one feeds.", "gold, silver and bronze", False),
    ("en", "Two statues howl; the third casts no shadow.", "the shadow", True),
    ("en", "Two statues howl; the third casts no shadow.", "the wolf", False),
    ("en", "Two statues howl; the third casts no shadow.", "the third statue", False),
    ("pt", "Um uivo grave ecoa três vezes. Pegadas grandes marcam a lama.", "alcateia", True),
    ("pt", "Um uivo grave ecoa três vezes. Pegadas grandes marcam a lama.", "sombra", False),
    ("pt", "Um uivo grave ecoa três vezes. Pegadas grandes marcam a lama.", "goblins", False),
    ("pt", "Uma oferenda pesa, outra arde e a última alimenta.", "pedra, madeira e fruta", True),
    ("pt", "Uma oferenda pesa, outra arde e a última alimenta.", "pedra e madeira", False),
    ("pt", "Uma oferenda pesa, outra arde e a última alimenta.", "ouro, prata e bronze", False),
    ("pt", "Duas estátuas uivam; a terceira não tem sombra.", "sombra", True),
    ("pt", "Duas estátuas uivam; a terceira não tem sombra.", "lobos", False),
    ("pt", "Duas estátuas uivam; a terceira não tem sombra.", "a terceira estátua", False),
    ("pt", "Duas estátuas uivam; a terceira não tem sombra.", "pedra", False),
]
for lang, clue, ans, ok in RIDDLES:
    add("riddle", lang, {"clue": clue, "answer": ans}, RIDDLE_Q, ok)

# ------------------------------------------------------------------ intent
INTENTS = {
    "buy": "purchase an item from the merchant",
    "sell": "sell an item to the merchant",
    "talk": "chat, greet someone or hear news",
    "attack": "attack or threaten violence",
    "travel": "go somewhere or ask for directions",
    "use": "use or equip an item",
    "deliver": "hand over an item",
    "ask": "ask a factual question about the world",
    "none": "no clear request",
}
INTENT_Q = {"q": {"type": "choice", "instructions": "What does the player want to do?",
                  "criteria": INTENTS}}
INTENT_CASES = [
    ("pt", "quero comprar uma poção", "buy"), ("pt", "vende-me a espada", "buy"),
    ("pt", "levo duas tochas", "buy"), ("en", "I want to buy a torch", "buy"),
    ("pt", "quero vender esta pele", "sell"), ("pt", "compras peles de lobo?", "sell"),
    ("pt", "vendo isto por dez moedas", "sell"), ("en", "I want to sell this pelt", "sell"),
    ("pt", "olá, como estás?", "talk"), ("pt", "tens novidades?", "talk"),
    ("pt", "fala-me da vila", "talk"), ("en", "hello, how are you?", "talk"),
    ("pt", "vou matar-te", "attack"), ("pt", "ataca o goblin", "attack"),
    ("pt", "prepara-te para lutar", "attack"), ("en", "I will kill you", "attack"),
    ("pt", "onde fica a floresta?", "travel"), ("pt", "vamos para a cidadela", "travel"),
    ("pt", "como chego ao poço?", "travel"), ("en", "where is the forest?", "travel"),
    ("pt", "bebo a poção", "use"), ("pt", "equipo a espada", "use"),
    ("pt", "acendo a tocha", "use"), ("en", "I drink the potion", "use"),
    ("pt", "aqui está a encomenda", "deliver"), ("pt", "entrego a carta", "deliver"),
    ("pt", "toma isto", "deliver"), ("en", "here is the package", "deliver"),
    ("pt", "quem matou o ferreiro?", "ask"), ("pt", "o que é isto?", "ask"),
    ("pt", "porque está a ponte fechada?", "ask"), ("en", "who killed the smith?", "ask"),
    ("pt", "hmm", "none"), ("pt", "não sei", "none"), ("en", "leave me alone", "none"),
]
for lang, utt, exp in INTENT_CASES:
    add("intent", lang, {"utterance": utt}, INTENT_Q, exp)

with open(f"{OUT}/bench2.jsonl", "w") as f:
    for it in items:
        f.write(json.dumps(it, ensure_ascii=False) + "\n")

from collections import Counter
print(f"{len(items)} itens -> bench2.jsonl")
print(Counter(x["cat"] for x in items))
print(Counter(x["lang"] for x in items))
