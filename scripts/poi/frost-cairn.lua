local rules = viber.load("lib/rules.lua")
-- poi/frost-cairn.lua: objeto místico (portado do TS) — prompt quando perto, leitura dá
-- XP uma única vez (a marca vive no save — state() renasce vazio no load).
local once = viber.load("lib/once.lua")
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    st.read = once.taken()
    viber.set_interaction(st.read and "Mojão já lido" or "Ler o mojão gelado", "e", 3.0)
  end
  local taken = once.taken()
  if taken ~= st.read then
    st.read = taken
    viber.set_interaction(st.read and "Mojão já lido" or "Ler o mojão gelado", "e", 3.0)
  end
  if viber.menu_open() then return end
  if viber.interacted("e") and not st.read then
    st.read = true
    once.mark()
    viber.set_interaction("Mojão já lido", "e", 3.0)
    rules.add_xp(60)
    viber.toast("“As pedras empilhadas apontam para o oeste.” (+60 XP)")
  end
end
