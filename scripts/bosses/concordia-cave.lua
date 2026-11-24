--@export cave_kind: string = ""

local fsm = vael.load("lib/fsm.lua")
local guards = {
  ["cave-wolf"] = { name = "Lobo do Covil", hp = 155, chase = 4.1, range = 1.8, damage = 14 },
  ["cave-tomb"] = { name = "Sentinela do Túmulo", hp = 205, chase = 2.8, range = 2.1, damage = 17 },
  ["cave-ice"] = { name = "Vigia da Gruta", hp = 190, chase = 3.25, range = 1.9, damage = 16 },
}
local controllers = {}
for kind, cfg in pairs(guards) do
  controllers[kind] = fsm.new{
    kind = kind, boss = cfg.name, hp = cfg.hp, chase = cfg.chase,
    range = cfg.range, damage = cfg.damage, aggro = 7, deaggro = 14, radius = 3,
    wander = 0, windup = 0.85, cooldown = 1.8, recover = 0.55, poise = 0.35,
  }
end

function on_update(dt)
  local controller = controllers[vael.prop("cave_kind")]
  if controller then controller.update(dt) end
end

function on_player_attack()
  local controller = controllers[vael.prop("cave_kind")]
  if controller then controller.alert() end
end
