-- enemies/goblin.lua: FSM wander/chase com aggro-chain e anti-stuck.
-- Toda a máquina vive em lib/fsm.lua (vael.load).
local fsm = vael.load("lib/fsm.lua").new{
  kind = "goblin",
  proto = "mob-goblin",
  wander = 1.6, chase = 3.2,
  aggro = 10, deaggro = 15,
  range = 1.6, damage = 10, cooldown = 1.4,
  windup = 0.45,
  special = { every = 3, windup = 1.0, radius = 2.8, mult = 1.25, recover = 1.1 },
  radius = 8,
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
