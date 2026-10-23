-- anvil.lua: prompt de interação + efeito (portado do TS; colheita pesada chega
-- com o Destructible/Inventory da Fase 3).
local rules = viber.load("lib/rules.lua")
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
  -- O `menu_open` não inclui o ecrã de morte: durante os 2,2 s de `dying` o
  -- [E] ainda chega aqui e a bigorna ficava marcada como inspecionada por um
  -- cadáver — a interação de uma vez consumia-se sem o jogador a ver.
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then return end
  if viber.interacted("e") and not st.done then
    st.done = true
    once.mark()
    set_prompt(true)
    viber.toast("A bigorna do ferreiro ainda está quente.")
  end
end
