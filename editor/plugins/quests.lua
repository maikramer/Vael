-- Plugin do Vael: editor de quests (`quests/*.json`).
--
-- Só usa a API genérica do editor (`Viber/editor/PLUGINS.md`): fs, JSON,
-- find_nodes e os widgets declarativos. A validação é a mesma que o lint
-- publica no painel Problems.

local BIOMES = { "city", "dark_forest", "desert", "swamp", "frozen_peaks" }
local OBJECTIVES = { "kill", "collect", "visit", "decision" }
local LINE_FIELDS = { "lines_intro", "lines_progress", "lines_complete" }

local state = {
  loaded = false,
  files = {},
  file_index = 0,
  selected = nil,
  draft = nil,
  search = "",
  error = nil,
  pending_delete = nil,
  show_validation = false,
  landmarks = nil,
  items = nil,
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

local function load_items(ctx)
  local data = read_json(ctx, "data/items.json")
  local out = {}
  for _, item in ipairs(data and data.items or {}) do
    if type(item) == "table" and item.id then
      out[item.id] = true
    end
  end
  state.items = out
end

local function load_landmarks(ctx)
  local data = read_json(ctx, "data/landmarks.json")
  local out = {}
  for _, mark in ipairs(data and data.landmarks or {}) do
    if type(mark) == "table" and mark.name then
      out[mark.name] = true
    end
  end
  state.landmarks = out
end

local function load_all(ctx)
  state.files = {}
  state.error = nil
  local entries, err = ctx.editor.fs.list("quests")
  if not entries then
    state.error = err or "não consegui listar quests/"
    state.loaded = true
    return
  end
  for _, entry in ipairs(entries) do
    if not entry.dir and entry.name:match("%.json$") and entry.name ~= "index.json" then
      local data, read_err = read_json(ctx, entry.path)
      if type(data) ~= "table" then
        state.error = (state.error and (state.error .. "\n") or "") ..
          entry.path .. ": " .. tostring(read_err or "JSON sem array de quests")
      else
        state.files[#state.files + 1] = {
          name = entry.name,
          path = entry.path,
          quests = data,
        }
      end
    end
  end
  table.sort(state.files, function(a, b) return a.name < b.name end)
  load_items(ctx)
  load_landmarks(ctx)
  local saved_file = ctx.editor.prefs.get("quests.file")
  state.file_index = 1
  for index, file in ipairs(state.files) do
    if file.name == saved_file then
      state.file_index = index
      break
    end
  end
  local saved_id = ctx.editor.prefs.get("quests.id")
  state.selected = nil
  state.draft = nil
  for _, quest in ipairs(state.files[state.file_index] and state.files[state.file_index].quests or {}) do
    if quest.id == saved_id then
      state.selected = saved_id
      state.draft = copy(quest)
      break
    end
  end
  state.loaded = true
end

local function current_file()
  return state.files[state.file_index]
end

local function all_quests()
  local out = {}
  for _, file in ipairs(state.files) do
    for _, quest in ipairs(file.quests or {}) do
      out[#out + 1] = { file = file, quest = quest }
    end
  end
  return out
end

local function find_quest(id)
  for _, entry in ipairs(all_quests()) do
    if entry.quest.id == id then
      return entry
    end
  end
  return nil
end

local function new_draft()
  local draft = {
    id = "nova_quest",
    npc = "",
    biome = "city",
    title = "",
    portrait = "",
    voice = "",
    lines_intro = {},
    lines_progress = {},
    lines_complete = {},
    objective = { type = "kill", target = "wolf", count = 1 },
    rewards = { gold = 0, xp = 0, items = {} },
  }
  local base = "nova_quest"
  local index = 1
  while find_quest(draft.id) do
    index = index + 1
    draft.id = base .. "_" .. index
  end
  return draft
end

local function objective(field, default)
  if type(state.draft.objective) ~= "table" then
    state.draft.objective = {}
  end
  local value = state.draft.objective[field]
  if value == nil then
    return default
  end
  return value
end

local function rewards(field, default)
  if type(state.draft.rewards) ~= "table" then
    state.draft.rewards = {}
  end
  local value = state.draft.rewards[field]
  if value == nil then
    return default
  end
  return value
end

local function lines(field)
  local value = state.draft[field]
  if type(value) ~= "table" then
    value = {}
    state.draft[field] = value
  end
  return value
end

local function validate_quest(ctx, quest, ignore_id)
  local problems = {}
  local id = trim(quest.id)
  if id == "" or not id:match("^[%w_%-%.]+$") then
    problems[#problems + 1] = "id inválido (usa letras, dígitos, _ ou -)"
  elseif find_quest(id) and id ~= ignore_id then
    problems[#problems + 1] = "id duplicado: " .. id
  end
  if trim(quest.title) == "" then
    problems[#problems + 1] = "title vazio"
  end
  -- Um campo de falas a texto (edição à mão) fazia `fs.json.array` rebentar
  -- no save (erro do Luau no callback, sem mensagem no painel).
  for _, field in ipairs(LINE_FIELDS) do
    if quest[field] ~= nil and type(quest[field]) ~= "table" then
      problems[#problems + 1] = field .. " tem de ser uma lista de falas"
    end
  end
  if type(quest.rewards) == "table" and quest.rewards.items ~= nil
    and type(quest.rewards.items) ~= "table" then
    problems[#problems + 1] = "rewards.items tem de ser uma lista"
  end
  local biome = trim(quest.biome)
  local biome_ok = false
  for _, known in ipairs(BIOMES) do
    if known == biome then
      biome_ok = true
    end
  end
  if not biome_ok then
    problems[#problems + 1] = "bioma desconhecido: " .. biome
  end
  local npc = trim(quest.npc)
  if npc == "" then
    problems[#problems + 1] = "npc vazio"
  elseif npc ~= "notice_board" then
    local found = ctx.editor.find_nodes({ name = npc, limit = 1 })
    if #found == 0 then
      problems[#problems + 1] = "npc não existe no mundo: " .. npc
    end
  end
  local obj = type(quest.objective) == "table" and quest.objective or {}
  local kind = trim(obj.type)
  local kind_ok = false
  for _, known in ipairs(OBJECTIVES) do
    if known == kind then
      kind_ok = true
    end
  end
  if not kind_ok then
    problems[#problems + 1] = "objective.type inválido: " .. kind
  end
  if trim(obj.target) == "" then
    problems[#problems + 1] = "objective.target vazio"
  elseif kind == "visit" then
    for place in tostring(obj.target):gmatch("%S+") do
      local found = ctx.editor.find_nodes({ name = place, limit = 1 })
      if #found == 0 then
        problems[#problems + 1] = "local de visita não existe: " .. place
      elseif state.landmarks and not state.landmarks[place] then
        problems[#problems + 1] = "local fora de data/landmarks.json: " .. place
      end
    end
  elseif kind == "collect" and state.items and not state.items[trim(obj.target)] then
    problems[#problems + 1] = "item de objetivo desconhecido: " .. trim(obj.target)
  end
  local count = tonumber(obj.count) or 0
  if count < 1 then
    problems[#problems + 1] = "objective.count tem de ser ≥ 1"
  end
  -- Bloco `parley`: o fecho exige um relato de uma palavra que o modelo
  -- resolve. Sem ele a quest fecha pelo caminho antigo.
  local parley = type(quest.parley) == "table" and quest.parley or nil
  if not parley then
    if kind ~= "decision" then
      problems[#problems + 1] = "sem parley (relato de 1 palavra)"
    end
  else
    local rep = type(parley.report) == "table" and parley.report or nil
    if not rep then
      problems[#problems + 1] = "parley.report ausente"
    else
      if trim(rep.prompt) == "" then
        problems[#problems + 1] = "parley.report.prompt vazio"
      end
      if trim(rep.example) == "" then
        problems[#problems + 1] = "parley.report.example vazio"
      end
      local expect = type(rep.expect) == "table" and rep.expect or {}
      if #expect == 0 then
        problems[#problems + 1] = "parley.report.expect vazio"
      end
      for _, e in ipairs(expect) do
        local okind, eid = trim(e.kind), trim(e.id)
        if okind == "item" then
          if state.items and not state.items[eid] then
            problems[#problems + 1] = "parley: item desconhecido: " .. eid
          end
        elseif okind == "local" then
          if state.landmarks and not state.landmarks[eid] then
            problems[#problems + 1] = "parley: local fora de landmarks.json: " .. eid
          end
        elseif okind ~= "criatura" then
          problems[#problems + 1] = "parley: kind inválido: " .. okind
        end
        if kind ~= "decision"
          and ((kind == "kill" and okind ~= "criatura")
            or (kind == "collect" and okind ~= "item")
            or (kind == "visit" and okind ~= "local")) then
          problems[#problems + 1] = "parley.expect não corresponde ao objective"
        end
      end
    end
  end
  local loot = type(quest.rewards) == "table" and quest.rewards.items or {}
  for index, entry in ipairs(loot) do
    local item, amount = tostring(entry):match("^([^:]+):?(%d*)$")
    if item == nil or trim(item) == "" then
      problems[#problems + 1] = "rewards.items[" .. index .. "] inválido"
    elseif state.items and not state.items[trim(item)] then
      problems[#problems + 1] = "item de recompensa desconhecido: " .. trim(item)
    elseif amount ~= "" and (tonumber(amount) or 0) < 1 then
      problems[#problems + 1] = "rewards.items[" .. index .. "] com quantidade < 1"
    end
  end
  return problems
end

-- O jogo só carrega o que está em `quests/index.json` (`rules.lua`);
-- README e engine prometem que é o editor quem mantém a lista. Sem isto,
-- um ficheiro novo (ou retirado do índice à mão) era editável aqui mas
-- invisível ao jogo.
local function ensure_index(ctx, file_name)
  local index = read_json(ctx, "quests/index.json")
  if type(index) ~= "table" then
    index = {}
  end
  for _ in pairs(index) do
    if index[1] == nil then
      return false, "quests/index.json não é uma lista; não toquei nele"
    end
    break
  end
  for _, name in ipairs(index) do
    if name == file_name then
      return true
    end
  end
  index[#index + 1] = file_name
  table.sort(index)
  local text, err = ctx.editor.fs.json.encode(ctx.editor.fs.json.array(index), true)
  if not text then
    return false, err
  end
  return ctx.editor.fs.write("quests/index.json", text)
end

local function save_file(ctx, file)
  local quests = {}
  for _, quest in ipairs(file.quests) do
    local out = copy(quest)
    for _, field in ipairs(LINE_FIELDS) do
      if type(out[field]) ~= "table" then
        out[field] = {}
      end
      out[field] = ctx.editor.fs.json.array(out[field])
    end
    if type(out.rewards) == "table" then
      if type(out.rewards.items) ~= "table" then
        out.rewards.items = {}
      end
      out.rewards.items = ctx.editor.fs.json.array(out.rewards.items)
    end
    quests[#quests + 1] = out
  end
  local ok, err = write_json(ctx, file.path, ctx.editor.fs.json.array(quests))
  if not ok then
    return false, err
  end
  local indexed, index_err = ensure_index(ctx, file.name)
  if not indexed then
    -- A quest gravou; sem o índice o jogo não a vê — avisa em vez de calar.
    return true, "gravado, mas " .. tostring(index_err or "falhou o quests/index.json")
  end
  return true
end

local function save_draft(ctx)
  local draft = state.draft
  local file = current_file()
  if not draft or not file then
    return
  end
  local problems = validate_quest(ctx, draft, state.selected)
  if #problems > 0 then
    state.error = table.concat(problems, "\n")
    return
  end
  local replaced = false
  for index, quest in ipairs(file.quests) do
    if quest.id == state.selected then
      file.quests[index] = copy(draft)
      replaced = true
      break
    end
  end
  if not replaced then
    file.quests[#file.quests + 1] = copy(draft)
  end
  local ok, err = save_file(ctx, file)
  if not ok then
    state.error = err or "falha a gravar"
    return
  end
  state.selected = draft.id
  state.error = nil
  ctx.editor.prefs.set("quests.file", file.name)
  ctx.editor.prefs.set("quests.id", draft.id)
  if err then
    ctx.editor.toast("warn", err)
  else
    ctx.editor.toast("success", "Quest gravada: " .. draft.id)
  end
end

local function delete_draft(ctx)
  local file = current_file()
  if not file or not state.selected then
    return
  end
  if state.pending_delete ~= state.selected then
    state.pending_delete = state.selected
    return
  end
  local index = nil
  for i, quest in ipairs(file.quests) do
    if quest.id == state.selected then
      index = i
      break
    end
  end
  local removed = index and table.remove(file.quests, index) or nil
  local ok, err = save_file(ctx, file)
  if not ok then
    -- A gravação falhou: repor a quest em memória. Sem isto a lista já a
    -- tinha perdido e a PRÓXIMA gravação do ficheiro apagava-a do disco.
    if removed then
      table.insert(file.quests, index, removed)
    end
    state.error = err or "falha a gravar"
    state.pending_delete = nil
    return
  end
  state.draft = nil
  state.selected = nil
  state.pending_delete = nil
  state.error = err
  ctx.editor.toast("success", "Quest apagada.")
end

local function filtered_quests()
  local file = current_file()
  local needle = state.search:lower()
  local out = {}
  for _, quest in ipairs(file and file.quests or {}) do
    local haystack = (tostring(quest.id or "") .. " " .. tostring(quest.title or "")):lower()
    if needle == "" or haystack:find(needle, 1, true) then
      out[#out + 1] = quest
    end
  end
  return out
end

local function file_options()
  local out = {}
  for _, file in ipairs(state.files) do
    out[#out + 1] = { value = file.name, label = file.name }
  end
  return out
end

local function selected_file_name()
  local file = current_file()
  return file and file.name or ""
end

local function biome_options()
  local out = {}
  for _, biome in ipairs(BIOMES) do
    out[#out + 1] = { value = biome, label = biome }
  end
  return out
end

local function objective_options()
  local out = {}
  for _, kind in ipairs(OBJECTIVES) do
    out[#out + 1] = { value = kind, label = kind }
  end
  return out
end

local function selected_node_name(ctx)
  if not ctx.selection then
    return nil
  end
  local node = ctx.editor.node(ctx.selection)
  return node and node.name or nil
end

local function render_form(ctx, children)
  local draft = state.draft
  local obj = type(draft.objective) == "table" and draft.objective or {}
  local rew = type(draft.rewards) == "table" and draft.rewards or {}
  children[#children + 1] = { type = "separator" }
  children[#children + 1] = { type = "heading", text = "Quest" }
  children[#children + 1] = { type = "text", id = "qid", label = "id", value = draft.id or "" }
  children[#children + 1] = { type = "text", id = "title", label = "title", value = draft.title or "" }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "text", id = "npc", label = "npc", value = draft.npc or "" },
      { type = "button", id = "use_npc", text = "Usar seleção" },
    },
  }
  children[#children + 1] = {
    type = "select",
    id = "biome",
    label = "biome",
    value = draft.biome or "city",
    options = biome_options(),
  }
  children[#children + 1] = { type = "heading", text = "Objetivo" }
  children[#children + 1] = {
    type = "select",
    id = "objective_type",
    label = "type",
    value = obj.type or "kill",
    options = objective_options(),
  }
  children[#children + 1] = {
    type = "text",
    id = "objective_target",
    label = "target",
    value = obj.target or "",
    placeholder = "kill/collect: id; visit: nomes; decision: etiqueta do desfecho",
  }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "number", id = "objective_count", label = "count", value = tonumber(obj.count) or 1, step = 1, min = 1 },
      { type = "number", id = "objective_radius", label = "radius", value = tonumber(obj.radius) or 0, step = 1, min = 0 },
    },
  }
  children[#children + 1] = { type = "heading", text = "Recompensas" }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "number", id = "reward_gold", label = "gold", value = tonumber(rew.gold) or 0, step = 1, min = 0 },
      { type = "number", id = "reward_xp", label = "xp", value = tonumber(rew.xp) or 0, step = 1, min = 0 },
    },
  }
  children[#children + 1] = { type = "text_lines", id = "reward_items", label = "items (id:count)", values = rew.items or {} }
  for _, field in ipairs(LINE_FIELDS) do
    children[#children + 1] = {
      type = "text_lines",
      id = field,
      label = field,
      values = draft[field] or {},
    }
  end
  children[#children + 1] = { type = "heading", text = "Apresentação" }
  children[#children + 1] = { type = "text", id = "portrait", label = "portrait", value = draft.portrait or "" }
  children[#children + 1] = { type = "text", id = "voice", label = "voice", value = draft.voice or "" }
end

local function render(ctx)
  if not state.loaded then
    return {
      type = "column",
      gap = 4,
      children = { { type = "label", text = "A carregar quests…", tone = "dim" } },
    }
  end

  local children = {
    { type = "heading", text = "Missões" },
    { type = "label", text = "Edita quests/*.json; o lint valida NPCs, locais e itens.", tone = "dim" },
  }
  if state.error then
    children[#children + 1] = { type = "label", text = state.error, tone = "error" }
  end
  if not current_file() then
    children[#children + 1] = { type = "label", text = "Sem ficheiros de quests.", tone = "warn" }
    return { type = "column", gap = 6, children = children }
  end

  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      {
        type = "select",
        id = "file",
        label = "ficheiro",
        value = selected_file_name(),
        options = file_options(),
      },
      { type = "button", id = "new", text = "Nova" },
      { type = "button", id = "save", text = "Gravar", primary = true },
      { type = "button", id = "reload", text = "Recarregar" },
    },
  }
  children[#children + 1] = {
    type = "text",
    id = "search",
    label = "filtrar",
    value = state.search,
  }

  local items = {}
  local used = {}
  for index, quest in ipairs(filtered_quests()) do
    -- Quests sem id (ou repetidas) não podem rebentar o painel inteiro
    -- (`ui: falta id` / "id interativo duplicado" virava um card de erro):
    -- ids sintéticos únicos deixam ver e selecionar as restantes.
    local id = tostring(quest.id or "")
    if id == "" then
      id = "sem_id"
    end
    local base, n = id, index
    while used[id] do
      id = base .. "_" .. n
      n = n + 1
    end
    used[id] = true
    items[#items + 1] = {
      id = id,
      label = (quest.title or "") .. "  ·  " .. tostring(quest.id or "(sem id)"),
    }
  end
  children[#children + 1] = {
    type = "list",
    id = "quests",
    selected = state.selected,
    items = items,
    height = 150,
  }

  if state.draft then
    render_form(ctx, children)
    local problems = validate_quest(ctx, state.draft, state.selected)
    if #problems > 0 then
      children[#children + 1] = { type = "heading", text = "Validação" }
      for _, problem in ipairs(problems) do
        children[#children + 1] = { type = "label", text = problem, tone = "warn" }
      end
    end
    local delete_label = state.pending_delete == state.selected and "Confirmar apagar" or "Apagar"
    children[#children + 1] = {
      type = "row",
      gap = 6,
      children = { { type = "button", id = "delete", text = delete_label } },
    }
  else
    children[#children + 1] = {
      type = "label",
      text = "Seleciona uma quest ou cria uma nova.",
      tone = "dim",
    }
  end
  return { type = "column", gap = 6, children = children }
end

local function on_event(ctx, event)
  if event.panel ~= "quests" then
    return
  end
  if event.kind == "change" and event.id == "file" then
    for index, file in ipairs(state.files) do
      if file.name == event.value then
        state.file_index = index
        break
      end
    end
    state.selected = nil
    state.draft = nil
    state.error = nil
    ctx.editor.prefs.set("quests.file", event.value)
    return
  end
  if event.kind == "select" and event.id == "quests" then
    for _, file in ipairs(state.files) do
      for _, quest in ipairs(file.quests) do
        if quest.id == event.value then
          state.selected = quest.id
          state.draft = copy(quest)
          state.error = nil
          state.pending_delete = nil
          ctx.editor.prefs.set("quests.id", quest.id)
          return
        end
      end
    end
    return
  end
  if event.kind == "change" and event.id == "search" then
    state.search = event.value or ""
    return
  end

  if event.kind == "click" and event.id == "reload" then
    load_all(ctx)
    return
  end
  if event.kind == "click" and event.id == "new" then
    state.draft = new_draft()
    state.selected = nil
    state.error = nil
    state.pending_delete = nil
    return
  end
  if event.kind == "click" and event.id == "save" then
    save_draft(ctx)
    return
  end
  if event.kind == "click" and event.id == "delete" then
    delete_draft(ctx)
    return
  end
  if event.kind == "click" and event.id == "use_npc" then
    local name = selected_node_name(ctx)
    if name and state.draft then
      state.draft.npc = name
    else
      ctx.editor.toast("warn", "Seleciona um nó com `name` no editor.")
    end
    return
  end

  if not state.draft then
    return
  end
  local draft = state.draft
  if event.kind == "change" then
    if event.id == "qid" then
      draft.id = event.value
    elseif event.id == "title" then
      draft.title = event.value
    elseif event.id == "npc" then
      draft.npc = event.value
    elseif event.id == "biome" then
      draft.biome = event.value
    elseif event.id == "portrait" then
      draft.portrait = event.value
    elseif event.id == "voice" then
      draft.voice = event.value
    elseif event.id == "objective_type" then
      if type(draft.objective) ~= "table" then draft.objective = {} end
      draft.objective.type = event.value
    elseif event.id == "objective_target" then
      if type(draft.objective) ~= "table" then draft.objective = {} end
      draft.objective.target = event.value
    elseif event.id == "objective_count" then
      if type(draft.objective) ~= "table" then draft.objective = {} end
      draft.objective.count = tonumber(event.value) or 1
    elseif event.id == "objective_radius" then
      if type(draft.objective) ~= "table" then draft.objective = {} end
      draft.objective.radius = tonumber(event.value) or 0
    elseif event.id == "reward_gold" then
      if type(draft.rewards) ~= "table" then draft.rewards = {} end
      draft.rewards.gold = tonumber(event.value) or 0
    elseif event.id == "reward_xp" then
      if type(draft.rewards) ~= "table" then draft.rewards = {} end
      draft.rewards.xp = tonumber(event.value) or 0
    elseif event.id == "reward_items" then
      if type(draft.rewards) ~= "table" then draft.rewards = {} end
      draft.rewards.items = draft.rewards.items or {}
      draft.rewards.items[(event.index or 0) + 1] = event.value or ""
    elseif event.id == "lines_intro" or event.id == "lines_progress" or event.id == "lines_complete" then
      local values = lines(event.id)
      values[(event.index or 0) + 1] = event.value or ""
    end
  elseif event.kind == "add" then
    if event.id == "reward_items" then
      if type(draft.rewards) ~= "table" then draft.rewards = {} end
      draft.rewards.items = draft.rewards.items or {}
      draft.rewards.items[#draft.rewards.items + 1] = ""
    elseif event.id == "lines_intro" or event.id == "lines_progress" or event.id == "lines_complete" then
      local values = lines(event.id)
      values[#values + 1] = ""
    end
  elseif event.kind == "remove" then
    local index = (event.index or 0) + 1
    if event.id == "reward_items" then
      if type(draft.rewards) == "table" and draft.rewards.items then
        table.remove(draft.rewards.items, index)
      end
    elseif event.id == "lines_intro" or event.id == "lines_progress" or event.id == "lines_complete" then
      local values = lines(event.id)
      table.remove(values, index)
    end
  end
end

local function on_command(ctx, id)
  if not state.loaded then
    load_all(ctx)
  end
  if id == "new" then
    state.draft = new_draft()
    state.selected = nil
    state.error = nil
    state.pending_delete = nil
    ctx.editor.open_panel("quests")
  elseif id == "validate" then
    state.show_validation = true
    ctx.editor.open_panel("quests")
  end
end

local function lint(ctx)
  if not state.loaded then
    load_all(ctx)
  end
  local findings = {}
  for _, entry in ipairs(all_quests()) do
    for _, problem in ipairs(validate_quest(ctx, entry.quest, entry.quest.id)) do
      findings[#findings + 1] = {
        severity = "warning",
        kind = "quest-ref",
        message = entry.file.name .. ": " .. tostring(entry.quest.id) .. ": " .. problem,
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
  id = "quests",
  title = "Missões",
  on_update = on_update,
  on_event = on_event,
  on_command = on_command,
  panels = {
    { id = "quests", title = "Missões", render = render },
  },
  commands = {
    { id = "new", title = "Quests: nova", keywords = "missão criar quest" },
    { id = "validate", title = "Quests: validar", keywords = "missão lint" },
  },
  lint = lint,
}
