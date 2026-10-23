local rules = viber.load("lib/rules.lua")
-- poi/sun-relic.lua: objeto místico (portado do TS) — prompt quando perto, leitura dá
-- XP uma única vez (a marca vive no save — state() renasce vazio no load).
local once = viber.load("lib/once.lua")
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    st.read = once.taken()
    viber.set_interaction(st.read and "A relíquia está quente" or "Tocar a relíquia", "e", 3.0)
  end
  local taken = once.taken()
  if taken ~= st.read then
    st.read = taken
    viber.set_interaction(st.read and "A relíquia está quente" or "Tocar a relíquia", "e", 3.0)
  end
  if viber.menu_open() then return end
  -- Herói caído: o modal de morte só abre ao fim de 2,2 s de `dying` e nessa
  -- janela o [E] ainda chega aqui. Um POI não paga XP a um cadáver.
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then return end
  if viber.interacted("e") and not st.read then
    st.read = true
    once.mark()
    viber.set_interaction("A relíquia está quente", "e", 3.0)
    rules.add_xp(30)
    viber.toast("A relíquia do sol aquece as mãos (+30 XP).")
  end
end
