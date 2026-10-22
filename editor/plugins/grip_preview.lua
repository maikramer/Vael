-- Plugin do VIBER EDITOR: preview e autoria do grip das armas no Vael.
--
-- O editor carrega `<raiz do projeto>/editor/plugins/*.lua` e chama os
-- callbacks do manifesto (`Viber/editor/PLUGINS.md`). Toda a regra de jogo
-- (nó, props, item, osso) vive aqui; o editor só oferece as ações genéricas
-- `editor.attach/detach/focus_player/set_props`.
--
-- O nó marcado com `groups="grip-preview"` (world/gameplay.xml) exporta:
--   preview (espada|tocha|machado|picareta), osso,
--   <item>-modelo, <item>-{x,y,z} (metros),
--   <item>-{rx,ry,rz} (graus), <item>-escala.
-- `preview` escolhe a arma/ferramenta mostrada; as mesmas props afinam o jogo
-- (scripts/game/equipment.lua).

local KNOWN = { "espada", "tocha", "machado", "picareta" }
local ROT_KEYS = { "rx", "ry", "rz" }
local POS_KEYS = { "x", "y", "z" }

local state = {
  item = nil,
  offsets = {},
  auto_focus = false,
  focused = false,
}

local function text(value, default)
  if type(value) == "string" and value ~= "" then
    return value
  end
  return default
end

local function number(value, default)
  local n = tonumber(value)
  if n == nil then
    return default
  end
  return n
end

local function grip_node(nodes)
  for _, node in ipairs(nodes or {}) do
    if node.groups and node.groups["grip-preview"] then
      return node
    end
  end
  return nil
end

