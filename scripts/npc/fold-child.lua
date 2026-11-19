local rules = viber.load("lib/rules.lua")
local opening = viber.load("lib/opening.lua")
local talk = viber.load("ui/say.lua")

local QUEST = "story_first_fold"

function on_update(_dt)
  local st = viber.state()
  local game = viber.game()
  local state = game["q." .. QUEST]
  local stage = game["q." .. QUEST .. ".stage"] or 1
  local dx, dz = opening.offset()
  if not dx then return end
  local x, _, z = viber.position()
  local gate = viber.find("gate_north")
  if not gate then return end
  local found, gx, _, gz = viber.entity_position(gate)
  if not found then return end
  local at_gate = (x - gx - 1.5)^2 + (z - gz + 1.5)^2 < 1.4
  if dx * dx + dz * dz > 25 or state ~= "active" or stage ~= 2 then st.paper = false end
  local legend, label = opening.legend(dx, dz)
  if state == "done" then
    label = "Rever a primeira dobra"
  elseif state ~= "active" then
    label = "Falar com a criança"
  elseif stage == 1 then
    label = "Perguntar pela Atalaia do Vale"
  elseif stage == 3 then
    label = at_gate and "Confirmar o caminho de volta" or "Acompanhar a criança"
  elseif not st.paper then
    label = "Abrir a folha assinada"
  end
  opening.prompt(st, label)
  if viber.menu_open() or talk.active() or viber.ui.focused() ~= nil then return end
  if (state == "done" or (state == "active" and stage == 3)) and not at_gate then
    viber.move_towards(gx + 1.5, gz - 1.5, 1.5)
    return
  end
  if state == "active" and stage == 2 and st.wrong then
    local tx, tz = gx + 5.5, gz - 6
    if (x - tx)^2 + (z - tz)^2 > 1 then
      viber.move_towards(tx, tz, 1.6)
      return
    end
    st.wrong = false
    talk.say("Isto é o curral! Vira a folha e tenta outra vez.")
  end
  if not viber.interacted("e") then return end
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  viber.face_player()
  if state == "done" then
    talk.say(game["campaign.fold"] == "roof" and "Sigo o telhado branco até casa." or "A curva do muro leva-me a casa.")
  elseif state ~= "active" then
    talk.say("O Edmar está à tua espera junto ao salão.")
  elseif stage == 1 then
    talk.say("Assina a Atalaia do Vale [F]. Traz o mapa ao portão e eu experimento-o contigo.")
  elseif stage == 2 then
    if not st.paper then
      st.paper = true
      talk.say("Anda à minha volta para orientar a folha. Escolhe o telhado, o muro ou a seta do curral. Confirma quando quiseres.", 8)
    elseif legend == "yard" then
      st.paper = false
      st.wrong = true
      talk.say("A seta diz para ali. Vem ver!")
    elseif viber.vael.opening_action("draw", legend) then
      st.paper = false
      talk.say("Já vejo o portão. Vem comigo!")
      viber.save_slot(0)
    end
  elseif at_gate and viber.vael.opening_action("arrive") then
    talk.say("Assim já sei voltar. Fico com a folha! Segue para a terra que escolheste.", 6)
    viber.sound("quest_complete")
    viber.save_slot(0)
  end
end
