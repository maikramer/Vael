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
--   say.say(text, secs)         -- fala do NPC que abriu a barra (orador
--                               -- guardado em `ask`; os callbacks correm no
--                               -- HUD e não herdam o `ctx.entity` do NPC)
--   say.cancel()
local M = {}

local INPUT = "say-input"
local ROW = "say"
local DEFAULT_TIMEOUT = 8.0
local THINK_TIMEOUT = 8.0
local DECISION_DIR = "assets/public/d1"
local NLU_DIR = "assets/public/nlu"

local st = {
  phase = "idle",          -- idle | input | waiting
  backend = nil,           -- decision | nlu
  id = nil,
  timer = 0,
  opts = nil,
  said = "",
  typed = "",
  speaker = nil,           -- bits da entidade que abriu a barra (NPC)
  init = false,
  load_error = nil,
}

function M.active()
  return st.phase ~= "idle"
end

-- Fala atribuída ao NPC que abriu a barra. Usar nos callbacks: o `M.tick`
-- corre no `on_update` do HUD, por isso o `viber.say` sem orador punha a
-- linha em cima do HUD (sem Transform) e caía no toast.
function M.say(text, secs)
  viber.say(text, secs, st.speaker)
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
  st.speaker = nil
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
  -- Como no `finish`: o callback de cancelamento ainda fala pelo NPC.
  if opts and opts.on_cancel then
    opts.on_cancel()
  end
  if st.opts == opts then
    hide()
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
  -- O NLU (GLiNER2) é o backend legado da barra (relatos sem `questions`),
  -- o fallback do enigma e a extração de entidades do juiz. Ninguém o
  -- carregava: `ready()` ficava false para sempre e esses caminhos degradavam
  -- em silêncio. O load é async, idempotente e falha macio sem os ficheiros.
  if viber.nlu and viber.nlu.load then
    viber.nlu.load(NLU_DIR)
  end
end

local function tr(text)
  -- O prompt da barra vai por `viber.ui.set`, que NÃO passa pelos embrulhos
  -- de saída (`viber.toast/say/...`): sem isto as perguntas dos enigmas e dos
  -- dadores ficavam em PT no modo EN apesar de terem entrada no
  -- `locale/strings_en.json`. O `lib/i18n.lua` instala `viber.__i18n_tr`
  -- quando o `lib/rules.lua` carrega (todos os que abrem a barra já o fizeram).
  local fn = viber.__i18n_tr
  if type(fn) == "function" and type(text) == "string" and text ~= "" then
    return fn(text)
  end
  return text
end

function M.ask(opts)
  M.init()
  -- Capturado AQUI (o `ask` corre no on_update do NPC); os callbacks correm
  -- depois no HUD, onde `ctx.entity` já não é o NPC.
  local speaker = opts.speaker or (viber.entity_id and viber.entity_id())
  local function unavailable(err)
    -- O callback fala pelo NPC mesmo quando a barra não abre.
    local previous = st.speaker
    st.speaker = speaker
    if opts.on_unavailable then
      opts.on_unavailable(err)
    end
    -- Só repõe se o callback não abriu já outra barra (com o seu orador).
    if st.speaker == speaker then
      st.speaker = previous
    end
  end
  local use_decision = opts.questions ~= nil
  local ready
  if use_decision then
    ready = viber.decision and viber.decision.ready and viber.decision.ready()
  else
    ready = viber.nlu and viber.nlu.ready and viber.nlu.ready()
  end
  if not ready and not opts.allow_fallback then
    unavailable(st.load_error)
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
  st.speaker = speaker
  st.timer = opts.timeout or DEFAULT_TIMEOUT
  -- A frase base (e o exemplo/dica, que são conteúdo) traduzem-se SEPARADAS:
  -- a frase composta com o sufixo não é a chave do locale.
  local prompt = tr(opts.prompt or "Diz uma palavra.")
  if opts.example and opts.example ~= "" then
    prompt = prompt .. "  ·  ex.: " .. tr(opts.example)
  elseif opts.hint and opts.hint ~= "" then
    prompt = prompt .. "  ·  " .. tr(opts.hint)
  elseif opts.allow_fallback then
    prompt = prompt .. "  ·  " .. tr("Diga um verso elemental ou o nome de uma criatura...")
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
  -- O callback corre ANTES do `hide`: ainda tem `st.speaker` (o NPC), porque
  -- `M.say`/`talk.say` dentro dele é atribuído a quem falou. Se o próprio
  -- callback abrir outra barra, `st.opts` já não é este e não a fechamos.
  if opts and opts.on_result then
    opts.on_result(result, said)
  end
  if st.opts == opts then
    hide()
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
  local ready
  if st.backend == "decision" then
    ready = viber.decision and viber.decision.ready and viber.decision.ready()
  else
    ready = viber.nlu and viber.nlu.ready and viber.nlu.ready()
  end
  if not ready and st.opts and st.opts.allow_fallback then
    st.said = text
    finish(nil)
    return
  end
  st.phase = "waiting"
  st.said = text
  st.timer = THINK_TIMEOUT
  viber.ui.set("say.prompt", tr("A ouvir…"))
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
    local speaker = st.speaker
    hide()
    if opts and opts.on_unavailable then
      st.speaker = speaker
      opts.on_unavailable(err)
      if st.speaker == speaker then
        st.speaker = nil
      end
    end
    return
  end
  st.id = id
end

function M.tick(dt)
  M.init()
  local backend = st.backend
  local can_poll
  if backend == "decision" then
    can_poll = viber.decision and viber.decision.poll
  else
    can_poll = viber.nlu and viber.nlu.poll
  end
  if not can_poll and not (st.opts and st.opts.allow_fallback) then
    return
  end

  -- Um modal por cima da barra (diário, mapa, viagem, morte, título) tem de a
  -- fechar: o <UiInput> ficava focado por baixo do véu.
  if st.phase ~= "idle" and viber.menu_open and viber.menu_open() then
    M.cancel()
    return
  end
  -- O ecrã de MORTE não entra no `menu_open` (os 2,2 s de `dying` ainda são
  -- jogáveis). Sem isto a barra de fala ficava focada por cima dele: o
  -- Enter/dígitos do "Tentar de novo" iam para o campo de fala e a entrega
  -- corria na callback (dador a pagar a um cadáver).
  if st.phase ~= "idle" and viber.ui and viber.ui.is_open
      and viber.ui.is_open("death") then
    M.cancel()
    return
  end

  -- O UiInput pode limpar o texto ao desfocar; guarda o último valor não
  -- vazio do frame para o `focus_changed` não submeter uma string vazia.
  if st.phase == "input" and viber.ui.read then
    local rd = viber.ui.read(INPUT)
    local txt = rd and rd.text or ""
    if txt ~= "" and txt ~= st.typed then
      -- O timeout é de OCIOSIDADE, não do total da barra: quem demora a
      -- pensar/escrever via a barra fechar antes do Enter (o timeout conta
      -- só enquanto o campo não muda).
      st.timer = st.opts and st.opts.timeout or DEFAULT_TIMEOUT
    end
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
  if can_poll then
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
        local speaker = st.speaker
        hide()
        if opts and opts.on_unavailable then
          st.speaker = speaker
          opts.on_unavailable(r.error)
          if st.speaker == speaker then
            st.speaker = nil
          end
        end
        return
      end
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
        M.say("Não percebi. Tenta outra vez.")
      end
      M.cancel()
    end
  end
end

return M