local function item_list(props)
  local out = {}
  for _, id in ipairs(KNOWN) do
    if text(props[id .. "-modelo"], "") ~= "" then
      out[#out + 1] = id
    end
  end
  if #out == 0 then
    out[1] = "espada"
  end
  return out
end

local function current_item(props)
  local items = item_list(props)
  local wanted = text(state.item, text(props.preview, text(props["item-inicial"], items[1])))
  for _, id in ipairs(items) do
    if id == wanted then
      return id
    end
  end
  return items[1]
end

local function offset(props, item, key)
  local saved = state.offsets[item]
  local value = saved and saved[key]
  if value ~= nil then
    return value
  end
  return number(props[item .. "-" .. key], key == "escala" and 1.0 or 0.0)
end

local function remember_offsets(props, item)
  local saved = {}
  for _, key in ipairs(POS_KEYS) do
    saved[key] = offset(props, item, key)
  end
  for _, key in ipairs(ROT_KEYS) do
    saved[key] = offset(props, item, key)
  end
  saved.escala = offset(props, item, "escala")
  state.offsets[item] = saved
end

local function attach(ctx, node, item)
  local props = node.props
  local url = text(props[item .. "-modelo"], "")
  if url == "" then
    ctx.editor.detach("grip")
    return
  end
  local saved = state.offsets[item] or {}
  ctx.editor.attach({
    slot = "grip",
    url = url,
    bone = text(props.osso, "hand_r"),
    x = number(saved.x, number(props[item .. "-x"], 0.0)),
    y = number(saved.y, number(props[item .. "-y"], 0.0)),
    z = number(saved.z, number(props[item .. "-z"], 0.0)),
    -- As props de rotação estão em GRAUS no Inspector; a API usa radianos.
    rx = math.rad(number(saved.rx, number(props[item .. "-rx"], 0.0))),
    ry = math.rad(number(saved.ry, number(props[item .. "-ry"], 0.0))),
    rz = math.rad(number(saved.rz, number(props[item .. "-rz"], 0.0))),
    scale = number(saved.escala, number(props[item .. "-escala"], 1.0)),
  })
end

local function asset_exists(ctx, url)
  if type(url) ~= "string" or url == "" then
    return false
  end
  local rel = url:gsub("^/", "")
  if ctx.editor.fs.exists(rel) then
    return true
  end
  for _, root in ipairs(ctx.project.assets_roots or {}) do
    if ctx.editor.fs.exists(root .. "/" .. rel) then
      return true
    end
  end
  return false
end

local function validate(ctx)
  local node = grip_node(ctx.nodes)
  if not node then
    return { "não há nenhum nó com groups=\"grip-preview\" (world/gameplay.xml)" }
  end
  local problems = {}
  local item = current_item(node.props)
  local url = text(node.props[item .. "-modelo"], "")
  if url == "" then
    problems[#problems + 1] = "o item `" .. item .. "` não tem `" .. item .. "-modelo`"
  elseif not asset_exists(ctx, url) then
    problems[#problems + 1] = "modelo não encontrado: " .. url
  end
  return problems
end

function on_start(ctx)
  state.item = ctx.editor.prefs.get("item")
  state.auto_focus = ctx.editor.prefs.get("auto_focus") == true
end

function on_update(ctx)
  local node = grip_node(ctx.nodes)
  if not node then
    state.focused = false
    ctx.editor.detach("grip")
    return
  end
  local item = current_item(node.props)
  if state.item ~= item then
    state.item = item
    ctx.editor.prefs.set("item", item)
  end
  if state.auto_focus and ctx.player and not state.focused then
    state.focused = ctx.editor.focus_player({ distance = 5.0, height = 1.1, pitch = 20.0 })
  end
  attach(ctx, node, item)
end

function on_event(ctx, event)
  if event.panel ~= "grip" then
    return
  end
  local node = grip_node(ctx.nodes)
  if not node then
    return
  end
  local item = current_item(node.props)
  local selector = node.selector

  if event.kind == "change" then
    if event.id == "item" then
      state.item = event.value
      state.offsets[event.value] = nil
      ctx.editor.prefs.set("item", event.value)
      remember_offsets(node.props, event.value)
      return
    end
    if event.id == "auto_focus" then
      state.auto_focus = event.value == "true"
      if not state.auto_focus then
        state.focused = false
      end
      ctx.editor.prefs.set("auto_focus", state.auto_focus)
      return
    end
    for _, key in ipairs(POS_KEYS) do
      if event.id == key then
        state.offsets[item] = state.offsets[item] or {}
        state.offsets[item][key] = tonumber(event.value) or 0.0
        return
      end
    end
    for _, key in ipairs(ROT_KEYS) do
      if event.id == key then
        state.offsets[item] = state.offsets[item] or {}
        state.offsets[item][key] = tonumber(event.value) or 0.0
        return
      end
    end
    if event.id == "escala" then
      state.offsets[item] = state.offsets[item] or {}
      state.offsets[item].escala = tonumber(event.value) or 1.0
      return
    end
  end

  if event.kind == "click" and event.id == "apply" then
    remember_offsets(node.props, item)
    local saved = state.offsets[item] or {}
    ctx.editor.set_props(selector, {
      [item .. "-x"] = saved.x or 0.0,
      [item .. "-y"] = saved.y or 0.0,
      [item .. "-z"] = saved.z or 0.0,
      [item .. "-rx"] = saved.rx or 0.0,
      [item .. "-ry"] = saved.ry or 0.0,
      [item .. "-rz"] = saved.rz or 0.0,
      [item .. "-escala"] = saved.escala or 1.0,
      preview = item,
    })
    ctx.editor.toast("success", "Grip aplicado ao nó.")
  elseif event.kind == "click" and event.id == "focus" then
    state.focused = ctx.editor.focus_player({ distance = 5.0, height = 1.1, pitch = 20.0 })
  elseif event.kind == "click" and event.id == "reset" then
    state.offsets[item] = nil
    ctx.editor.detach("grip")
  end
end

function on_command(ctx, id)
  if id == "focus" then
    ctx.editor.focus_player({ distance = 5.0, height = 1.1, pitch = 20.0 })
  elseif id == "open" then
    ctx.editor.open_panel("grip")
  end
end

function render(ctx)
  local node = grip_node(ctx.nodes)
  local children = {
    { type = "heading", text = "Grip" },
    { type = "label", text = "Preview da arma/ferramenta no osso do herói.", tone = "dim" },
  }
  if not node then
    children[#children + 1] = {
      type = "label",
      text = "Sem nó groups=\"grip-preview\". Adiciona-o em world/gameplay.xml.",
      tone = "warn",
    }
    return { type = "column", gap = 6, children = children }
  end

  local props = node.props
  local item = current_item(props)
  local options = {}
  for _, id in ipairs(item_list(props)) do
    options[#options + 1] = { value = id, label = id }
  end
  local saved = state.offsets[item] or {}

  children[#children + 1] = {
    type = "select",
    id = "item",
    label = "item",
    value = item,
    options = options,
  }
  children[#children + 1] = {
    type = "label",
    text = "osso: " .. text(props.osso, "hand_r"),
    tone = "dim",
  }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "number", id = "x", label = "x", value = number(saved.x, number(props[item .. "-x"], 0.0)), step = 0.01 },
      { type = "number", id = "y", label = "y", value = number(saved.y, number(props[item .. "-y"], 0.0)), step = 0.01 },
      { type = "number", id = "z", label = "z", value = number(saved.z, number(props[item .. "-z"], 0.0)), step = 0.01 },
    },
  }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "number", id = "rx", label = "rx°", value = number(saved.rx, number(props[item .. "-rx"], 0.0)), step = 1 },
      { type = "number", id = "ry", label = "ry°", value = number(saved.ry, number(props[item .. "-ry"], 0.0)), step = 1 },
      { type = "number", id = "rz", label = "rz°", value = number(saved.rz, number(props[item .. "-rz"], 0.0)), step = 1 },
      { type = "number", id = "escala", label = "×", value = number(saved.escala, number(props[item .. "-escala"], 1.0)), step = 0.05, min = 0.01 },
    },
  }
  children[#children + 1] = {
    type = "checkbox",
    id = "auto_focus",
    label = "Enquadrar o herói automaticamente",
    value = state.auto_focus,
  }
  children[#children + 1] = {
    type = "row",
    gap = 6,
    children = {
      { type = "button", id = "apply", text = "Aplicar ao nó", primary = true },
      { type = "button", id = "focus", text = "Enquadrar herói" },
      { type = "button", id = "reset", text = "Repor preview" },
    },
  }
  for _, problem in ipairs(validate(ctx)) do
    children[#children + 1] = { type = "label", text = problem, tone = "warn" }
  end
  return { type = "column", gap = 6, children = children }
end

function lint(ctx)
  local findings = {}
  for _, problem in ipairs(validate(ctx)) do
    findings[#findings + 1] = { severity = "warning", kind = "grip", message = problem }
  end
  return findings
end

return {
  api = 1,
  id = "grip",
  title = "Grip",
  on_start = on_start,
  on_update = on_update,
  on_event = on_event,
  on_command = on_command,
  panels = {
    { id = "grip", title = "Grip", render = render },
  },
  commands = {
    { id = "focus", title = "Grip: enquadrar herói", keywords = "arma tocha espada" },
    { id = "open", title = "Grip: abrir painel", keywords = "arma tocha espada" },
  },
  lint = lint,
}
