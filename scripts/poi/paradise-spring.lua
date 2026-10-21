local rules = viber.load("lib/rules.lua")
-- poi/paradise-spring.lua: objeto místico (portado do TS) — prompt quando perto, leitura dá
-- XP uma única vez (a marca vive no save — state() renasce vazio no load).
local once = viber.load("lib/once.lua")
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    st.read = once.taken()
    viber.set_interaction("Banhar-se na fonte", "e", 3.5)
  end
  if viber.menu_open() then
    return
  end
  if viber.interacted("e") and not st.read then
    local alive, hp = rules.player_hp()
    -- Consumir a fonte de uma vez enquanto o herói está caído dava a cura
    -- e o respawn por cima.
    if alive and hp <= 0 then
      return
    end
    st.read = true
    once.mark()
    viber.set_interaction("A fonte está calma", "e", 3.5)
    local ok, _, max_hp = rules.player_hp()
    local healed = 40
    if ok then
      healed = math.min(40, math.max(0, (max_hp or 0) - (hp or 0)))
    end
    if healed >= 1 then
      rules.heal_player(healed)
    end
    rules.add_xp(35)
    if healed >= 1 then
      viber.toast("A água da fonte restaura o corpo e a mente (+" .. math.floor(healed) .. " HP, +35 XP).")
    else
      viber.toast("A água da fonte clareia a mente (+35 XP).")
    end
  end
end
