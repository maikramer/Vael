-- anvil.lua: prompt de interação + efeito (portado do TS; colheita pesada chega
-- com o Destructible/Inventory da Fase 3).
local once = viber.load("lib/once.lua")
local PROMPT, DONE_PROMPT = "Inspecionar a bigorna", "A bigorna está fria"
local function set_prompt(done)
  viber.set_interaction(done and DONE_PROMPT or PROMPT, "e", 2.5)
end
function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    st.done = once.taken()
    set_prompt(st.done)
  end
  local taken = once.taken()
  if taken ~= st.done then
    st.done = taken
    set_prompt(taken)
  end
  if viber.menu_open() then return end
  if viber.interacted("e") and not st.done then
    st.done = true
    once.mark()
    set_prompt(true)
    viber.toast("A bigorna do ferreiro ainda está quente.")
  end
end
