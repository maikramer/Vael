local rules = vael.load("lib/rules.lua")
-- bog-moss.lua: tufo de musgo-do-pântano colhível ([J]) — fonte do
-- item `bog-moss` (objetivo collect ×10 da quest do pântano).
local once = vael.load("lib/once.lua")
function on_update(dt)
  local st = vael.state() -- POR ENTIDADE
  if not st.ready then
    st.ready = true
    -- Sem marca no save, gravar e carregar replantava o musgo e a quest farmava.
    if once.taken() then
      vael.despawn_self()
      return
    end
    vael.set_interaction("Colher musgo", "j", 2.5)
  end
  -- Relê a marca: um Load na mesma cena não recria a entidade.
  if once.taken() then
    if not st.done then
      st.done = true
      vael.despawn_self()
    end
    return
  end
  if vael.menu_open() then return end
  -- O `menu_open` não inclui o ecrã de morte: o herói caído ainda colhia.
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  if vael.interacted("j") and not st.done then
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
      vael.toast("A bolsa de musgo está cheia (" .. cap .. ").")
      return
    end
    st.done = true
    once.mark()
    rules.add_xp(8)
    local ok, x, y, z = vael.entity_position()
    if ok and vael.vael and vael.vael.collect_burst then
      vael.vael.collect_burst("bog-moss", x, y + 0.35, z, 4)
    end
    if ok and vael.burst then
      vael.burst("leaves", x, y + 0.35, z, 14)
    end
    if vael.play_clip and vael.player_id then
      local pid = vael.player_id()
      if pid then vael.play_clip("gather", { speed = 1.15, id = pid }) end
    end
    vael.sound("harvest")
    vael.toast("Musgo-do-pântano colhido (+8 XP, +1 bog-moss)")
    vael.topple()
    -- O prop tombado continuava a oferecer "Colher musgo" para sempre.
    vael.despawn_self()
  end
end
