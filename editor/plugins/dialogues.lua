-- Plugin do Vael: editor de grafos de diálogo (`dialogues/*.json`).
--
-- O runtime é `scripts/townsfolk.lua`; este plugin só escreve o mesmo
-- formato e valida referências (start, goto, locale e quests).

local state = {
  loaded = false,
  files = {},
  file_index = 0,
  graph = nil,
  selected_node = nil,
  original_node_id = nil,
  choice_index = nil,
  error = nil,
  pending_delete = nil,
  quests = nil,
  locale = nil,
}

local function copy(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, item in pairs(value) do
    out[key] = copy(item)
  end
  return out
end

local function trim(value)
  return tostring(value or ""):match("^%s*(.-)%s*$")
end

local function read_json(ctx, path)
  local text, err = ctx.editor.fs.read(path)
  if not text then
    return nil, err or ("não consegui ler " .. path)
  end
  return ctx.editor.fs.json.decode(text)
end

local function write_json(ctx, path, value)
  local text, err = ctx.editor.fs.json.encode(value, true)
  if not text then
    return false, err
  end
  return ctx.editor.fs.write(path, text)
end

local function load_quests(ctx)
  local ids = {}
  local index = read_json(ctx, "quests/index.json")
  for _, name in ipairs(index or {}) do
    local data = read_json(ctx, "quests/" .. name)
    for _, quest in ipairs(data or {}) do
      if type(quest) == "table" and quest.id then
        ids[quest.id] = true
      end
    end
  end
  state.quests = ids
end

local function load_locale(ctx)
  local data = read_json(ctx, "locale/pt.json")
  state.locale = type(data) == "table" and data or {}
end

local function load_all(ctx)
  state.files = {}
  state.error = nil
  local entries, err = ctx.editor.fs.list("dialogues")
  if not entries then
    state.error = err or "não consegui listar dialogues/"
    state.loaded = true
    return
  end
  for _, entry in ipairs(entries) do
    if not entry.dir and entry.name:match("%.json$") then
      local data, read_err = read_json(ctx, entry.path)
      if type(data) ~= "table" or type(data.nodes) ~= "table" then
        state.error = (state.error and (state.error .. "\n") or "") ..
          entry.path .. ": " .. tostring(read_err or "sem `nodes`")
      else
        state.files[#state.files + 1] = {
          name = entry.name,
          path = entry.path,
          graph = data,
        }
      end
    end
  end
  table.sort(state.files, function(a, b) return a.name < b.name end)
  load_quests(ctx)
  load_locale(ctx)
  local saved = ctx.editor.prefs.get("dialogues.file")
  state.file_index = 1
  for index, file in ipairs(state.files) do
    if file.name == saved then
      state.file_index = index
      break
    end
  end
  state.graph = state.files[state.file_index] and state.files[state.file_index].graph or nil
  state.selected_node = nil
  state.choice_index = nil
  state.original_node_id = nil
  state.loaded = true
end

local function current_file()
  return state.files[state.file_index]
end

local function sorted_nodes()
  local out = {}
  for id in pairs(state.graph and state.graph.nodes or {}) do
    out[#out + 1] = id
  end
  table.sort(out)
  return out
end

local function node_options(include_empty)
  local out = {}
  if include_empty then
    out[#out + 1] = { value = "", label = "(fim)" }
  end
  for _, id in ipairs(sorted_nodes()) do
    out[#out + 1] = { value = id, label = id }
  end
  return out
end

local function selected_node()
  if not state.graph or not state.selected_node then
    return nil
  end
  return state.graph.nodes[state.selected_node]
end

local function selected_choice()
  local node = selected_node()
  if not node or type(node.choices) ~= "table" then
    return nil
  end
  return node.choices[state.choice_index]
end

local function unique_node_id(base)
  local id = base
  local index = 1
  while state.graph.nodes[id] do
    index = index + 1
    id = base .. "_" .. index
  end
  return id
end

local function save_graph(ctx)
  local file = current_file()
  if not file or not state.graph then
    return
  end
  state.error = nil
  if trim(state.graph.start) == "" then
    state.error = "`start` vazio"
    return
  end
  -- Portões duros antes de escrever: gravava grafos com `start` caído,
  -- nós sem id e `goto` pendurados (o delete de nó não reescrevia os
  -- `goto` que apontavam para ele) — o jogo apanhava o grafo partido.
  local nodes = type(state.graph.nodes) == "table" and state.graph.nodes or {}
  if not nodes[state.graph.start] then
    state.error = "`start` aponta para um nó inexistente: " .. tostring(state.graph.start)
    return
  end
  for id, node in pairs(nodes) do
    if trim(tostring(id)) == "" then
      state.error = "há um nó com id vazio"
      return
    end
    local choices = type(node) == "table" and node.choices or nil
    if choices ~= nil and type(choices) ~= "table" then
      state.error = "nó `" .. tostring(id) .. "`: choices tem de ser uma lista"
      return
    end
    for _, choice in ipairs(choices or {}) do
      local goto = trim(tostring(type(choice) == "table" and choice.goto or ""))
      if goto ~= "" and not nodes[goto] then
        state.error = "nó `" .. tostring(id) .. "`: goto inexistente: " .. goto
        return
      end
    end
  end
  if state.original_node_id and state.selected_node ~= state.original_node_id then
    local old = state.original_node_id
    local new = state.selected_node
    if state.graph.nodes[new] then
      state.error = "já existe um nó `" .. new .. "`"
      return
    end
    state.graph.nodes[new] = state.graph.nodes[old]
    state.graph.nodes[old] = nil
    if state.graph.start == old then
      state.graph.start = new
    end
    for _, node in pairs(state.graph.nodes) do
      for _, choice in ipairs(node.choices or {}) do
        if choice.goto == old then
          choice.goto = new
        end
      end
    end
    state.original_node_id = new
  end
  local out = {}
  for key, value in pairs(state.graph) do
    out[key] = value
  end
  out.nodes = {}
  for id, node in pairs(state.graph.nodes) do
    local clean = copy(node)
    if type(clean.choices) ~= "table" then
      clean.choices = {}
    end
    clean.choices = ctx.editor.fs.json.array(clean.choices)
    out.nodes[id] = clean
  end
  local ok, err = write_json(ctx, file.path, out)
  if not ok then
    state.error = err or "falha a gravar"
    return
  end
  file.graph = state.graph
  ctx.editor.prefs.set("dialogues.file", file.name)
  ctx.editor.toast("success", "Diálogo gravado: " .. file.name)
end

local function validate_graph(ctx, graph, dialogue_id)
  local problems = {}
  local nodes = type(graph.nodes) == "table" and graph.nodes or {}
  dialogue_id = dialogue_id or ""
  if trim(graph.start) == "" then
    problems[#problems + 1] = "`start` vazio"
  elseif not nodes[graph.start] then
    problems[#problems + 1] = "`start` aponta para um nó inexistente: " .. tostring(graph.start)
  end
  for id, node in pairs(nodes) do
    if type(node) ~= "table" then
      problems[#problems + 1] = id .. ": nó não é uma tabela"
    else
      if trim(tostring(id)) == "" then
        problems[#problems + 1] = "nó com id vazio"
      end
      if node.choices ~= nil and type(node.choices) ~= "table" then
        problems[#problems + 1] = id .. ": choices tem de ser uma lista"
      end
      local text = tostring(node.text or "")
      if text == "" then
        problems[#problems + 1] = id .. ": texto vazio"
      elseif text:sub(1, 1) == "@" and state.locale and not state.locale[text:sub(2)] then
        problems[#problems + 1] = id .. ": chave de locale em falta: " .. text
      end
      for index, choice in ipairs(node.choices or {}) do
        local goto = tostring(choice.goto or "")
        if goto ~= "" and not nodes[goto] then
          problems[#problems + 1] = id .. " escolha " .. index .. ": goto inexistente: " .. goto
        end
        if choice.quest_accept and state.quests and not state.quests[dialogue_id] then
          problems[#problems + 1] = id .. ": quest_accept sem quest `" .. dialogue_id .. "`"
        end
        if choice.quest_turn_in and state.quests and not state.quests[dialogue_id] then
          problems[#problems + 1] = id .. ": quest_turn_in sem quest `" .. dialogue_id .. "`"
        end
      end
    end
  end
  -- Nós inalcançáveis a partir do start.
  local seen = {}
  local function walk(id)
    if not id or seen[id] or not nodes[id] then
      return
    end
    seen[id] = true
    for _, choice in ipairs(nodes[id].choices or {}) do
      walk(choice.goto)
    end
  end
  walk(graph.start)
  for id in pairs(nodes) do
    if not seen[id] then
      problems[#problems + 1] = id .. ": nó inalcançável a partir do start"
    end
  end
  return problems
end

local function render_choice_form(children)
  local choice = selected_choice()
  if not choice then
    return
  end
  children[#children + 1] = { type = "heading", text = "Escolha " .. tostring(state.choice_index) }
  children[#children + 1] = { type = "text", id = "choice_text", label = "texto", value = choice.text or "" }
  children[#children + 1] = {
    type = "select",
    id = "choice_goto",
    label = "goto",
    value = choice.goto or "",
    options = node_options(true),
  }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "checkbox", id = "choice_accept", label = "quest_accept", value = choice.quest_accept == true },
      { type = "checkbox", id = "choice_turn_in", label = "quest_turn_in", value = choice.quest_turn_in == true },
      { type = "button", id = "remove_choice", text = "Remover escolha" },
    },
  }
end

local function render(ctx)
  if not state.loaded then
    return {
      type = "column",
      gap = 4,
      children = { { type = "label", text = "A carregar diálogos…", tone = "dim" } },
    }
  end
  local children = {
    { type = "heading", text = "Diálogos" },
    { type = "label", text = "Grafos de dialogues/*.json (lidos por scripts/townsfolk.lua).", tone = "dim" },
  }
  if state.error then
    children[#children + 1] = { type = "label", text = state.error, tone = "error" }
  end
  if not current_file() then
    children[#children + 1] = { type = "label", text = "Sem grafos em dialogues/.", tone = "warn" }
    return { type = "column", gap = 6, children = children }
  end

  local files = {}
  for _, file in ipairs(state.files) do
    files[#files + 1] = { value = file.name, label = file.name }
  end
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "select", id = "file", label = "ficheiro", value = current_file().name, options = files },
      { type = "button", id = "new_node", text = "Novo nó" },
      { type = "button", id = "save", text = "Gravar", primary = true },
      { type = "button", id = "reload", text = "Recarregar" },
    },
  }
  children[#children + 1] = {
    type = "select",
    id = "start",
    label = "start",
    value = state.graph.start or "",
    options = node_options(false),
  }

  local nodes = {}
  for _, id in ipairs(sorted_nodes()) do
    local node = state.graph.nodes[id]
    nodes[#nodes + 1] = {
      id = id,
      label = id .. "  ·  " .. tostring(node.text or ""):sub(1, 48),
    }
  end
  children[#children + 1] = {
    type = "list",
    id = "nodes",
    selected = state.selected_node,
    items = nodes,
    height = 140,
  }

  local node = selected_node()
  if node then
    children[#children + 1] = { type = "separator" }
    children[#children + 1] = { type = "heading", text = "Nó" }
    children[#children + 1] = { type = "text", id = "node_id", label = "id", value = state.selected_node }
    children[#children + 1] = { type = "text", id = "speaker", label = "speaker", value = node.speaker or "" }
    children[#children + 1] = { type = "text", id = "text", label = "texto", value = node.text or "" }
    children[#children + 1] = {
      type = "row",
      gap = 6,
      children = {
        { type = "button", id = "add_choice", text = "Adicionar escolha" },
        {
          type = "button",
          id = "delete_node",
          text = state.pending_delete == state.selected_node and "Confirmar apagar" or "Apagar nó",
        },
      },
    }

    local choices = {}
    for index, choice in ipairs(node.choices or {}) do
      choices[#choices + 1] = {
        id = tostring(index),
        label = tostring(index) .. ". " .. tostring(choice.text or "") ..
          "  →  " .. (trim(choice.goto) ~= "" and tostring(choice.goto) or "(fim)"),
      }
    end
    children[#children + 1] = {
      type = "list",
      id = "choices",
      selected = state.choice_index and tostring(state.choice_index) or nil,
      items = choices,
      height = 100,
    }
    render_choice_form(children)

    for _, problem in ipairs(validate_graph(ctx, state.graph, current_file().name:gsub("%.json$", ""))) do
      children[#children + 1] = { type = "label", text = problem, tone = "warn" }
    end
  else
    children[#children + 1] = {
      type = "label",
      text = "Seleciona um nó para editar.",
      tone = "dim",
    }
  end
  return { type = "column", gap = 6, children = children }
end

local function on_event(ctx, event)
  if event.panel ~= "dialogues" then
    return
  end
  if event.kind == "change" and event.id == "file" then
    for index, file in ipairs(state.files) do
      if file.name == event.value then
        state.file_index = index
        break
      end
    end
    state.graph = current_file() and current_file().graph or nil
    state.selected_node = nil
    state.original_node_id = nil
    state.choice_index = nil
    state.pending_delete = nil
    state.error = nil
    ctx.editor.prefs.set("dialogues.file", event.value)
    return
  end
  if event.kind == "click" and event.id == "reload" then
    load_all(ctx)
    return
  end
  if event.kind == "click" and event.id == "save" then
    save_graph(ctx)
    return
  end
  if not state.graph then
    return
  end
  if event.kind == "change" and event.id == "start" then
    state.graph.start = event.value
    return
  end
  if event.kind == "select" and event.id == "nodes" then
    state.selected_node = event.value
    state.original_node_id = event.value
    state.choice_index = nil
    state.pending_delete = nil
    return
  end
  if event.kind == "select" and event.id == "choices" then
    state.choice_index = tonumber(event.value) or 1
    return
  end

  local node = selected_node()
  if event.kind == "change" and node then
    if event.id == "node_id" then
      state.selected_node = event.value
    elseif event.id == "speaker" then
      node.speaker = event.value
    elseif event.id == "text" then
      node.text = event.value
    elseif event.id == "choice_text" then
      local choice = selected_choice()
      if choice then choice.text = event.value end
    elseif event.id == "choice_goto" then
      local choice = selected_choice()
      if choice then choice.goto = event.value end
    elseif event.id == "choice_accept" then
      local choice = selected_choice()
      if choice then choice.quest_accept = event.value == "true" or nil end
    elseif event.id == "choice_turn_in" then
      local choice = selected_choice()
      if choice then choice.quest_turn_in = event.value == "true" or nil end
    end
    return
  end

  if event.kind == "click" and event.id == "new_node" then
    local id = unique_node_id("novo_no")
    state.graph.nodes[id] = { speaker = "", text = "", choices = {} }
    state.selected_node = id
    state.original_node_id = id
    state.choice_index = nil
    return
  end
  if event.kind == "click" and event.id == "add_choice" and node then
    node.choices = node.choices or {}
    node.choices[#node.choices + 1] = { text = "", goto = "" }
    state.choice_index = #node.choices
    return
  end
  if event.kind == "click" and event.id == "remove_choice" and node then
    if state.choice_index then
      table.remove(node.choices or {}, state.choice_index)
      state.choice_index = nil
    end
    return
  end
  if event.kind == "click" and event.id == "delete_node" and state.selected_node then
    if state.pending_delete ~= state.selected_node then
      state.pending_delete = state.selected_node
      return
    end
    state.graph.nodes[state.selected_node] = nil
    state.selected_node = nil
    state.original_node_id = nil
    state.choice_index = nil
    state.pending_delete = nil
    return
  end
end

local function lint(ctx)
  if not state.loaded then
    load_all(ctx)
  end
  local findings = {}
  for _, file in ipairs(state.files) do
    for _, problem in ipairs(validate_graph(ctx, file.graph, file.name:gsub("%.json$", ""))) do
      findings[#findings + 1] = {
        severity = "warning",
        kind = "dialogue-ref",
        message = file.name .. ": " .. problem,
      }
    end
  end
  return findings
end

function on_update(ctx)
  if not state.loaded then
    load_all(ctx)
  end
end

return {
  api = 1,
  id = "dialogues",
  title = "Diálogos",
  on_update = on_update,
  on_event = on_event,
  panels = {
    { id = "dialogues", title = "Diálogos", render = render },
  },
  lint = lint,
}
