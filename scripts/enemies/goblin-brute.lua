-- enemies/goblin-brute.lua: o goblin BRUTO dos Picos — arquétipo de
-- INVESTIDA. A meia distância trava, marca o corredor a vermelho e arranca em
-- linha recta: sai do corredor (ou dá dash [C]) e ele passa ao lado, exposto
-- durante 1,4 s. Colado, bate devagar e com força.
-- Conta como "goblin" nas missões (prefixo `goblin-`). Máquina: lib/fsm.lua.
local fsm = viber.load("lib/fsm.lua").new{
  kind = "goblin-brute",
  proto = "mob-goblin-brute",
  hp = 220,
  wander = 1.3, chase = 3.0,
  aggro = 13, deaggro = 19,
  range = 2.0, damage = 16, cooldown = 1.9,
  windup = 0.65, recover = 0.6,
  radius = 7,
  charge = { min = 5, max = 12, cooldown = 6, windup = 0.9, speed = 10,
             mult = 1.7, recover = 1.4 },
  on_first = function() viber.set_scale(1.25) end,
  on_chase = function() viber.sound("roar") end,
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
