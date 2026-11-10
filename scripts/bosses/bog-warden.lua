local rules = viber.load("lib/rules.lua")
-- bosses/bog-warden.lua: guardião do pântano. Toda a máquina vive em
-- lib/fsm.lua (viber.load). Especial: GOLPE DE LODO — pancada no chão que
-- envenena quem ficar na zona (o antídoto [2] ganha uso).
local fsm = viber.load("lib/fsm.lua").new{
  kind = "bog-warden",
  hp = 600,
  intro = "guardiao-lodo", -- cena de despertar (world/cinematics.xml)
  wander = 1.0, chase = 3.0,
  aggro = 13, deaggro = 18,
  range = 2.4, damage = 16, cooldown = 1.6,
  windup = 0.7, recover = 0.6,
  radius = 9,
  boss = "Guardião do Lodo",
  poise = 0.13,
  special = {
    every = 3, windup = 1.1, radius = 4.8, mult = 1.3,
    at = "self", clip = "swordheavy,attack", color = "#6fdc5a",
    on_hit = function() rules.apply_status("venom", 6) end,
  },
  enrage = { at = 0.4, speed = 1.2, cooldown = 0.75,
             toast = "O Guardião ruge — o lodo sobe-lhe pelos braços!" },
  -- FASES: aos 60 % chama três boglings do lodo; aos 30 % chama um
  -- feiticeiro e o golpe de lodo passa a cair onde o herói está.
  phases = {
    { at = 0.6, toast = "O lodo mexe-se — os afogados respondem ao Guardião!",
      color = "#6fdc5a", summon = { proto = "mob-bogling", count = 3 } },
    { at = 0.3, toast = "O Guardião conta mais um afogado: tu.", color = "#6fdc5a",
      summon = { proto = "mob-bogling-hexer", count = 1 },
      special = {
        every = 2, windup = 1.0, radius = 4.0, mult = 1.4,
        at = "player", clip = "swordheavy,attack", color = "#6fdc5a",
        on_hit = function() rules.apply_status("venom", 7) end,
      } },
  },
  on_chase = function() viber.sound("roar") end,
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
