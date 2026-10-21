-- simple-rpg — comportamento do menu.
--
-- Missões, mochila e talentos vêm de ui/journal.lua. A loja vem de
-- game/shop.lua. Este script trata dos cliques.

local journal = viber.load("ui/journal.lua")

local SKILLS = {
  "vitality1", "strength1", "agility1", "precision1",
  "vitality2", "strength2", "agility2", "precision2",
}

-- Acessibilidade: a engine expõe `viber.a11y`/`a11y_set`; o menu só desenha
-- as linhas e manda a alteração. `interact_hold` é lido pelo diálogo nativo.
local A11Y_ROWS = {
  { id = "shake", label = "Tremor da câmara" },
  { id = "hitstop", label = "Hit-stop no impacto" },
  { id = "interact_hold", label = "[E] por hold (0,35 s)" },
  { id = "subtitle", label = "Tamanho das legendas" },
}
local A11Y_SCALES = { 1.0, 1.25, 1.5, 2.0 }
local a11y_last

local function a11y_default()
  return { subtitle_scale = 1.0, shake = true, hitstop = true, interact_hold = false }
end

local function a11y_refresh()
  if not viber.a11y then
    return
  end
  local a = viber.a11y() or a11y_default()
  local sig = table.concat({
    tostring(a.subtitle_scale), tostring(a.shake),
    tostring(a.hitstop), tostring(a.interact_hold),
  }, "|")
  if sig == a11y_last then
    return
  end
  a11y_last = sig
  local rows = {}
  for _, row in ipairs(A11Y_ROWS) do
    local value
    if row.id == "subtitle" then
      value = string.format("%.2f×", a.subtitle_scale or 1)
    else
      value = a[row.id] and "ligado" or "desligado"
    end
    rows[#rows + 1] = { id = row.id, label = row.label, value = value }
  end
  viber.ui.list("a11y", rows)
end

local function a11y_clicks()
  if not viber.a11y or not viber.a11y_set then
    return
  end
  local a = viber.a11y() or a11y_default()
  for _, row in ipairs(A11Y_ROWS) do
    if viber.ui.clicked("a11y-" .. row.id) then
      if row.id == "subtitle" then
        local at = 1
        for i, scale in ipairs(A11Y_SCALES) do
          if math.abs((a.subtitle_scale or 1) - scale) < 0.01 then
            at = i % #A11Y_SCALES + 1
            break
          end
        end
        viber.a11y_set({ subtitle_scale = A11Y_SCALES[at] })
      else
        viber.a11y_set({ [row.id] = not a[row.id] })
      end
      a11y_last = nil
      viber.sound("ui")
    end
  end
end

-- A loja é dirigida pelas LINHAS da lista `shop` (game/shop.lua monta-as):
-- um preço "+N" é venda, o resto é compra. Com a lista fixa de antes
-- (só madeira/pedra) os despojos de quest apareciam mas o clique morria.
local function shop_clicks()
  for _, row in ipairs(viber.ui.rows("shop")) do
    if row.id and viber.ui.clicked("shop-" .. row.id) then
      local selling = tostring(row.price or ""):sub(1, 1) == "+"
      viber.ui.action(selling and "sell" or "buy", row.id)
    end
  end
end

function on_update(dt)
  if viber.ui.is_open("menu") then
    journal.refresh()
    a11y_refresh()
    a11y_clicks()
  end
  if viber.ui.clicked("menu-close") then
    viber.ui.open("menu", false)
  end

  for _, id in ipairs(SKILLS) do
    if viber.ui.clicked("skill-" .. id) then
      journal.learn(id)
    end
  end

  shop_clicks()

  if viber.ui.clicked("act-save") then viber.ui.action("save", "") end
  if viber.ui.clicked("act-load") then viber.ui.action("load", "") end
  if viber.ui.clicked("act-profiler") then
    viber.toast("Profiler: tecla P")
  end
end
