local rules = viber.load("lib/rules.lua")
local actions = viber.load("lib/actions.lua")

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
-- Teto de pilha por omissão (é o de `lib/rules.lua`: STACK=99); um item com
-- `stack` próprio usa esse valor (`cap_of`). Comprar no limite tem de aparecer
-- bloqueado, não como uma linha "buy" que só dá erro.
local STACK_MAX = rules.stack_max()

-- Catálogo do mercador: vem de `data/items.json` (`shop.buy`/`shop.sell`).
-- Um item sem `shop` não entra na banca; os preços continuam a ser dados do
-- jogo, não do código da loja.
--
-- Os preços vêm de um JSON editável: um valor que não seja um número finito
-- não pode entrar no catálogo. `vault_take` rejeita um preço não finito e o
-- retorno era ignorado — `buy: NaN` dava o item de graça — e vender a 0/negativo
-- apagava o item sem pagar. `zero_ok` preserva uma linha de oferta a 0 ouro.
local function price_of(value, zero_ok)
  if type(value) ~= "number" or value ~= value or value < 0 or value == math.huge then
    return nil
  end
  if value == 0 and not zero_ok then
    return nil
  end
  return value
end

local function build_catalog()
  local out = {}
  for _, item in ipairs(rules.items()) do
    local shop = item.shop
    if type(shop) == "table" then
      local buy, sell = price_of(shop.buy, true), price_of(shop.sell, false)
      if buy or sell then
        -- Um label que não seja texto rebentava a lista (`label .. " ×N"`).
        local label = shop.label
        if type(label) ~= "string" or label == "" then
          label = item.label
        end
        if type(label) ~= "string" or label == "" then
          label = item.id
        end
        out[item.id] = {
          label = label,
          buy = buy,
          sell = sell,
        }
      end
    end
  end
  return out
end

local CATALOG = build_catalog()

-- Teto REAL de cada pilha: `rules.item_add` respeita o `stack` do item; o
-- catálogo tem de usar o mesmo número. Com STACK_MAX fixo, a linha nascia
-- "buy" e o clique só descobria o limite depois de tirar o ouro.
local function cap_of(id)
  local def = rules.item_def and rules.item_def(id)
  local stack = def and tonumber(def.stack)
  if stack and stack > 0 then
    return math.floor(stack)
  end
  return STACK_MAX
end

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
    elseif (stock or 0) >= cap_of(id) then
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

-- Fora de uma cena com mercador nada chamava `update` (é o mercador quem
-- drena a fila): depois de um Carregar dentro de uma casa a lista ficava com
-- o ouro/stock do jogo anterior até ao primeiro clique. Um tick lento
-- mantém-na atual em qualquer cena; a assinatura evita reescrever a UI. O
-- estado guarda o handle para o hot-reload não acumular timers.
local ok_state, st = pcall(viber.state)
if ok_state and type(st) == "table" then
  if st.refresh_timer then
    pcall(viber.timer_cancel, st.refresh_timer)
  end
  local ok_timer, handle = pcall(viber.every, 0.5, refresh)
  st.refresh_timer = ok_timer and handle or nil
end

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
    if rules.vault_get(id) >= cap_of(id) then
      viber.toast("Já tens o máximo disso (" .. cap_of(id) .. ").")
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
    actions.record("comprar", "item", id)
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
    local paid = rules.vault_add("gold", price) or 0
    if paid < price then
      -- O ouro tem teto global (`rules.vault_add` clampa): sem caber, o
      -- parcial volta a sair e o item regressa ao cofre — senão sobrava ouro
      -- por cada venda recusada e o item ficava.
      if paid > 0 then
        rules.vault_take("gold", paid)
      end
      rules.vault_add(id, 1)
      viber.toast("Não coube o ouro na bolsa.")
      sfx("error")
      refresh()
      return
    end
    sfx("coin")
    actions.record("vender", "item", id)
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
