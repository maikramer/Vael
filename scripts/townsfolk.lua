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

-- Alvo de sinais: `connect-opened="@discordia:notice"` num baú faz os
-- moradores do grupo reagirem (vira-se para o player, abana a cabeça e,
-- com `prop-notice`, comenta). Os argumentos do sinal não interessam aqui.
function notice(...)
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

local function show_dialogue(st)
  local body = node_body(st.graph, cursor(st))
  if body then
    viber.say(body)
    viber.gesture("talk")
    -- A hotbar também lê 1/2: enquanto a conversa está no ecrã, os dígitos
    -- são escolhas e não poção/antídoto. Timestamp de UI (não vai ao save).
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
  end
  show_dialogue(st)
  return true
end

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (o top-level é partilhado)
  if not st.ready then
    st.ready = true
    st.gesture_t = 3 + math.random() * 4 -- 1.º gesto cedo
    -- O [E] é SEMPRE registado: com `dialogue_id` o grafo corre aqui (Lua);
    -- sem ele, a conversa é a banter. O foco de interação escolhe o NPC
    -- mais próximo e só ele vê `viber.interacted("e")`.
    st.dialogue_id = viber.prop("dialogue_id")
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
        if viber.interacted("e") then
          st.talking = true
          show_dialogue(st)
        end
        -- Escolhas só com a conversa ABERTA: antes, a 6 m de distância (o
        -- `talk_range`), 1/2 avançavam o grafo e ainda bebiam poções.
        if st.talking then
          for digit = 1, 9 do
            if viber.input.pressed(tostring(digit)) then
              choose(st, digit)
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
  st.talking = false
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
