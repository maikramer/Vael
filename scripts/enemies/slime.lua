-- enemies/slime.lua: a criatura tutorial — lenta e inofensiva.
-- Toda a máquina vive em lib/fsm.lua (vael.load).
local fsm = vael.load("lib/fsm.lua").new{
  kind = "slime",
  proto = "mob-slime",
  weak = { shock = 1.5 },
  wander = 0.7, chase = 2.0,
  aggro = 6, deaggro = 11,
  range = 1.4, damage = 6, cooldown = 2.0,
  windup = 0.7, recover = 0.6, -- tutorial: aviso longo, punição generosa
  special = { every = 3, at = "player", windup = 1.15, radius = 1.55, mult = 0.85,
              recover = 1.15, clip = "attack", sound = "whoosh", cue = "leave_mark" },
  radius = 3,
  anti_stuck = false,
  on_first = function() vael.log("slime ativo: FSM wander/chase rodando") end,
}

function on_update(dt)
  fsm.update(dt)
end
