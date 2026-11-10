-- anvil.lua: a bigorna do Bram abre a FORJA (separador do diário).
-- As receitas vivem em `data/recipes.json` e a regra em `lib/craft.lua`; este
-- script só publica o prompt e abre o separador certo.
local rules = viber.load("lib/rules.lua")

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    viber.set_interaction("Forjar na bigorna", "e", 2.5)
  end
  if viber.menu_open() then return end
  -- O `menu_open` não inclui o ecrã de morte: durante os 2,2 s de `dying` o
  -- [E] ainda chega aqui e um cadáver abria a forja.
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then return end
  if viber.interacted("e") then
    viber.sound("mine_hit")
    viber.ui.open("menu", true)
    viber.ui.select_tab("menu", "forge")
  end
end
