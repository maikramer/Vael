-- Barra de fala: o NPC pergunta, o jogador responde por texto.
-- Máquina de estados sem bloquear o frame: ask -> focus -> submit -> poll.
--
-- Dois motores, o mesmo ecrã:
--   * `questions` (novo): `viber.decision` (d1-omni) classifica a fala contra
--     opções tipadas (`choice`/`noul`/`score`). É o caminho dos NPCs.
--   * sem `questions` (legado): `viber.nlu` (GLiNER2) devolve entidades.
--
-- API:
--   say.init()                  -- chamar uma vez (hud.lua); pré-carrega o d1
--   say.ask{ prompt=, example=, questions=, state=, on_result=fn(result, text),
--            on_unavailable=fn(), on_cancel=fn(), timeout= }
--   say.tick(dt)                -- por frame (hud.lua)
--   say.active()                -- true enquanto a barra está aberta
--   say.cancel()
local M = {}

local INPUT = "say-input"
local ROW = "say"
local DEFAULT_TIMEOUT = 8.0
local THINK_TIMEOUT = 8.0
local DECISION_DIR = "assets/public/d1"

local st = {
  phase = "idle",          -- idle | input | waiting
  backend = nil,           -- decision | nlu
  id = nil,
  timer = 0,
  opts = nil,
  said = "",
  typed = "",
  init = false,
  load_error = nil,
}

function M.active()
  return st.phase ~= "idle"
end

local function blur()
  -- Focar um nó que não é UiInput limpa o foco (e devolve o teclado ao jogo).
  viber.ui.focus(ROW)
end

local function hide()
  st.phase = "idle"
  st.backend = nil
  st.id = nil
  st.opts = nil
  st.said = ""
  st.typed = ""
  st.timer = 0
  viber.ui.set("say.active", false)
  viber.ui.set_text(INPUT, "")
  -- Liberta a hotbar/equipamento: o bloqueio é renovado em `tick` enquanto a
  -- barra está aberta, por isso tem de morrer com ela.
  viber.ui.set("dialogue.until", 0)
  blur()
end

function M.cancel()
  local opts = st.opts
  hide()
  if opts and opts.on_cancel then
    opts.on_cancel()
  end
end

function M.init()
  if st.init then
    return
  end
  st.init = true
  viber.ui.set("say.active", false)
  viber.ui.set("say.prompt", "")
  viber.ui.set_text(INPUT, "")
  if viber.decision and viber.decision.available
      and viber.decision.available(DECISION_DIR) then
    viber.decision.load(DECISION_DIR)
  end
end

function M.ask(opts)
  M.init()
  local use_decision = opts.questions ~= nil
  if use_decision then
    if not (viber.decision and viber.decision.ready and viber.decision.ready()) then
      if opts.on_unavailable then
        opts.on_unavailable(st.load_error)
      end
      return false
    end
  elseif not (viber.nlu and viber.nlu.ready and viber.nlu.ready()) then
    if opts.on_unavailable then
      opts.on_unavailable(st.load_error)
    end
    return false
  end
  if st.phase ~= "idle" then
    hide()
  end
  -- Limpa eventos velhos (o blur do pedido anterior chega um frame atrasado
  -- e cancelaria este antes de o jogador escrever).
  if viber.ui.events then
    viber.ui.events(INPUT)
  end
  st.opts = opts
  st.backend = use_decision and "decision" or "nlu"
  st.phase = "input"
  st.id = nil
  st.typed = ""
  st.timer = opts.timeout or DEFAULT_TIMEOUT
  local prompt = opts.prompt or "Diz uma palavra."
  if opts.example and opts.example ~= "" then
    prompt = prompt .. "  ·  ex.: " .. opts.example
  end
  viber.ui.set("say.prompt", prompt)
  viber.ui.set_text(INPUT, "")
  viber.ui.set("say.active", true)
  viber.ui.focus(INPUT)
  -- Bloqueia hotbar/equipamento durante a fala (mesmo canal do diálogo).
  -- Os consumidores só honram timestamps a ≤2 s; o bloqueio é renovado em
  -- `tick` enquanto durar.
  viber.ui.set("dialogue.until", viber.time() + 1.0)
  return true
