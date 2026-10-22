-- Plugin do Vael: editor de marcos da Nota (`data/landmarks.json`).
--
-- Os nomes são entidades do mundo; o plugin valida-os com `find_nodes` e as
-- quests de survey contra `quests/*.json`.

local BIOMES = { "tower", "dark_forest", "desert", "swamp", "frozen_peaks", "alto" }

local state = {
  loaded = false,
  data = nil,
  selected = nil,
  error = nil,
  pending_delete = nil,
  quests = nil,
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

local function read_json(ctx, path)
  local text, err = ctx.editor.fs.read(path)
  if not text then
    return nil, err
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

local function load_all(ctx)
  local data, err = read_json(ctx, "data/landmarks.json")
  if type(data) ~= "table" or type(data.landmarks) ~= "table" then
    state.error = err or "data/landmarks.json sem `landmarks`"
    state.loaded = true
    return
  end
  state.data = data
  state.error = nil
  state.selected = ctx.editor.prefs.get("landmarks.name")
  if state.selected and not state:find(state.selected) then
    state.selected = nil
  end
  load_quests(ctx)
  state.loaded = true
end

function state:find(name)
  for _, mark in ipairs(self.data and self.data.landmarks or {}) do
    if mark.name == name then
      return mark
    end
  end
  return nil
end

local function find_mark(name)
  return state:find(name)
end

local function unique_name()
  local base = "novo-marco"
  local name = base
  local index = 1
  while find_mark(name) do
    index = index + 1
    name = base .. "-" .. index
  end
  return name
end

local function save_data(ctx)
  if not state.data then
    return
  end
  state.error = nil
  local seen = {}
  for _, mark in ipairs(state.data.landmarks) do
    local name = tostring(mark.name or "")
    if name == "" then
      state.error = "há um marco sem nome"
      return
    end
    if seen[name] then
      state.error = "marco duplicado: " .. name
      return
    end
    seen[name] = true
    if tostring(mark.label or "") == "" then
      state.error = "marco `" .. name .. "` sem label"
      return
    end
    if (tonumber(mark.radius) or 0) <= 0 then
      state.error = "marco `" .. name .. "`: radius tem de ser > 0"
      return
    end
  end
  local data = copy(state.data)
  data.landmarks = ctx.editor.fs.json.array(data.landmarks)
  local ok, err = write_json(ctx, "data/landmarks.json", data)
  if not ok then
    state.error = err or "falha a gravar"
    return
  end
  ctx.editor.toast("success", "Marcos gravados.")
end

local function biome_options()
  local out = {}
  for _, biome in ipairs(BIOMES) do
    out[#out + 1] = { value = biome, label = biome }
  end
  return out
end

local function render(ctx)
  if not state.loaded then
    return { type = "column", gap = 4, children = { { type = "label", text = "A carregar marcos…", tone = "dim" } } }
  end
  local children = {
    { type = "heading", text = "Marcos da Nota" },
    { type = "label", text = "data/landmarks.json — nomes de entidades, bioma, raio e quest de survey.", tone = "dim" },
  }
  if state.error then
    children[#children + 1] = { type = "label", text = state.error, tone = "error" }
  end
  if not state.data then
    return { type = "column", gap = 6, children = children }
  end

  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "button", id = "new", text = "Novo marco" },
      { type = "button", id = "save", text = "Gravar", primary = true },
      { type = "button", id = "reload", text = "Recarregar" },
    },
  }
  local rows = {}
  for _, mark in ipairs(state.data.landmarks) do
    rows[#rows + 1] = {
      id = mark.name,
      label = tostring(mark.name) .. "  ·  " .. tostring(mark.label or "") ..
        "  ·  " .. tostring(mark.biome or ""),
    }
  end
  children[#children + 1] = {
    type = "list",
    id = "landmarks",
    selected = state.selected,
    items = rows,
    height = 190,
  }

  local mark = state.selected and find_mark(state.selected) or nil
  if mark then
    children[#children + 1] = { type = "separator" }
    children[#children + 1] = { type = "heading", text = "Marco" }
    children[#children + 1] = { type = "text", id = "name", label = "name (entidade)", value = mark.name or "" }
    children[#children + 1] = { type = "text", id = "label", label = "label", value = mark.label or "" }
    children[#children + 1] = {
      type = "select",
      id = "biome",
      label = "biome",
      value = mark.biome or "tower",
      options = biome_options(),
    }
    children[#children + 1] = {
      type = "row",
      gap = 6,
      children = {
        { type = "number", id = "radius", label = "radius", value = tonumber(mark.radius) or 10, step = 1, min = 1 },
        { type = "checkbox", id = "pilgrim", label = "pilgrim", value = mark.pilgrim == true },
      },
    }
    children[#children + 1] = { type = "text", id = "survey", label = "survey (quest)", value = mark.survey or "" }
    children[#children + 1] = {
      type = "button",
      id = "delete",
      text = state.pending_delete == state.selected and "Confirmar apagar" or "Apagar marco",
    }
  else
    children[#children + 1] = { type = "label", text = "Seleciona um marco.", tone = "dim" }
  end
  return { type = "column", gap = 6, children = children }
end

local function on_event(ctx, event)
  if event.panel ~= "landmarks" then
    return
  end
  if event.kind == "select" and event.id == "landmarks" then
    state.selected = event.value
    state.pending_delete = nil
    ctx.editor.prefs.set("landmarks.name", event.value)
    return
  end
  if event.kind == "click" and event.id == "reload" then
    load_all(ctx)
    return
  end
  if event.kind == "click" and event.id == "save" then
    save_data(ctx)
    return
  end
  if event.kind == "click" and event.id == "new" then
    local name = unique_name()
    state.data.landmarks[#state.data.landmarks + 1] = {
      name = name,
      biome = "tower",
      label = "Novo marco",
      radius = 10,
    }
    state.selected = name
    state.pending_delete = nil
    return
  end
  local mark = state.selected and find_mark(state.selected) or nil
  if not mark then
    return
  end
  if event.kind == "click" and event.id == "delete" then
    if state.pending_delete ~= state.selected then
      state.pending_delete = state.selected
      return
    end
    for index, entry in ipairs(state.data.landmarks) do
      if entry.name == state.selected then
        table.remove(state.data.landmarks, index)
        break
      end
    end
    state.selected = nil
    state.pending_delete = nil
    return
  end
  if event.kind == "change" then
    if event.id == "name" then
      mark.name = event.value
      state.selected = event.value
    elseif event.id == "label" then
      mark.label = event.value
    elseif event.id == "biome" then
      mark.biome = event.value
    elseif event.id == "radius" then
      mark.radius = tonumber(event.value) or 1
    elseif event.id == "survey" then
      if tostring(event.value or "") == "" then
        mark.survey = nil
      else
        mark.survey = event.value
      end
    elseif event.id == "pilgrim" then
      mark.pilgrim = event.value == "true" or nil
    end
  end
end

local function lint(ctx)
  if not state.loaded then
    load_all(ctx)
  end
  local findings = {}
  if not state.data then
    return { { severity = "error", kind = "landmarks", message = state.error or "data/landmarks.json inválido" } }
  end
  local seen = {}
  for _, mark in ipairs(state.data.landmarks or {}) do
    local name = tostring(mark.name or "")
    if name == "" or seen[name] then
      findings[#findings + 1] = { severity = "error", kind = "landmarks", message = "nome vazio ou duplicado: " .. name }
    end
    seen[name] = true
    local found = ctx.editor.find_nodes({ name = name, limit = 1 })
    if #found == 0 then
      findings[#findings + 1] = { severity = "error", kind = "landmarks", message = "entidade não existe no mundo: " .. name }
    end
    local biome_ok = false
    for _, biome in ipairs(BIOMES) do
      if biome == mark.biome then
        biome_ok = true
      end
    end
    if not biome_ok then
      findings[#findings + 1] = { severity = "warning", kind = "landmarks", message = name .. ": bioma desconhecido: " .. tostring(mark.biome) }
    end
    if mark.survey and state.quests and not state.quests[mark.survey] then
      findings[#findings + 1] = { severity = "warning", kind = "landmarks", message = name .. ": quest de survey inexistente: " .. tostring(mark.survey) }
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
  id = "landmarks",
  title = "Marcos",
  on_update = on_update,
  on_event = on_event,
  panels = {
    { id = "landmarks", title = "Marcos", render = render },
  },
  lint = lint,
}
