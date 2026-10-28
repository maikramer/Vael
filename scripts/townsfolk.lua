-- townsfolk.lua: o NPC civil do mundo. Passeia perto de casa; perto do player
-- para, vira-se e gesticula de vez em quando; [E] troca conversa.
--
-- Um só script para a rua, as salas e os balcões: o que muda é DADO de cada
-- instância (`prop-*` no XML, Inspector › Script no editor, `viber scene set`):
--   * rua (default): passeio de 4 m a 1.1 m/s;
--   * dentro de casa: `prop-radius="1.6" prop-speed="0.6"` — com o passeio de
--     rua o NPC atravessava a parede;
--   * ao balcão: `prop-radius="0"` — fica no lugar e atende.
--@export speed: number(0..4) = 1.1                 -- velocidade a passear (m/s)
--@export radius: number(0..20) = 4                 -- raio do passeio à volta de casa (m); 0 = fica no lugar
--@export talk_range: number(0.5..15) = 4.5        -- abaixo desta distância para e vira-se para o player (m)
--@export interact_range: number(0.5..10) = 3.5    -- alcance do [E] (m)
--@export prompt: string = "Falar"                  -- texto do prompt de interação
--@export dialogue_id: string = ""                  -- grafo em dialogues/; não vazio = conversa por grafo
--@export banter: string[] = "“Bom dia para uma caminhada, não achas?”" | "“Dizem que há ferro novo no mercado.”" | "“Cuidado com os lobos a norte.”"
--@export gestures: string[] = wave | talk | yes | no   -- gestos ao acaso enquanto o player está perto
--@export notice: string = ""                      -- fala quando reage a um sinal (`notice`); "" = só se vira
--@export notice_range: number(0..60) = 18         -- só reage com o player a menos disto (m)

local rules = viber.load("lib/rules.lua")
local talk = viber.load("ui/say.lua")
local travel = viber.load("game/travel.lua")
local marks = viber.load("lib/landmarks.lua")
local parley = viber.load("lib/parley.lua")
local decide = viber.load("lib/decide.lua")

local function typing()
  return viber.ui and viber.ui.focused and viber.ui.focused() ~= nil
end

-- O mesmo fold do `parley.strip` (minúsculas, acentos, pontuação, espaços).
-- O `raw` do parley chega SEM acentos e o label autoral mantém-nos, por isso
-- o caminho `exact` ("Atalaia do Pântano" == "atalaia do pântano") quase nunca
-- disparava e o pin ficava com o marco mais PRÓXIMO do conjunto em vez do nome.
local function fold(s)
  s = tostring(s or ""):lower()
  s = s:gsub("á", "a"):gsub("à", "a"):gsub("â", "a"):gsub("ã", "a"):gsub("ä", "a")
  s = s:gsub("é", "e"):gsub("è", "e"):gsub("ê", "e"):gsub("ë", "e")
  s = s:gsub("í", "i"):gsub("ì", "i"):gsub("î", "i"):gsub("ï", "i")
  s = s:gsub("ó", "o"):gsub("ò", "o"):gsub("ô", "o"):gsub("õ", "o"):gsub("ö", "o")
  s = s:gsub("ú", "u"):gsub("ù", "u"):gsub("û", "u"):gsub("ü", "u")
  s = s:gsub("ç", "c")
  s = s:gsub("%p", " ")
  s = s:gsub("[-_]", " ")
  s = s:gsub("%s+", " ")
  return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

-- "Perguntar por um lugar": o d1 resolve frases; os aliases autorais
-- resolvem uma palavra solta ("poço", "atalaia") que o `choice` rejeita.
local function alias_place(text)
  local res = parley.resolve(text, nil)
  local raw = fold(res.raw)
  local best, best_d, exact
  local has, px, _, pz = viber.player_position()
  for id in pairs(res["local"] or {}) do
    local label = fold(marks.label(id))
    if raw ~= "" and label == raw and not exact then
      exact = id
    end
    local x, _, z = marks.position(id)
    if x and has then
      local dx, dz = x - px, z - pz
      local d = dx * dx + dz * dz
      if not best_d or d < best_d then
        best, best_d = id, d
      end
    elseif not best then
      best = id
    end
  end
  return exact or best
end

