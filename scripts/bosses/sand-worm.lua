-- bosses/sand-worm.lua: verme do deserto. Toda a máquina vive em
-- lib/fsm.lua (viber.load). Especial: TREMOR — mergulha e rebenta a areia à
-- volta; o aviso cobre um círculo largo, sair dele a correr ou com dash.
local fsm = viber.load("lib/fsm.lua").new{
  kind = "sand-worm",
  hp = 500,
  intro = "guardiao-verme", -- cena de despertar (world/cinematics.xml)
  wander = 1.2, chase = 4.0,
  aggro = 12, deaggro = 17,
  range = 2.6, damage = 20, cooldown = 2.0,
  windup = 0.7, recover = 0.7,
  radius = 8,
  boss = "Verme das Areias",
  poise = 0.14,
  special = {
    every = 3, windup = 1.2, radius = 5.5, mult = 1.4,
    at = "self", clip = "roar,attack", color = "#ff8a2a",
    label = "A areia treme…",
  },
  enrage = { at = 0.35, speed = 1.2, cooldown = 0.7,
             toast = "O Verme enfurece-se — a areia ferve!" },
  -- FASES: aos 65 % passa a ENTERRAR-SE (invisível e intocável) e rebenta a
  -- areia debaixo do herói — a zona laranja avisa 1 s antes; aos 30 % fá-lo
  -- mais vezes e mais depressa.
  phases = {
    { at = 0.65, toast = "O Verme mergulha na areia!", color = "#ff8a2a",
      burrow = { every = 9, under = 1.6, windup = 1.0, radius = 4.5, mult = 1.5,
                 label = "A areia abre-se debaixo de ti!" } },
    { at = 0.3, toast = "A duna inteira mexe-se!", color = "#ff8a2a", speed = 1.1,
      burrow = { every = 6, under = 1.2, windup = 0.85, radius = 5.0, mult = 1.7 } },
  },
  on_chase = function() viber.sound("roar") end,
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
