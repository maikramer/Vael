-- Plugin do Vael: editor de progressão (`data/progression.json`).
--
-- Curva de XP, XP por abate e talentos (ids, requisitos e efeitos). O jogo
-- lê isto em `scripts/lib/rules.lua`; o diário em `scripts/ui/journal.lua`.

local state = {
  loaded = false,
  data = nil,
  selected = nil,
  error = nil,
  pending_delete = nil,
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

local find_skill

local function load_all(ctx)
  local data, err = read_json(ctx, "data/progression.json")
  if type(data) ~= "table" or type(data.skills) ~= "table" then
    state.error = err or "data/progression.json sem `skills`"
    state.loaded = true
    return
  end
  state.data = data
  state.data.xp = state.data.xp or {}
  state.data.kill_xp = state.data.kill_xp or {}
  state.error = nil
  state.selected = ctx.editor.prefs.get("progression.skill")
  if state.selected and not find_skill(state.selected) then
    state.selected = nil
  end
  state.loaded = true
end

find_skill = function(id)
  for _, skill in ipairs(state.data and state.data.skills or {}) do
    if skill.id == id then
      return skill
    end
  end
  return nil
end

local function unique_skill_id()
  local base = "novo_talento"
  local id = base
  local index = 1
  while find_skill(id) do
    index = index + 1
    id = base .. "_" .. index
  end
  return id
end

local function skill_options(include_empty)
  local out = {}
  if include_empty then
    out[#out + 1] = { value = "", label = "(sem requisito)" }
  end
  for _, skill in ipairs(state.data and state.data.skills or {}) do
    out[#out + 1] = { value = skill.id, label = skill.label or skill.id }
  end
  return out
end

local function kill_xp_lines()
  local out = {}
  for kind, xp in pairs(state.data and state.data.kill_xp or {}) do
    if kind ~= "default" then
      out[#out + 1] = tostring(kind) .. ":" .. tostring(xp)
    end
  end
  table.sort(out)
  return out
end

local function save_data(ctx)
  if not state.data then
    return
  end
  state.error = nil
  local xp = state.data.xp or {}
  if (tonumber(xp.base) or 0) < 1 then
    state.error = "xp.base tem de ser ≥ 1"
    return
  end
  if (tonumber(xp.growth) or 0) <= 1 then
    state.error = "xp.growth tem de ser > 1"
    return
  end
  local seen = {}
  for _, skill in ipairs(state.data.skills) do
    local id = tostring(skill.id or "")
    if id == "" or seen[id] then
      state.error = "talento com id vazio ou duplicado: " .. id
      return
    end
    seen[id] = true
    if tostring(skill.label or "") == "" then
      state.error = "talento `" .. id .. "` sem label"
      return
    end
    if skill.requires and skill.requires ~= "" and not seen[skill.requires] and not find_skill(skill.requires) then
      state.error = "talento `" .. id .. "` requer `" .. skill.requires .. "` inexistente"
      return
    end
  end
  local data = copy(state.data)
  data.skills = ctx.editor.fs.json.array(data.skills)
  local ok, err = write_json(ctx, "data/progression.json", data)
  if not ok then
    state.error = err or "falha a gravar"
    return
  end
  ctx.editor.toast("success", "Progressão gravada.")
end

local function effect(skill, key)
  skill.effects = type(skill.effects) == "table" and skill.effects or {}
  return tonumber(skill.effects[key]) or 0
end

local function render(ctx)
  if not state.loaded then
    return { type = "column", gap = 4, children = { { type = "label", text = "A carregar progressão…", tone = "dim" } } }
  end
  local children = {
    { type = "heading", text = "Progressão" },
    { type = "label", text = "data/progression.json — XP, abates e talentos.", tone = "dim" },
  }
  if state.error then
    children[#children + 1] = { type = "label", text = state.error, tone = "error" }
  end
  if not state.data then
    return { type = "column", gap = 6, children = children }
  end

  local xp = state.data.xp or {}
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "button", id = "save", text = "Gravar", primary = true },
      { type = "button", id = "reload", text = "Recarregar" },
      { type = "button", id = "new_skill", text = "Novo talento" },
    },
  }
  children[#children + 1] = { type = "heading", text = "XP" }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "number", id = "xp_base", label = "base", value = tonumber(xp.base) or 100, step = 1, min = 1 },
      { type = "number", id = "xp_growth", label = "growth", value = tonumber(xp.growth) or 1.5, step = 0.05, min = 1.01 },
      { type = "number", id = "xp_points", label = "pontos/nível", value = tonumber(xp.skill_points_per_level) or 1, step = 1, min = 0 },
      { type = "number", id = "talent_every", label = "sementes/talento", value = tonumber(state.data.song_seed_talent_every) or 5, step = 1, min = 0 },
    },
  }
  children[#children + 1] = {
    type = "text_lines",
    id = "kill_xp",
    label = "XP por abate (criatura:xp)",
    values = kill_xp_lines(),
  }

  local rows = {}
  for _, skill in ipairs(state.data.skills) do
    rows[#rows + 1] = {
      id = skill.id,
      label = tostring(skill.id) .. "  ·  " .. tostring(skill.label or ""),
    }
  end
  children[#children + 1] = { type = "heading", text = "Talentos" }
  children[#children + 1] = {
    type = "list",
    id = "skills",
    selected = state.selected,
    items = rows,
    height = 140,
  }

  local skill = state.selected and find_skill(state.selected) or nil
  if skill then
    children[#children + 1] = { type = "separator" }
    children[#children + 1] = { type = "label", text = "id: " .. tostring(skill.id), tone = "dim" }
    children[#children + 1] = { type = "text", id = "skill_label", label = "label", value = skill.label or "" }
    children[#children + 1] = {
      type = "select",
      id = "skill_requires",
      label = "requires",
      value = skill.requires or "",
      options = skill_options(true),
    }
    children[#children + 1] = {
      type = "row",
      gap = 6,
      children = {
        { type = "number", id = "eff_hp", label = "max_hp", value = effect(skill, "max_hp"), step = 1, min = 0 },
        { type = "number", id = "eff_damage", label = "damage", value = effect(skill, "damage"), step = 1, min = 0 },
        { type = "number", id = "eff_speed", label = "speed", value = effect(skill, "speed"), step = 0.01, min = 0 },
        { type = "number", id = "eff_crit", label = "crit", value = effect(skill, "crit"), step = 0.01, min = 0 },
      },
    }
    children[#children + 1] = {
      type = "button",
      id = "delete_skill",
      text = state.pending_delete == state.selected and "Confirmar apagar" or "Apagar talento",
    }
  else
    children[#children + 1] = { type = "label", text = "Seleciona um talento.", tone = "dim" }
  end
  return { type = "column", gap = 6, children = children }
end

local function set_effect(skill, key, value)
  skill.effects = type(skill.effects) == "table" and skill.effects or {}
  local number = tonumber(value) or 0
  if number == 0 then
    skill.effects[key] = nil
  else
    skill.effects[key] = number
  end
end

local function on_event(ctx, event)
  if event.panel ~= "progression" then
    return
  end
  if event.kind == "select" and event.id == "skills" then
    state.selected = event.value
    state.pending_delete = nil
    ctx.editor.prefs.set("progression.skill", event.value)
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
  if event.kind == "click" and event.id == "new_skill" then
    local id = unique_skill_id()
    state.data.skills[#state.data.skills + 1] = {
      id = id,
      label = "Novo talento",
      requires = "",
      effects = {},
    }
    state.selected = id
    state.pending_delete = nil
    return
  end

  if event.kind == "add" and event.id == "kill_xp" then
    local key = "nova_criatura"
    local index = 1
    while state.data.kill_xp[key] do
      index = index + 1
      key = "nova_criatura_" .. index
    end
    state.data.kill_xp[key] = 1
    return
  end
  if event.kind == "remove" and event.id == "kill_xp" then
    local lines = kill_xp_lines()
    local line = lines[(event.index or 0) + 1]
    local kind = line and line:match("^([^:]+):")
    if kind then
      state.data.kill_xp[kind] = nil
    end
    return
  end

  if event.kind == "change" then
    if event.id == "xp_base" then
      state.data.xp.base = tonumber(event.value) or 100
      return
    elseif event.id == "xp_growth" then
      state.data.xp.growth = tonumber(event.value) or 1.5
      return
    elseif event.id == "xp_points" then
      state.data.xp.skill_points_per_level = tonumber(event.value) or 1
      return
    elseif event.id == "talent_every" then
      state.data.song_seed_talent_every = tonumber(event.value) or 5
      return
    elseif event.id == "kill_xp" then
      local line = tostring(event.value or "")
      local kind, xp = line:match("^([^:]+):(%d+)$")
      if kind then
        state.data.kill_xp[kind] = tonumber(xp)
      end
      return
    end
  end

  local skill = state.selected and find_skill(state.selected) or nil
  if not skill then
    return
  end
  if event.kind == "click" and event.id == "delete_skill" then
    if state.pending_delete ~= state.selected then
      state.pending_delete = state.selected
      return
    end
    for index, entry in ipairs(state.data.skills) do
      if entry.id == state.selected then
        table.remove(state.data.skills, index)
        break
      end
    end
    state.selected = nil
    state.pending_delete = nil
    return
  end
  if event.kind == "change" then
    if event.id == "skill_label" then
      skill.label = event.value
    elseif event.id == "skill_requires" then
      skill.requires = event.value
    elseif event.id == "eff_hp" then
      set_effect(skill, "max_hp", event.value)
    elseif event.id == "eff_damage" then
      set_effect(skill, "damage", event.value)
    elseif event.id == "eff_speed" then
      set_effect(skill, "speed", event.value)
    elseif event.id == "eff_crit" then
      set_effect(skill, "crit", event.value)
    end
  end
end

local function lint(ctx)
  if not state.loaded then
    load_all(ctx)
  end
  local findings = {}
  if not state.data then
    return { { severity = "error", kind = "progression", message = state.error or "data/progression.json inválido" } }
  end
  local xp = state.data.xp or {}
  if (tonumber(xp.base) or 0) < 1 then
    findings[#findings + 1] = { severity = "error", kind = "progression", message = "xp.base < 1" }
  end
  if (tonumber(xp.growth) or 0) <= 1 then
    findings[#findings + 1] = { severity = "error", kind = "progression", message = "xp.growth ≤ 1 (não sobe)" }
  end
  local seen = {}
  for _, skill in ipairs(state.data.skills or {}) do
    local id = tostring(skill.id or "")
    if id == "" or seen[id] then
      findings[#findings + 1] = { severity = "error", kind = "progression", message = "talento vazio/duplicado: " .. id }
    end
    seen[id] = true
    if skill.requires and skill.requires ~= "" and not find_skill(skill.requires) then
      findings[#findings + 1] = { severity = "warning", kind = "progression", message = id .. ": requires inexistente: " .. tostring(skill.requires) }
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
  id = "progression",
  title = "Progressão",
  on_update = on_update,
  on_event = on_event,
  panels = {
    { id = "progression", title = "Progressão", render = render },
  },
  lint = lint,
}
