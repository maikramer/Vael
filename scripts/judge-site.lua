-- judge-site.lua: o altar que julga o que o herói FEZ.
--
-- Sem texto do jogador: a pista são os props à volta (lobo, poço, chama, na
-- ordem). O primeiro [E] lê as marcas e abre a janela; os [E] seguintes
-- julgam o registo de ações (`lib/actions.lua`) com o modelo a extrair as
-- entidades de cada ação e `lib/judge.lua` a comparar requisitos + ordem.
--@export verdict_id: string = ""

local judge = viber.load("lib/judge.lua")
local rules = viber.load("lib/rules.lua")

local function set_prompt(done)
  viber.set_interaction(done and "O altar está satisfeito" or "Prestar contas ao altar", "e", 3.0)
end

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE
  if not st.ready then
    st.ready = true
    st.id = viber.prop("verdict_id") or ""
    if st.id ~= "" then
      st.done = judge.solved(st.id)
      set_prompt(st.done)
    end
  end
  if st.id == "" or viber.menu_open() then
    return
  end
  -- Herói caído: o modal de morte só abre ao fim dos 2,2 s de `dying`, e
  -- nessa janela o [E] ainda chega aqui. O altar não julga (nem paga o XP/
  -- itens do veredicto) a um cadáver — o mesmo portão do baú e dos POIs.
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return
  end
  local done = judge.solved(st.id)
  if done ~= st.done then
    st.done = done
    set_prompt(done)
  end
  if not viber.interacted("e") then
    return
  end
  if done then
    viber.say("O altar está satisfeito.")
    return
  end
  local rule = judge.get(st.id)
  if not rule then
    return
  end
  if not judge.started(st.id) then
    judge.start(st.id)
    viber.say(rule.intro or "As marcas esperam. O altar pesa o que fizeste.")
    return
  end
  local result = judge.evaluate(st.id)
  local text = (rule.verdicts or {})[result.verdict] or "O altar pesa as tuas ações."
  viber.say(text)
  judge.reward(st.id, result)
  -- Ordem errada: o próximo [E] volta a ler as marcas e abre uma janela
  -- nova (as ações antigas não podem manchar a nova tentativa).
  if result.verdict == "wrong" then
    judge.reset(st.id)
  end
end
