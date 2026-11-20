-- enemies/shade.lua: criatura rápida do ermo (sem anti-stuck nem
-- aggro-chain). Toda a máquina vive em lib/fsm.lua.
local fsm = vael.load("lib/fsm.lua").new{
  kind = "shade",
  proto = "mob-shade",
  nocturnal = true, -- de noite vê mais longe e bate mais
  wander = 1.4, chase = 4.2,
  aggro = 12, deaggro = 17,
  range = 1.8, damage = 14, cooldown = 2.0,
  windup = 0.55,
  special = { every = 3, at = "player", radius = 1.55, windup = 0.85, mult = 0.6,
              clip = "attack", sound = "whoosh", fx = "magic", color = "#c7a2ff", cue = "echo_mark",
              echo = { radius = 1.55, windup = 0.95, mult = 0.65, recover = 1.25,
                       clip = "attack", sound = "whoosh", fx = "magic", color = "#c7a2ff", cue = "echo_mark" } },
  radius = 5,
  anti_stuck = false,
}

function on_update(dt)
  fsm.update(dt)
end
