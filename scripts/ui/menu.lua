-- simple-rpg — comportamento do menu.
--
-- Missões, mochila e talentos vêm de ui/journal.lua. A loja vem de
-- game/shop.lua. Este script trata dos cliques.

local journal = viber.load("ui/journal.lua")

local SKILLS = {
  "vitality1", "strength1", "agility1", "precision1",
  "vitality2", "strength2", "agility2", "precision2",
}

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
