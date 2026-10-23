-- Plugin do Vael: validação cruzada do conteúdo.
--
-- Não edita nada: lê quests, diálogos, itens, marcos, progressão e locale e
-- publica findings no Problems (e num painel próprio). É o antigo lint de
-- quests que vivia na engine, agora do lado do jogo.

local BIOMES = { city = true, dark_forest = true, desert = true, swamp = true, frozen_peaks = true }

local state = {
  findings = nil,
}

local function read_json(ctx, path)
  local text = ctx.editor.fs.read(path)
  if not text then
    return nil
  end
  local value = ctx.editor.fs.json.decode(text)
  return value
end

local function load_items(ctx)
  local data = read_json(ctx, "data/items.json") or {}
  local out = {}
  for _, item in ipairs(data.items or {}) do
    if type(item) == "table" and item.id then
      out[item.id] = item
    end
  end
  return out
end

local function load_landmarks(ctx)
  local data = read_json(ctx, "data/landmarks.json") or {}
  local out = {}
  for _, mark in ipairs(data.landmarks or {}) do
    if type(mark) == "table" and mark.name then
      out[mark.name] = mark
    end
  end
  return out
end

local function load_quests(ctx)
  local files = {}
  local quests = {}
  local index = read_json(ctx, "quests/index.json") or {}
  for _, name in ipairs(index) do
    local list = read_json(ctx, "quests/" .. name)
    files[#files + 1] = { name = name, quests = list or {} }
    for _, quest in ipairs(list or {}) do
      if type(quest) == "table" and quest.id then
        quests[#quests + 1] = { file = name, quest = quest }
      end
    end
  end
  return files, quests
end

local function load_dialogues(ctx)
  local out = {}
  local entries = ctx.editor.fs.list("dialogues")
  for _, entry in ipairs(entries or {}) do
    if not entry.dir and entry.name:match("%.json$") then
      out[#out + 1] = {
        name = entry.name,
        id = entry.name:gsub("%.json$", ""),
        graph = read_json(ctx, entry.path) or {},
      }
    end
  end
  return out
end

local function load_locales(ctx)
  return {
    pt = read_json(ctx, "locale/pt.json") or {},
    en = read_json(ctx, "locale/en.json") or {},
  }
end

local function entity_exists(ctx, name)
  return #ctx.editor.find_nodes({ name = name, limit = 1 }) > 0
end

local function validate(ctx)
  local findings = {}
  local function add(severity, kind, message)
    findings[#findings + 1] = { severity = severity, kind = kind, message = message }
  end

  local items = load_items(ctx)
  local landmarks = load_landmarks(ctx)
  local files, quests = load_quests(ctx)
  local quest_ids = {}
  for _, entry in ipairs(quests) do
    local quest = entry.quest
    local id = tostring(quest.id or "")
    if id == "" then
      add("error", "quest-id", entry.file .. ": quest sem id")
    elseif quest_ids[id] then
      add("error", "quest-id", entry.file .. ": id duplicado: " .. id)
    end
    quest_ids[id] = true
    if not BIOMES[quest.biome] then
      add("warning", "quest-biome", id .. ": bioma inválido: " .. tostring(quest.biome))
    end
    local npc = tostring(quest.npc or "")
    if npc == "" then
      add("error", "quest-npc", id .. ": npc vazio")
    elseif npc ~= "notice_board" and not entity_exists(ctx, npc) then
      add("error", "quest-npc", id .. ": npc inexistente: " .. npc)
    end
    local obj = type(quest.objective) == "table" and quest.objective or {}
    local target = tostring(obj.target or "")
    if target == "" then
      add("error", "quest-objective", id .. ": objective.target vazio")
    elseif obj.type == "visit" then
      for place in target:gmatch("%S+") do
        if not entity_exists(ctx, place) then
          add("error", "quest-visit", id .. ": local inexistente: " .. place)
        elseif not landmarks[place] then
          add("warning", "quest-visit", id .. ": local fora de landmarks.json: " .. place)
        end
      end
    elseif obj.type == "collect" and not items[target] then
      add("warning", "quest-collect", id .. ": item de objetivo desconhecido: " .. target)
    end
    local parley = type(quest.parley) == "table" and quest.parley or nil
    if not parley then
      if obj.type ~= "decision" then
        add("error", "quest-parley", id .. ": sem parley (relato de 1 palavra)")
      end
    else
      local rep = type(parley.report) == "table" and parley.report or nil
      if not rep then
        add("error", "quest-parley", id .. ": parley.report ausente")
      else
        if tostring(rep.prompt or "") == "" then
          add("warning", "quest-parley", id .. ": parley.report.prompt vazio")
        end
        if tostring(rep.example or "") == "" then
          add("warning", "quest-parley", id .. ": parley.report.example vazio")
        end
        local expect = type(rep.expect) == "table" and rep.expect or {}
        if #expect == 0 then
          add("error", "quest-parley", id .. ": parley.report.expect vazio")
        end
        for _, e in ipairs(expect) do
          local okind, eid = tostring(e.kind or ""), tostring(e.id or "")
          if okind == "item" and not items[eid] then
            add("error", "quest-parley", id .. ": item de parley desconhecido: " .. eid)
          elseif okind == "local" and not landmarks[eid] then
            add("warning", "quest-parley", id .. ": local de parley fora de landmarks.json: " .. eid)
          elseif okind ~= "criatura" and okind ~= "item" and okind ~= "local" then
            add("error", "quest-parley", id .. ": parley kind inválido: " .. okind)
          end
        end
      end
    end
    for _, entry_item in ipairs(type(quest.rewards) == "table" and quest.rewards.items or {}) do
      local item = tostring(entry_item):match("^([^:]+)")
      if item and not items[item] then
        add("warning", "quest-reward", id .. ": item de recompensa desconhecido: " .. item)
      end
    end
  end
  for name, mark in pairs(landmarks) do
    if not entity_exists(ctx, name) then
      add("error", "landmark-entity", "marco sem entidade no mundo: " .. name)
    end
    if mark.survey and not quest_ids[mark.survey] then
      add("warning", "landmark-survey", name .. ": survey inexistente: " .. tostring(mark.survey))
    end
  end
  local locales = load_locales(ctx)
  for _, dialogue in ipairs(load_dialogues(ctx)) do
    local nodes = type(dialogue.graph.nodes) == "table" and dialogue.graph.nodes or {}
    if not nodes[dialogue.graph.start] then
      add("error", "dialogue-start", dialogue.name .. ": start inexistente")
    end
    for node_id, node in pairs(nodes) do
      local text = tostring(node.text or "")
      if text:sub(1, 1) == "@" then
        local key = text:sub(2)
        if not locales.pt[key] then
          add("warning", "dialogue-locale", dialogue.name .. ": chave em falta em pt: " .. key)
        end
        if not locales.en[key] then
          add("warning", "dialogue-locale", dialogue.name .. ": chave em falta em en: " .. key)
        end
      end
      for _, choice in ipairs(node.choices or {}) do
        if tostring(choice.goto or "") ~= "" and not nodes[choice.goto] then
          add("error", "dialogue-goto", dialogue.name .. "/" .. node_id .. ": goto inexistente: " .. tostring(choice.goto))
        end
        if (choice.quest_accept or choice.quest_turn_in) and not quest_ids[dialogue.id] then
          add("error", "dialogue-quest", dialogue.name .. "/" .. node_id .. ": quest `" .. dialogue.id .. "` inexistente")
        end
      end
    end
  end
  return findings
end

local function run(ctx)
  state.findings = validate(ctx)
  local errors = 0
  for _, finding in ipairs(state.findings) do
    if finding.severity == "error" then
      errors = errors + 1
    end
  end
  ctx.editor.toast(errors > 0 and "warn" or "success",
    "Conteúdo: " .. #state.findings .. " finding(s), " .. errors .. " erro(s).")
  return state.findings
end

local function render(ctx)
  local children = {
    { type = "heading", text = "Conteúdo" },
    { type = "label", text = "Validação cruzada de quests, diálogos, itens, marcos e locale.", tone = "dim" },
    { type = "button", id = "run", text = "Validar tudo", primary = true },
  }
  if state.findings then
    children[#children + 1] = {
      type = "label",
      text = #state.findings .. " finding(s)",
      tone = #state.findings > 0 and "warn" or "dim",
    }
    for _, finding in ipairs(state.findings) do
      children[#children + 1] = {
        type = "label",
        text = "[" .. finding.kind .. "] " .. finding.message,
        tone = finding.severity == "error" and "error" or "warn",
      }
    end
  end
  return { type = "column", gap = 4, children = children }
end

local function on_event(ctx, event)
  if event.panel == "content_lint" and event.kind == "click" and event.id == "run" then
    run(ctx)
  end
end

local function on_command(ctx, id)
  if id == "run" then
    run(ctx)
    ctx.editor.open_panel("content_lint")
  end
end

local function lint(ctx)
  state.findings = validate(ctx)
  return state.findings
end

return {
  api = 1,
  id = "content_lint",
  title = "Conteúdo",
  on_event = on_event,
  on_command = on_command,
  panels = {
    { id = "content_lint", title = "Conteúdo", render = render },
  },
  commands = {
    { id = "run", title = "Conteúdo: validar tudo", keywords = "lint quests diálogos itens" },
  },
  lint = lint,
}
