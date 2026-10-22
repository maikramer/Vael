local rules = viber.load("lib/rules.lua")
-- chest.lua: baú de uma abertura só — loot real no vault/inventário via hooks
-- da economia. O conteúdo é de CADA baú (`prop-*` no XML / Inspector › Script).
--@export xp: int(0..) = 30                 -- XP ao abrir
--@export gold: int(0..) = 25               -- ouro ao abrir
--@export item: string = "potion"           -- item do inventário ("" = nenhum)
--@export item_count: int(0..) = 1          -- quantos desse item
--@export item_label: string = "poção"      -- nome do item no toast
--@export prompt: string = "Abrir o baú"
--@export range: number(0.5..10) = 2.8      -- alcance do [E] (m)
--@signal opened(gold, xp)                  -- abriu (1×) — quem ouve é dado do baú: connect-opened="@grupo:método"

local once = viber.load("lib/once.lua")

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (o top-level é partilhado)
  if not st.ready then
    st.ready = true
    -- state() morre no load: sem a marca em viber.game() cada baú pagava outra vez.
    st.opened = once.taken()
    viber.set_interaction(st.opened and "Vazio" or viber.prop("prompt"), "e", viber.prop("range"))
  end
  -- Relê a marca todos os frames: um Load na MESMA cena troca o
  -- `viber.game()` sem recriar a entidade; com `st.opened` em cache o baú
  -- voltava a pagar (ou ficava "Abrir" depois de carregar um save aberto).
  local taken = once.taken()
  if taken ~= st.opened then
    st.opened = taken
    viber.set_interaction(taken and "Vazio" or viber.prop("prompt"), "e", viber.prop("range"))
  end
  if viber.menu_open() then
    return
  end
  if viber.interacted("e") and not st.opened then
    st.opened = true
    once.mark()
    -- O prompt nasce no ready: sem o re-registar continuava "Abrir o baú".
    viber.set_interaction("Vazio", "e", viber.prop("range"))
    local p = viber.props()
    local loot = {}
    local short = false
    if p.xp > 0 then
      rules.add_xp(p.xp)
      loot[#loot + 1] = "+" .. p.xp .. " XP"
    end
    local paid_gold = 0
    if p.gold > 0 then
      -- `vault_add` devolve o que REALMENTE entrou (teto de 1e6 no ouro).
      paid_gold = rules.vault_add("gold", p.gold) or 0
      if paid_gold > 0 then
        loot[#loot + 1] = "+" .. paid_gold .. " ouro"
      end
      if paid_gold < p.gold then
        short = true
      end
    end
    if p.item ~= "" and p.item_count > 0 then
      -- A mochila tem teto de 99: o toast dizia "+3 poções" mesmo quando só
      -- uma cabia. Mostra o crédito real e avisa que ficou conteúdo atrás.
      local paid_items = rules.item_add(p.item, p.item_count) or 0
      if paid_items > 0 then
        loot[#loot + 1] = "+" .. paid_items .. " " .. p.item_label
      end
      if paid_items < p.item_count then
        short = true
      end
    end
    -- O sinal leva o que foi pago, não o que estava autorado no baú.
    viber.emit("opened", paid_gold, p.xp)
    local ok, x, y, z = viber.entity_position()
    if ok then
      viber.sound("chest_open", { x = x, y = y, z = z })
    else
      viber.sound("chest_open")
    end
    if #loot == 0 then
      if short then
        viber.toast("O baú range e cede... mas não coube nada na mochila.")
      else
        viber.toast("O baú range e cede... está vazio.")
      end
    else
      local tail = short and " (não coube tudo)" or ""
      viber.toast("O baú range e cede... " .. table.concat(loot, ", ") .. tail)
    end
  elseif viber.interacted("e") then
    viber.toast("O baú está vazio.")
  end
end
