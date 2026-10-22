local rules = viber.load("lib/rules.lua")
-- bog-moss.lua: tufo de musgo-do-pântano colhível ([J]) — fonte do
-- item `bog-moss` (objetivo collect ×10 da quest do pântano).
local once = viber.load("lib/once.lua")
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE
  if not st.ready then
    st.ready = true
    -- Sem marca no save, gravar e carregar replantava o musgo e a quest farmava.
    if once.taken() then
      viber.despawn_self()
      return
    end
    viber.set_interaction("Colher musgo", "j", 2.5)
  end
  -- Relê a marca: um Load na mesma cena não recria a entidade.
  if once.taken() then
    if not st.done then
      st.done = true
      viber.despawn_self()
    end
    return
  end
  if viber.menu_open() then return end
  if viber.interacted("j") and not st.done then
    -- A bolsa de recursos tem teto de 99: com a pilha cheia o `item_add`
    -- devolvia 0 e o tufo era gasto na mesma (XP + despawn) sem dar o item.
    -- Sem crédito, o musgo fica no mundo para depois de fazer espaço.
    local added = rules.item_add("bog-moss", 1) or 0
    if added < 1 then
      viber.toast("A bolsa de musgo está cheia (99).")
      return
    end
    st.done = true
    once.mark()
    rules.add_xp(8)
    viber.toast("Musgo-do-pântano colhido (+8 XP, +1 bog-moss)")
    viber.topple()
    -- O prop tombado continuava a oferecer "Colher musgo" para sempre.
    viber.despawn_self()
  end
end
