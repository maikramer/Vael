-- enemies/bogling.lua: criatura lenta do pântano (sem anti-stuck nem
-- aggro-chain — wander curto). Toda a máquina vive em lib/fsm.lua.
local fsm = vael.load("lib/fsm.lua").new{
  kind = "bogling",
  proto = "mob-bogling",
  weak = { burn = 2.0 }, -- lodo seco arde bem
  wander = 1.0, chase = 2.8,
  aggro = 8, deaggro = 13,
  range = 1.5, damage = 8, cooldown = 1.5,
  windup = 0.6,
  special = { every = 3, windup = 1.1, inner = 1.65, radius = 3.6, mult = 1.15,
              recover = 1.1, clip = "attack", sound = "whoosh", color = "#d7974d", cue = "safe_center" },
  radius = 4,
  anti_stuck = false,
}

function on_update(dt)
  fsm.update(dt)
end
