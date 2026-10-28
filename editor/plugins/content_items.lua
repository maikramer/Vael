-- Plugin do Vael: editor de itens e da loja (`data/items.json`).
--
-- A loja é uma vista do mesmo registo: `shop.buy`/`shop.sell` no item. Um
-- preço 0/ausente tira o item da banca.

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

local function load_all(ctx)
  local data, err = read_json(ctx, "data/items.json")
  if type(data) ~= "table" or type(data.items) ~= "table" then
    state.error = err or "data/items.json sem `items`"
    state.loaded = true
    return
  end
  state.data = data
  state.error = nil
  local saved = ctx.editor.prefs.get("items.id")
  state.selected = nil
  for _, item in ipairs(data.items) do
    if item.id == saved then
      state.selected = saved
      break
    end
  end
  state.loaded = true
end

local function find_item(id)
  for _, item in ipairs(state.data and state.data.items or {}) do
    if item.id == id then
      return item
    end
  end
  return nil
end

local function unique_id()
  local base = "novo_item"
  local id = base
  local index = 1
  while find_item(id) do
    index = index + 1
    id = base .. "_" .. index
  end
  return id
end

local function sorted_items()
  local out = copy(state.data and state.data.items or {})
  table.sort(out, function(a, b)
    return (tonumber(a.order) or 0) < (tonumber(b.order) or 0)
  end)
  return out
end

local function save_data(ctx)
  if not state.data then
    return
  end
  state.error = nil
  local seen = {}
  for _, item in ipairs(state.data.items) do
    local id = tostring(item.id or "")
    if id == "" then
      state.error = "há um item sem id"
      return
    end
    if seen[id] then
      state.error = "id duplicado: " .. id
      return
    end
    seen[id] = true
    if tostring(item.label or "") == "" then
      state.error = "item `" .. id .. "` sem label"
      return
    end
    if item.shop then
      local buy = tonumber(item.shop.buy) or 0
      local sell = tonumber(item.shop.sell) or 0
      if buy < 0 or sell < 0 then
        state.error = "item `" .. id .. "`: preço negativo"
        return
      end
    end
  end
  -- Préços 0/0 sem label tiram o item da banca — só DEPOIS de validar tudo:
  -- dentro da validação, um item seguinte que falhasse deixava a memória
  -- (loja já limpa) divergida do disco (nada gravado).
  for _, item in ipairs(state.data.items) do
    if type(item.shop) == "table" then
      local buy = tonumber(item.shop.buy) or 0
      local sell = tonumber(item.shop.sell) or 0
      if buy == 0 and sell == 0 and tostring(item.shop.label or "") == "" then
        item.shop = nil
      end
    end
  end
  local data = copy(state.data)
  data.items = ctx.editor.fs.json.array(data.items)
  local ok, err = write_json(ctx, "data/items.json", data)
  if not ok then
    state.error = err or "falha a gravar"
    return
  end
  ctx.editor.toast("success", "Itens e loja gravados.")
end

local function shop(item)
  if type(item.shop) ~= "table" then
    item.shop = {}
  end
  return item.shop
end

