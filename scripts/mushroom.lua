local rules = viber.load("lib/rules.lua")
-- mushroom.lua: colheita de cogumelo (+XP). Sai do mundo ao ser colhido.
--@export xp: int(0..) = 5                  -- XP da colheita
--@export label: string = "Cogumelo"        -- nome no prompt e no toast
--@export key: enum(j, e, f) = j            -- tecla de colheita
--@export range: number(0.5..10) = 2.5      -- alcance (m)

local once = viber.load("lib/once.lua")

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (o top-level é partilhado)
  local key = viber.prop("key")
  if not st.ready then
    st.ready = true
    -- O despawn não fica no XML: um load trazia o cogumelo de volta, com XP.
    if once.taken() then
      viber.despawn_self()
      return
    end
    viber.set_interaction("Colher " .. string.lower(viber.prop("label")), key, viber.prop("range"))
  end
  -- Relê a marca: um Load na MESMA cena não recria a entidade, e o
  -- `st.done` em cache deixava colher (ou bloqueava) outra vez.
  if once.taken() then
    if not st.done then
      st.done = true
      viber.despawn_self()
    end
    return
  end
  if viber.menu_open() then return end
  -- O `menu_open` não inclui o ecrã de morte: um herói caído continuava a
  -- colher (XP) junto ao sítio onde caiu.
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  if viber.interacted(key) and not st.done then
    st.done = true
    once.mark()
    local xp = viber.prop("xp")
    rules.add_xp(xp)
    -- A colheita de árvores/pedras confirma com som; o cogumelo desaparecia
    -- em silêncio (só o toast).
    local ok, x, y, z = viber.entity_position()
    if ok and viber.vael and viber.vael.collect_burst then
      viber.vael.collect_burst("mushroom", x, y + 0.35, z, 4)
    end
    if ok and viber.burst then
      viber.burst("sparkle", x, y + 0.35, z, 12)
    end
    if viber.play_clip and viber.player_id then
      local pid = viber.player_id()
      if pid then viber.play_clip("gather", { speed = 1.15, id = pid }) end
    end
    viber.sound("harvest")
    viber.toast(viber.prop("label") .. " colhido (+" .. xp .. " XP)")
    viber.despawn_self() -- colhido: sai do mundo (o prompt desaparece com ele)
  end
end
