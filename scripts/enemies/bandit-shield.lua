-- enemies/bandit-shield.lua: bandido de ESCUDO do Ermo — arquétipo de
-- bloqueio. De frente, os golpes normais ressaltam (20 % do dano); o remate
-- da chain (3.º golpe) parte a guarda e atordoa, as costas não têm escudo, e
-- depois de atacar fica aberto durante o `recover`.
-- Conta como "bandit" nas missões (prefixo `bandit-`). Máquina: lib/fsm.lua.
local fsm = viber.load("lib/fsm.lua").new{
  kind = "bandit-shield",
  proto = "mob-bandit-shield",
  hp = 150,
  wander = 1.5, chase = 3.3,
  aggro = 10, deaggro = 15,
  range = 2.0, damage = 13, cooldown = 1.8,
  windup = 0.55, recover = 0.75, -- a janela para o apanhar aberto
  radius = 6,
  shield = { arc = 0.3, reduce = 0.8 },
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
