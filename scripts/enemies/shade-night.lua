-- enemies/shade-night.lua: SOMBRA DA NOITE — só existe entre as 20h e as 5h.
-- Nasce perto do herói fora da vila (game/spawns.lua), é mais rápida e bate
-- mais do que a sombra comum, e desfaz-se ao amanhecer sem deixar nada.
-- Conta como "shade" nas missões (prefixo `shade-`). Máquina: lib/fsm.lua.
local fsm = viber.load("lib/fsm.lua").new{
  kind = "shade-night",
  hp = 120,
  wander = 1.6, chase = 4.6,
  aggro = 14, deaggro = 22,
  range = 1.8, damage = 15, cooldown = 1.7,
  windup = 0.5,
  radius = 6,
  anti_stuck = false,
  nocturnal = true,
  fades_at_dawn = true,
  respawn = false,
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
