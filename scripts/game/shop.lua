local rules = viber.load("lib/rules.lua")

-- A loja corre no mercador (controller no mundo) e no UiRoot (menu): sem
-- posição explícita, `viber.sound` tocava na origem do controller/UI e o
-- sino da compra ouvia-se na praça, não na banca.
local function sfx(name)
  local has, x, y, z = viber.player_position()
  if has then
    viber.sound(name, { x = x, y = y, z = z })
  else
    viber.sound(name)
  end
end
-- game/shop.lua: a LOJA é do JOGO — catálogo, preços e negociação em Lua.
--
-- A engine só publica a ação: os botões da tab Loja fazem
-- `viber.ui.action("buy"/"sell", item)` e chegam aqui via `viber.events()`
-- como {type="ui_action", name=…, arg=…}. Este módulo reclama a fonte da
-- lista `shop` (`viber.ui.list` — a engine deixa de a alimentar) e reclama
-- as ações (`viber.own_action` — o catálogo nativo cala). Sem este módulo,
-- a loja nativa da engine continua a funcionar (compat).
--
-- Uso (ver merchant.lua): `local shop = viber.load("game/shop.lua")` no
-- top-level e `shop.update(dt)` dentro do `on_update`. O mercador leva
-- `tag="always-active"`: sem isso o LOD de 45 m congelava o script, os
-- cliques buy/sell acumulavam na fila e descarregavam todos ao voltar
-- (compras em rajada) — e a lista ficava no ouro do frame em que a loja
-- abriu, porque `refresh` lia o snapshot ANTES dos comandos da compra.

-- Catálogo do mercador: buy = preço de compra (nil = não se vende ao herói),
-- sell = preço de venda (nil = não se compra para si). Preços em ouro.
-- O teto de pilha é o mesmo de `lib/rules.lua` (STACK=99): comprar no limite
-- tem de aparecer bloqueado, não como uma linha "buy" que só dá erro.
local STACK_MAX = rules.stack_max()

-- Catálogo do mercador: vem de `data/items.json` (`shop.buy`/`shop.sell`).
-- Um item sem `shop` não entra na banca; os preços continuam a ser dados do
-- jogo, não do código da loja.
local function build_catalog()
  local out = {}
  for _, item in ipairs(rules.items()) do
    local shop = item.shop
    if type(shop) == "table" and (shop.buy or shop.sell) then
      out[item.id] = {
        label = shop.label or item.label,
        buy = shop.buy,
        sell = shop.sell,
      }
    end
  end
  return out
end

local CATALOG = build_catalog()

local M = {}

local function rows()
  local out = {}
  local gold = rules.vault_get("gold")
  for id, entry in pairs(CATALOG) do
    local selling = entry.sell ~= nil
    local stock = rules.vault_get(id)
    -- status alimenta a classe CSS da linha ({status} no template). As
    -- classes `buy`/`sell`/`unaffordable` existiam no menu.css mas o script
    -- só emitia ready/locked: a linha não distinguia comprar de vender.
    local status
    if selling then
      status = (stock or 0) > 0 and "sell" or "locked"
    elseif (stock or 0) >= STACK_MAX then
      -- Pilha cheia: o clique só produzia "Já tens o máximo". A linha passa
      -- a nascer bloqueada, como as de venda sem stock.
      status = "locked"
    else
      status = gold >= entry.buy and "buy" or "unaffordable"
    end
    local label = entry.label
    if selling then
      label = label .. " ×" .. tostring(stock or 0)
    end
    out[#out + 1] = {
      id = id,
      label = label,
      qty = "1",
      price = selling and ("+" .. entry.sell) or tostring(entry.buy),
      status = status,
    }
  end
  table.sort(out, function(a, b) return a.label < b.label end)
  return out
end

-- A lista só é reescrita quando o conteúdo muda: `viber.ui.list` sobe a
-- versão do repetidor e reconstrói as linhas (perde o scroll) se for
-- chamada todos os frames com a mesma tabela.
local last_sig = ""

local function signature(list)
  local parts = {}
  for i, row in ipairs(list) do
    parts[i] = row.id .. "\0" .. row.status .. "\0" .. row.price .. "\0" .. row.label
  end
  return table.concat(parts, "\1")
end

local function refresh()
  local list = rows()
  local sig = signature(list)
  if sig == last_sig then
    return
  end
  last_sig = sig
  viber.ui.list("shop", list)
end

-- top-level do módulo (corre 1× por mundo): reclama lista + ações.
viber.own_action("buy")
viber.own_action("sell")
refresh()

local function trade(action, id)
  local entry = CATALOG[id]
  if entry == nil then return end
  if action == "buy" then
    local price = entry.buy
    if price == nil then
      viber.toast("Não vendo isso.")
      sfx("error")
      return
    end
    if rules.vault_get("gold") < price then
      viber.toast(entry.label .. " custa " .. price .. " de ouro — não chega.")
      sfx("error")
      return
    end
    if rules.vault_get(id) >= STACK_MAX then
      viber.toast("Já tens o máximo disso (" .. STACK_MAX .. ").")
      sfx("error")
      return
    end
    rules.vault_take("gold", price)
    local got = rules.item_add(id, 1)
    if not got or got < 1 then
      rules.vault_add("gold", price)
      viber.toast("Não coube na mochila.")
      sfx("error")
      return
    end
    sfx("buy")
    viber.toast("Comprado " .. entry.label:gsub("^Comprar ", "") .. ".")
  elseif action == "sell" then
    local price = entry.sell
    if price == nil or rules.vault_get(id) == 0 then
      viber.toast("Sem isso para vender.")
      sfx("error")
      return
    end
    -- A linha pode ter sido montada com stock de um frame anterior (ou o
    -- item pode ter saído noutro caminho): sem esta guarda o ouro entrava
    -- mesmo quando `vault_take` falhava.
    if not rules.vault_take(id, 1) then
      viber.toast("Sem isso para vender.")
      sfx("error")
      return
    end
    rules.vault_add("gold", price)
    sfx("coin")
    viber.toast("Vendido por " .. price .. " de ouro.")
  end
  refresh()
end

-- Compra/venda direta para a UI (ui/menu.lua). O caminho por
-- `viber.ui.action` depende de quem carregou o módulo ser o dono da ação; em
-- interiores sem mercador a fila nunca era drenada. A UI autoload chama
-- isto e a negociação funciona em qualquer cena, no frame do clique.
M.trade = trade

function M.update(dt)
  -- Snapshot deste frame (já inclui as compras do frame anterior). Sem
  -- isto a linha ficava "pronta" depois de o ouro acabar, até ao clique
  -- seguinte — e o ouro de um abate nunca destrancava uma compra.
  refresh()
  -- Os CLIQUES das linhas são roteados pelo ui/menu.lua
  -- (`shop-{id}` → `viber.ui.action`). Várias entidades `merchant.lua` na
  -- mesma cena partilham ESTA fila (uma por path): a primeira drena, a
  -- segunda vê-a vazia — uma compra não cobra duas vezes.
  for _, ev in ipairs(viber.events()) do
    if ev.type == "ui_action" and (ev.name == "buy" or ev.name == "sell") then
      trade(ev.name, ev.arg)
    end
  end
end

return M
