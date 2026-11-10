local rules = viber.load("lib/rules.lua")
-- enemies/bogling-hexer.lua: bogling FEITICEIRO do Pântano — arquétipo de
-- zona. Mantém 5–13 m e marca a roxo o chão onde o herói ESTÁ; 1,2 s depois a
-- zona rebenta e envenena quem ficou. Frágil: fecha a distância e cai depressa
-- (encurralado, defende-se à unhada).
-- Conta como "bogling" nas missões (prefixo `bogling-`). Máquina: lib/fsm.lua.
local fsm = viber.load("lib/fsm.lua").new{
  kind = "bogling-hexer",
  proto = "mob-bogling-hexer",
  hp = 80,
  wander = 1.0, chase = 3.0,
  aggro = 14, deaggro = 20,
  range = 1.5, damage = 9, cooldown = 1.6,
  windup = 0.6,
  radius = 4,
  anti_stuck = false,
  weak = { burn = 2.0 },
  caster = { range = 13, min = 5, cooldown = 4.2, windup = 1.2, radius = 2.6,
             mult = 1.5, color = "#6fdc5a",
             on_hit = function() rules.apply_status("venom", 4) end },
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
