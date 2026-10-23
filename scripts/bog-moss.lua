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
  -- O `menu_open` não inclui o ecrã de morte: o herói caído ainda colhia.
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  if viber.interacted("j") and not st.done then
    -- A bolsa de recursos tem tecto de `data/items.json` (`stack_max`) ou do
    -- `stack` do próprio item (é o mesmo número que `rules.item_add` usa).
    -- A mensagem tinha o 99 fixo: ao baixar o tecto no JSON dizia um limite
    -- que já não era o real.
    local def = rules.item_def and rules.item_def("bog-moss")
    local cap = rules.stack_max()
    if def and def.kind ~= "resource" then
      local stack = tonumber(def.stack)
      if stack and stack > 0 then
        cap = math.floor(stack)
      end
    end
    local added = rules.item_add("bog-moss", 1) or 0
    if added < 1 then
      viber.toast("A bolsa de musgo está cheia (" .. cap .. ").")
      return
    end
    st.done = true
    once.mark()
    rules.add_xp(8)
    viber.sound("harvest")
    viber.toast("Musgo-do-pântano colhido (+8 XP, +1 bog-moss)")
    viber.topple()
    -- O prop tombado continuava a oferecer "Colher musgo" para sempre.
    viber.despawn_self()
  end
end