end

local function finish(result)
  local opts = st.opts
  local said = st.said
  hide()
  if opts and opts.on_result then
    opts.on_result(result, said)
  end
end

local function decision_state(opts, text)
  local state = opts.state
  if type(state) == "function" then
    state = state(text)
  end
  local s = {}
  if type(state) == "table" then
    for k, v in pairs(state) do
      s[k] = v
    end
  elseif state ~= nil then
    s.context = state
  end
  s.utterance = text
  return s
end

local function submit(text)
  text = tostring(text or ""):gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    hide()
    return
  end
  st.phase = "waiting"
  st.said = text
  st.timer = THINK_TIMEOUT
  viber.ui.set("say.prompt", "A ouvir…")
  viber.ui.set("dialogue.until", viber.time() + 1.0)
  local id, err
  if st.backend == "decision" then
    id, err = viber.decision.submit(decision_state(st.opts, text), st.opts.questions)
  else
    id, err = viber.nlu.submit(text)
  end
  if not id then
    st.phase = "idle"
    local opts = st.opts
    hide()
    if opts and opts.on_unavailable then
      opts.on_unavailable(err)
    end
    return
  end
  st.id = id
end

function M.tick(dt)
  M.init()
  local backend = st.backend
  if backend == "decision" then
    if not (viber.decision and viber.decision.poll) then
      return
    end
  elseif not (viber.nlu and viber.nlu.poll) then
    return
  end

  -- Um modal por cima da barra (diário, mapa, viagem, morte, título) tem de a
  -- fechar: o <UiInput> ficava focado por baixo do véu.
  if st.phase ~= "idle" and viber.menu_open and viber.menu_open() then
    M.cancel()
    return
  end

  -- O UiInput pode limpar o texto ao desfocar; guarda o último valor não
  -- vazio do frame para o `focus_changed` não submeter uma string vazia.
  if st.phase == "input" and viber.ui.read then
    local rd = viber.ui.read(INPUT)
    local txt = rd and rd.text or ""
    if txt ~= "" then
      st.typed = txt
    end
  end

  -- 1. Eventos do input (Enter/Esc/clique fora desfocam a engine).
  if viber.ui.events then
    local evs = viber.ui.events(INPUT)
    for i = 1, (evs.n or 0) do
      local ev = evs[i]
      if ev and ev.type == "focus_changed" and ev.focused == false and st.phase == "input" then
        local text = ""
        local rd = viber.ui.read(INPUT)
        if rd then
          text = rd.text or ""
        end
        if text == "" then
          text = st.typed or ""
        end
        if text ~= "" then
          submit(text)
        else
          M.cancel()
        end
      end
    end
  end

  -- 2. Resultados do worker. Drena sempre: o `loaded` pode chegar a qualquer
  --    momento e há resultados tardios de pedidos cancelados.
  while true do
    local r = backend == "decision" and viber.decision.poll() or viber.nlu.poll()
    if not r then
      break
    end
    if r.kind == "loaded" then
      if not r.ok then
        st.load_error = r.error
      end
    elseif backend == "decision" and r.kind == "decided"
        and st.phase == "waiting" and r.id == st.id then
      finish(r.answers)
      return
    elseif backend == "nlu" and r.kind == "parsed"
        and st.phase == "waiting" and r.id == st.id then
      finish(r)
      return
    elseif r.kind == "error" and st.phase == "waiting" and r.id == st.id then
      local opts = st.opts
      hide()
      if opts and opts.on_unavailable then
        opts.on_unavailable(r.error)
      end
      return
    end
  end

  -- 3. Timeouts.
  if st.phase ~= "idle" then
    -- Renova o bloqueio curto: a barra continua visível e os consumidores de
    -- `dialogue.until` só aceitam valores próximos (≤2 s do relógio atual).
    viber.ui.set("dialogue.until", viber.time() + 1.0)
    st.timer = st.timer - (dt or 0)
    if st.timer <= 0 then
      if st.phase == "waiting" then
        viber.say("Não percebi. Tenta outra vez.")
      end
      M.cancel()
    end
  end
end

return M
