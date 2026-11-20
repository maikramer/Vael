local rules = vael.load("lib/rules.lua")
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

local once = vael.load("lib/once.lua")

function on_update(dt)
  local st = vael.state() -- POR ENTIDADE (o top-level é partilhado)
  if not st.ready then
    st.ready = true
    -- state() morre no load: sem a marca em vael.game() cada baú pagava outra vez.
    st.opened = once.taken()
    vael.set_interaction(st.opened and "Vazio" or vael.prop("prompt"), "e", vael.prop("range"))
  end
  -- Relê a marca todos os frames: um Load na MESMA cena troca o
  -- `vael.game()` sem recriar a entidade; com `st.opened` em cache o baú
  -- voltava a pagar (ou ficava "Abrir" depois de carregar um save aberto).
  local taken = once.taken()
  if taken ~= st.opened then
    st.opened = taken
    vael.set_interaction(taken and "Vazio" or vael.prop("prompt"), "e", vael.prop("range"))
  end
  if vael.menu_open() then
    return
  end
  -- O `menu_open` não inclui o ecrã de morte: o herói caído continuava a
  -- receber [E] e um baú pagava loot/XP depois de morrer.
  local _, hp = rules.player_hp()
  if hp <= 0 then
    return
  end
  if vael.interacted("e") and not st.opened then
    local p = vael.props()
    local loot = {}
    local want_items = (p.item ~= "" and p.item_count > 0) and p.item_count or 0
    -- Capacidade ANTES de marcar/entregar: o baú é o único detentor deste
    -- conteúdo. A ordem antiga fechava o baú primeiro e o `item_add`/`vault_add`
    -- só aceitavam o que cabia — o resto (pilha a 99, ouro no tecto) perdia-se
    -- para sempre, apesar do aviso "(não coube tudo)". Testa-se o crédito real
    -- (`vault_add` conhece o tecto dos recursos e o `stack` de cada item) e
    -- devolve-se a parte que entrou, para o baú ficar fechado e intacto.
    local paid_items = 0
    if want_items > 0 then
      paid_items = rules.item_add(p.item, want_items) or 0
      if paid_items < want_items then
        if paid_items > 0 then
          rules.vault_take(p.item, paid_items)
        end
        vael.toast("Não cabe tudo na mochila — o baú continua fechado.")
        vael.sound("error")
        return
      end
    end
    local paid_gold = 0
    if p.gold > 0 then
      paid_gold = rules.vault_add("gold", p.gold) or 0
      if paid_gold < p.gold then
        if paid_gold > 0 then
          rules.vault_take("gold", paid_gold)
        end
        if paid_items > 0 then
          rules.vault_take(p.item, paid_items)
        end
        vael.toast("Não cabe o ouro na bolsa — o baú continua fechado.")
        vael.sound("error")
        return
      end
    end
    -- Tudo coube: só agora o baú fica aberto e o conteúdo é pago.
    st.opened = true
    once.mark()
    -- O prompt nasce no ready: sem o re-registar continuava "Abrir o baú".
    vael.set_interaction("Vazio", "e", vael.prop("range"))
    if p.xp > 0 then
      rules.add_xp(p.xp)
      loot[#loot + 1] = "+" .. p.xp .. " XP"
    end
    if paid_gold > 0 then
      loot[#loot + 1] = "+" .. paid_gold .. " ouro"
    end
    if paid_items > 0 then
      loot[#loot + 1] = "+" .. paid_items .. " " .. p.item_label
    end
    -- O sinal leva o que foi pago, não o que estava autorado no baú.
    vael.emit("opened", paid_gold, p.xp)
    local ok, x, y, z = vael.entity_position()
    if ok then
      vael.sound("chest_open", { x = x, y = y, z = z })
      if #loot > 0 then
        if vael.burst then
          vael.burst("sparkle", x, y + 0.85, z, 18)
        end
        if vael.vael and vael.vael.collect_burst then
          vael.vael.collect_burst("gold", x, y + 0.85, z, 6)
        end
      end
    else
      vael.sound("chest_open")
    end
    if #loot == 0 then
      vael.toast("O baú range e cede... está vazio.")
    else
      vael.toast("O baú range e cede... " .. table.concat(loot, ", "))
    end
  elseif vael.interacted("e") then
    vael.toast("O baú está vazio.")
  end
end
