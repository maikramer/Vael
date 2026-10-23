local rules = viber.load("lib/rules.lua")
-- poi/survey-tower.lua — Atalaia de agrimensura (a "torre sheikah" de Vael).
--
-- Ao chegar ao topo e SONDAR a terra [E], a atalaia revela o bioma associado
-- (prop-biome): os marcos da Nota desse bioma acendem na bússola/minimapa e
-- passam a aceitar [F] (medir/assinar) e viagem rápida [G]. A revelação vive
-- na ENGINE (travel::RevealedBiomes) e viaja no save — o script só decide
-- QUANDO revelar e conta a história. Re-sondar depois é inócuo (e dizido).
--
-- Ficção (GDD "Discordia: A Nota do Mundo"): os cartógrafos do If mantêm
-- atalaias onde a Nota soa mais alto; do topo, o mundo "mede-se" — o mesmo
-- verbo dos marcos ([F] "Medido e assinado").
--
--@export biome: string = "tower"          -- bioma revelado (tower = só a atalaia)
--@export label: string = "Atalaia"        -- nome no toast
--@export xp: int(0..) = 40                -- XP da primeira sondagem
--@export prompt: string = "Sondar a terra"
--@export range: number(0.5..10) = 2.6     -- alcance do [E] (m)

-- Herói caído: o modal de morte só abre ao fim de 2,2 s de `dying`, e nessa
-- janela o [E] ainda chega aqui. A atalaia não revela o bioma (nem paga XP)
-- a um cadáver.
local function hero_down()
  local alive, hp = rules.player_hp()
  return alive and (hp or 0) <= 0
end

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (o top-level é partilhado)
  if not st.ready then
    st.ready = true
    st.done = rules.biome_revealed(viber.prop("biome"))
    viber.set_interaction(st.done and "A atalaia já falou" or viber.prop("prompt"), "e", viber.prop("range"))
  end
  -- Relê o bioma a cada frame: carregar um save anterior reabre a atalaia.
  -- O prompt acompanha o ESTADO (antes ficava "já falou" para sempre mesmo
  -- depois de um load que apagasse `biome.*`).
  local done = rules.biome_revealed(viber.prop("biome"))
  if done ~= st.done then
    st.done = done
    -- Estado por ENTIDADE: um load que apague `biome.*` reabre a atalaia, mas
    -- `st.reminded` continuava verdadeiro e o aviso de "já falou" nunca mais
    -- aparecia depois da nova sondagem.
    st.reminded = false
    viber.set_interaction(done and "A atalaia já falou" or viber.prop("prompt"), "e", viber.prop("range"))
  end
  if st.done then
    -- A atalaia já falou, mas o herói caído também não precisa de ouvi-la:
    -- o modal de morte só abre 2,2 s depois e o [E] ainda chega aqui.
    if viber.menu_open() or hero_down() then return end
    if viber.interacted("e") and not st.reminded then
      st.reminded = true
      viber.toast("A " .. viber.prop("label") .. " já cartografou esta terra.")
    end
    return
  end
  if viber.menu_open() or hero_down() then return end
  if viber.interacted("e") then
    local p = viber.props()
    rules.reveal_biome(p.biome)
    if p.xp > 0 then
      rules.add_xp(p.xp)
    end
    local _, x, y, z = viber.entity_position()
    viber.burst("magic", x, y + 1.4, z, 24)
    viber.sound("quest_complete")
    -- Com `xp = 0` autorado o toast dizia "(+0 XP)".
    local xp_note = p.xp > 0 and (" (+" .. p.xp .. " XP)") or ""
    viber.toast("A " .. p.label .. " acorda: a Nota do bioma entra na tua bússola!" .. xp_note)
    viber.log("atalaia revelou bioma: " .. p.biome)
  end
end
