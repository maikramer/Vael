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
-- game/shop.lua: a LOJA é do JOGO — a UI, o som e os toasts ficam aqui.
--
-- O catálogo, os preços, os tetos e a negociação vivem em `src/economy.rs`
-- (`viber.vael.shop_rows`/`shop_trade`). A engine só publica a ação: os
-- botões da tab Loja fazem `viber.ui.action("buy"/"sell", item)` e chegam
-- aqui via `viber.events()` como {type="ui_action", name=…, arg=…}. Este
-- módulo reclama a fonte da lista `shop` (`viber.ui.list` — a engine deixa de
-- a alimentar) e reclama as ações (`viber.own_action` — o catálogo nativo
-- cala). Sem este módulo, a loja nativa da engine continua a funcionar
-- (compat).
--
-- Uso (ver merchant.lua): `local shop = viber.load("game/shop.lua")` no
-- top-level e `shop.update(dt)` dentro do `on_update`. O mercador leva
-- `tag="always-active"`: sem isso o LOD de 45 m congelava o script, os
-- cliques buy/sell acumulavam na fila e descarregavam todos ao voltar
-- (compras em rajada) — e a lista ficava no ouro do frame em que a loja
-- abriu, porque `refresh` lia o snapshot ANTES dos comandos da compra.

local M = {}

-- Linhas do Rust (`shop_rows`: id/label/price/stock/status/selling) já vêm
-- ordenadas; aqui só se formata o texto da UI. O stock só aparece nas linhas
-- de venda e o prefixo "+" é o que o ui/menu.lua usa para distinguir venda
-- de compra.
local function rows()
  local out = {}
  for _, row in ipairs(viber.vael.shop_rows()) do
    local label = row.label
    if row.selling then
      label = label .. " ×" .. tostring(row.stock)
    end
    out[#out + 1] = {
      id = row.id,
      label = label,
      qty = "1",
      price = row.selling and ("+" .. row.price) or tostring(row.price),
      status = row.status,
    }
  end
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

-- Mensagem de falha da negociação (o resultado vem do Rust). Devolve `false`
-- para id/acção desconhecidos: o Lua antigo saía sem tocar na UI.
local function fail_toast(action, result)
  local reason = result.reason
  if reason == "no_stock" or (reason == "no_price" and action == "sell") then
    viber.toast("Sem isso para vender.")
  elseif reason == "no_price" then
    viber.toast("Não vendo isso.")
  elseif reason == "no_gold" then
    local label = result.label or ""
    viber.toast(label .. " custa " .. result.price .. " de ouro — não chega.")
  elseif reason == "full" then
    viber.toast("Já tens o máximo disso (" .. result.cap .. ").")
  elseif reason == "no_space" then
    viber.toast("Não coube na mochila.")
  elseif reason == "gold_full" then
    viber.toast("Não coube o ouro na bolsa.")
  else
    return false
  end
  return true
end

local function trade(action, id)
  local result = viber.vael.shop_trade(action, id)
  if type(result) ~= "table" then
    return
  end
  if result.ok then
    -- O cofre mudou diretamente em Rust: as quests de recolha têm de rever
    -- o progresso (o antigo `rules.vault_take/add` fazia-o em cada passo).
    rules.sync_collect()
    if action == "buy" then
      sfx("buy")
      actions.record("comprar", "item", id)
      local label = tostring(result.label or id)
      viber.toast("Comprado " .. label:gsub("^Comprar ", "") .. ".")
    elseif action == "sell" then
      sfx("coin")
      actions.record("vender", "item", id)
      viber.toast("Vendido por " .. result.price .. " de ouro.")
    end
  elseif not fail_toast(action, result) then
    return
  else
    sfx("error")
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