-- "Perguntar por um lugar": uma palavra resolve o marco e marca o waypoint.
local function ask_place()
  viber.say("Que lugar?")
  local options = {}
  for _, m in ipairs(marks.LIST or {}) do
    options[#options + 1] = { key = m.name, description = m.label or m.name }
  end
  options[#options + 1] = { key = "none", description = "nenhum lugar conhecido" }
  if decide.ready() then
    talk.ask({
      prompt = "Que lugar?",
      example = "poço, atalaia, mojão",
      state = { npc = "morador da vila" },
      questions = { q = decide.choice("Qual lugar o jogador quer marcar no mapa?", options) },
      on_result = function(answers, text)
        local id = answers.q and answers.q.choice
        if not id or id == "none" then
          id = alias_place(text)
        end
        if not id then
          viber.say("Não conheço esse lugar.")
        else
          travel.pin(id)
          viber.say("Marquei no teu mapa.")
        end
      end,
      on_cancel = function()
        viber.say("Fica para outra vez.")
      end,
      on_unavailable = function()
        viber.say("Não te ouvi.")
      end,
    })
    return
  end
  viber.say("Não te ouvi.")
end

-- Alvo de sinais: `connect-opened="@discordia:notice"` num baú faz os
-- moradores do grupo reagirem (vira-se para o player, abana a cabeça e,
-- com `prop-notice`, comenta). Os argumentos do sinal não interessam aqui.
function notice(...)
  -- Menu/diário/mapa abertos: o jogador parou para ler; um sinal de baú não
  -- pode virar o NPC e atirar um toast por cima do menu (os restantes
  -- handlers já devolvem cedo com `menu_open`).
  if viber.menu_open() then return end
  local has, px, _, pz = viber.player_position()
  if not has then return end
  local x, _, z = viber.position()
  if math.sqrt((px - x) ^ 2 + (pz - z) ^ 2) > viber.prop("notice_range") then return end
  viber.face_player()
  viber.gesture("no")
  local line = viber.prop("notice")
  if line ~= "" then viber.toast(line) end
end

-- ── grafo de diálogo em Lua ────────────────────────────────────────────
-- O formato é o mesmo de `dialogues/<id>.json` (nós, escolhas 1..9,
-- `quest_accept`/`quest_turn_in`). O cursor vive em `viber.game()` para o
-- save o guardar; `@chave` resolve pelo locale ativo.

local function graph_for(id)
  local ok, graph = pcall(viber.load_json, "dialogues/" .. id .. ".json")
  if ok and type(graph) == "table" and type(graph.nodes) == "table" then
    return graph
  end
  return nil
end

local function node_body(graph, node_id)
  local node = graph.nodes[node_id]
  if not node then
    return nil
  end
  local speaker = viber.localize(node.speaker or "")
  local text = viber.localize(node.text or "")
  local body = speaker ~= "" and (speaker .. ": " .. text) or text
  for i, choice in ipairs(node.choices or {}) do
    body = body .. "\n" .. i .. ". " .. viber.localize(choice.text or "")
  end
  body = body .. "\n9. Perguntar por um lugar"
  return body, node
end

local function cursor(st)
  local id = st.dialogue_id
  local saved = viber.game()["dialogue." .. id]
  if saved and st.graph.nodes[saved] then
    return saved
  end
  return st.graph.start
end

local BALLOON_SECS = 4.0 -- `hud::BALLOON_DURATION`: o balão nativo dura 4 s

local function show_dialogue(st)
  local body, node = node_body(st.graph, cursor(st))
  if body then
    viber.say(body)
    -- `voice` é campo documentado no README de dialogues/ (clip de SFX
    -- opcional por nó); nenhum leitor o tocava — a doc era inerte. Um clip
    -- desconhecido é erro de script com warn 1× (a conversa sobrevive).
    local voice = node and node.voice
    if type(voice) == "string" and voice ~= "" then
      viber.sound(voice)
    end
    viber.gesture("talk")
    -- A hotbar também lê 1/2: enquanto o BALÃO está no ecrã, os dígitos são
    -- escolhas e não poção/antídoto. Depois de 4 s a conversa expira (antes
    -- renovávamos para sempre e beber uma poção perto do NPC era impossível).
    st.dialog_until = viber.time() + BALLOON_SECS
    viber.ui.set("dialogue.until", viber.time() + 0.75)
  end
end

local function choose(st, digit)
  local node = st.graph.nodes[cursor(st)]
  local choice = node and (node.choices or {})[digit]
  if not choice then
    return false
  end
  local id = st.dialogue_id
  if choice.quest_accept then
    rules.quest_accept(id)
  end
  if choice.quest_turn_in then
    rules.quest_turn_in(id)
  end
  if choice.goto and st.graph.nodes[choice.goto] then
    viber.game()["dialogue." .. id] = choice.goto
  elseif not choice.goto or choice.goto == "" then
    -- O editor grava `goto` vazio como "(fim)". Sem isto o balão voltava a
    -- abrir o MESMO nó no [E] seguinte e a conversa não tinha saída.
    viber.game()["dialogue." .. id] = nil
    st.talking = false
    viber.ui.set("dialogue.until", 0)
    return true
  end
  show_dialogue(st)
  return true
end

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (o top-level é partilhado)
  -- Relógio para trás = load: um `st.talking` de outra sessão não pode
  -- manter a hotbar trancada até o relógio alcançar o timestamp antigo.
  local now = viber.time()
  if st.clock and now < st.clock then
    st.talking = false
    st.dialog_until = 0
    viber.ui.set("dialogue.until", 0)
  end
  st.clock = now
  if not st.ready then
    st.ready = true
    st.gesture_t = 3 + math.random() * 4 -- 1.º gesto cedo
    -- O [E] é SEMPRE registado: com `dialogue_id` o grafo corre aqui (Lua);
    -- sem ele, a conversa é a banter. O foco de interação escolhe o NPC
    -- mais próximo e só ele vê `viber.interacted("e")`.
    st.dialogue_id = viber.prop("dialogue_id")
    -- Um bind de diálogo de outra sessão/cena não pode trancar a hotbar.
    viber.ui.set("dialogue.until", 0)
    viber.set_interaction(viber.prop("prompt"), "e", viber.prop("interact_range"))
    if st.dialogue_id ~= "" then
      st.graph = graph_for(st.dialogue_id)
    end
  end
  -- Um JSON ilegível no arranque (ficheiro renomeado, IO transitório)
  -- deixava o NPC sem conversa para sempre: tenta de novo de 2 em 2 s.
  if st.dialogue_id ~= "" and not st.graph then
    st.graph_t = (st.graph_t or 0) - dt
    if st.graph_t <= 0 then
      st.graph_t = 2.0
      st.graph = graph_for(st.dialogue_id)
    end
  end
  local has, px, py, pz = viber.player_position()
  if not has then return end
  local x, y, z = viber.position()
  local dist = math.sqrt((px - x) ^ 2 + (pz - z) ^ 2)
  if dist < viber.prop("talk_range") then
    viber.face_player()
    st.gesture_t = st.gesture_t - dt
    if st.gesture_t <= 0 then
      st.gesture_t = 7 + math.random() * 8 -- 7–15 s
      local gestures = viber.prop("gestures")
      if #gestures > 0 then
        viber.gesture(gestures[math.random(#gestures)])
      end
    end
    if not viber.menu_open() then
      if st.dialogue_id ~= "" and st.graph then
        if viber.interacted("e") and not typing() and not talk.active() then
          st.talking = true
          show_dialogue(st)
        end
        -- Escolhas só com a conversa ABERTA: antes, a 6 m de distância (o
        -- `talk_range`), 1/2 avançavam o grafo e ainda bebiam poções.
        if st.talking and viber.time() >= (st.dialog_until or 0) then
          -- O balão expirou (4 s sem escolha): a conversa fechou. Sem isto o
          -- `st.talking` ficava true para sempre e a hotbar/equipamento
          -- continuavam trancados enquanto o jogador estivesse perto.
          st.talking = false
          viber.ui.set("dialogue.until", 0)
        end
        if st.talking and not typing() and not talk.active() then
          -- Renova o portão da hotbar/equipamento ENQUANTO o balão está no
          -- ecrã (o timestamp de 0,75 s expirava se o jogador lesse devagar).
          viber.ui.set("dialogue.until", viber.time() + 0.75)
          for digit = 1, 9 do
            if viber.input.pressed(tostring(digit)) then
              if digit == 9 then
                ask_place()
              else
                choose(st, digit)
              end
              break
            end
          end
        end
      elseif (st.dialogue_id == "" or not st.graph) and viber.interacted("e") then
        -- Grafo em falta não pode matar a conversa: cai para a banter.
        local banter = viber.prop("banter")
        if #banter > 0 then
          st.i = ((st.i or 0) % #banter) + 1
          viber.toast(banter[st.i])
        end
        viber.gesture("talk")
      end
    end
    return
  end
  if st.talking then
    st.talking = false
    viber.ui.set("dialogue.until", 0)
  end
  local radius = viber.prop("radius")
  if radius <= 0 then return end -- balcão: não passeia
  if st.target == nil then
    local tx, tz = viber.wander_target(radius)
    st.target = { tx, tz }
  end
  local td = math.sqrt((st.target[1] - x) ^ 2 + (st.target[2] - z) ^ 2)
  -- Chegou: o limiar acompanha o raio (numa sala de 1.6 m, 0.6 m era meio passeio).
  if td < math.min(0.6, radius * 0.25) then
    st.target = nil
    st.stuck, st.last_td = 0, nil
    return
  end
  -- anti-stuck: td sem diminuir = preso num collider; novo destino (mais cedo
  -- em salas pequenas, onde o NPC encosta mais vezes a móveis/paredes).
  if st.last_td ~= nil and td >= st.last_td then
    st.stuck = (st.stuck or 0) + dt
    if st.stuck > math.min(6, 1 + radius) then
      local tx, tz = viber.wander_target(radius)
      st.target = { tx, tz }
      st.stuck, st.last_td = 0, nil
      return
    end
  else
    st.stuck = 0
  end
  st.last_td = td
  viber.move_towards(st.target[1], st.target[2], viber.prop("speed"))
end