local function render(ctx)
  if not state.loaded then
    return { type = "column", gap = 4, children = { { type = "label", text = "A carregar itens…", tone = "dim" } } }
  end
  local children = {
    { type = "heading", text = "Itens e loja" },
    { type = "label", text = "data/items.json — labels, pilhas e preços (buy/sell).", tone = "dim" },
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
      { type = "button", id = "new", text = "Novo item" },
      { type = "button", id = "save", text = "Gravar", primary = true },
      { type = "button", id = "reload", text = "Recarregar" },
    },
  }
  local rows = {}
  for _, item in ipairs(sorted_items()) do
    local sale = item.shop and ((item.shop.buy and ("buy " .. item.shop.buy)) or
      (item.shop.sell and ("sell " .. item.shop.sell))) or ""
    rows[#rows + 1] = {
      id = item.id,
      label = tostring(item.id) .. "  ·  " .. tostring(item.label or "") ..
        (sale ~= "" and ("  ·  " .. sale) or ""),
    }
  end
  children[#children + 1] = {
    type = "list",
    id = "items",
    selected = state.selected,
    items = rows,
    height = 190,
  }

  local item = state.selected and find_item(state.selected) or nil
  if item then
    local shop_data = type(item.shop) == "table" and item.shop or {}
    children[#children + 1] = { type = "separator" }
    children[#children + 1] = { type = "heading", text = "Item" }
    children[#children + 1] = { type = "label", text = "id: " .. item.id, tone = "dim" }
    children[#children + 1] = { type = "text", id = "label", label = "label", value = item.label or "" }
    children[#children + 1] = {
      type = "select",
      id = "kind",
      label = "kind",
      value = item.kind or "item",
      options = {
        { value = "item", label = "item" },
        { value = "resource", label = "resource (teto alto)" },
      },
    }
    children[#children + 1] = {
      type = "row",
      gap = 6,
      children = {
        { type = "number", id = "stack", label = "stack", value = tonumber(item.stack) or 99, step = 1, min = 1 },
        { type = "number", id = "order", label = "order", value = tonumber(item.order) or 0, step = 1, min = 0 },
      },
    }
    children[#children + 1] = { type = "heading", text = "Loja" }
    children[#children + 1] = {
      type = "row",
      gap = 6,
      children = {
        { type = "number", id = "buy", label = "buy", value = tonumber(shop_data.buy) or 0, step = 1, min = 0 },
        { type = "number", id = "sell", label = "sell", value = tonumber(shop_data.sell) or 0, step = 1, min = 0 },
      },
    }
    children[#children + 1] = { type = "text", id = "shop_label", label = "label da loja", value = shop_data.label or "" }
    children[#children + 1] = {
      type = "button",
      id = "delete",
      text = state.pending_delete == state.selected and "Confirmar apagar" or "Apagar item",
    }
  else
    children[#children + 1] = { type = "label", text = "Seleciona um item.", tone = "dim" }
  end
  return { type = "column", gap = 6, children = children }
end

local function on_event(ctx, event)
  if event.panel ~= "items" then
    return
  end
  if event.kind == "select" and event.id == "items" then
    state.selected = event.value
    state.pending_delete = nil
    ctx.editor.prefs.set("items.id", event.value)
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
    local id = unique_id()
    local item = {
      id = id,
      label = "Novo item",
      kind = "item",
      stack = 99,
      order = #(state.data.items or {}),
    }
    state.data.items[#state.data.items + 1] = item
    state.selected = id
    state.pending_delete = nil
    return
  end
  local item = state.selected and find_item(state.selected) or nil
  if not item then
    return
  end
  if event.kind == "click" and event.id == "delete" then
    if state.pending_delete ~= state.selected then
      state.pending_delete = state.selected
      return
    end
    for index, entry in ipairs(state.data.items) do
      if entry.id == state.selected then
        table.remove(state.data.items, index)
        break
      end
    end
    state.selected = nil
    state.pending_delete = nil
    return
  end
  if event.kind == "change" then
    if event.id == "label" then
      item.label = event.value
    elseif event.id == "kind" then
      item.kind = event.value
      if event.value == "resource" then
        item.stack = tonumber(state.data.resource_cap) or 1000000
      else
        -- Voltar a item normal: o stack do recurso (1.000.000) ficava no
        -- item e `rules.vault_add` usa `def.stack` como teto real.
        local stack_max = tonumber(state.data.stack_max) or 99
        if (tonumber(item.stack) or 0) > stack_max then
          item.stack = stack_max
        end
      end
    elseif event.id == "stack" then
      item.stack = tonumber(event.value) or 1
    elseif event.id == "order" then
      item.order = tonumber(event.value) or 0
    elseif event.id == "buy" then
      local value = tonumber(event.value) or 0
      if value > 0 then shop(item).buy = value else shop(item).buy = nil end
    elseif event.id == "sell" then
      local value = tonumber(event.value) or 0
      if value > 0 then shop(item).sell = value else shop(item).sell = nil end
    elseif event.id == "shop_label" then
      shop(item).label = event.value
    end
  end
end

local function lint(ctx)
  if not state.loaded then
    load_all(ctx)
  end
  local findings = {}
  if not state.data then
    return { { severity = "error", kind = "items", message = state.error or "data/items.json inválido" } }
  end
  local seen = {}
  for _, item in ipairs(state.data.items or {}) do
    local id = tostring(item.id or "")
    if id == "" or seen[id] then
      findings[#findings + 1] = { severity = "error", kind = "items", message = "id vazio ou duplicado: " .. id }
    end
    seen[id] = true
    if tostring(item.label or "") == "" then
      findings[#findings + 1] = { severity = "warning", kind = "items", message = id .. ": sem label" }
    end
    if item.shop and ((tonumber(item.shop.buy) or 0) < 0 or (tonumber(item.shop.sell) or 0) < 0) then
      findings[#findings + 1] = { severity = "error", kind = "items", message = id .. ": preço negativo" }
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
  id = "items",
  title = "Itens",
  on_update = on_update,
  on_event = on_event,
  panels = {
    { id = "items", title = "Itens", render = render },
  },
  lint = lint,
}
