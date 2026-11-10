local rules = viber.load("lib/rules.lua")
-- bosses/boss.lua: ogro dos Picos Gelados com gating (loop 6): dorme
-- enquanto houver OUTROS hostis vivos na banda sul. Toda a máquina vive em
-- lib/fsm.lua (viber.load). Chefe final: poise alto, SISMO a cada 3 golpes
-- (zona larga à volta dele) e fúria abaixo de 40 % de vida.
local fsm = viber.load("lib/fsm.lua").new{
  kind = "boss",
  hp = 1200,
  intro = "guardiao-ogro", -- cena de despertar (world/cinematics.xml)
  wander = 1.2, chase = 3.4,
  aggro = 14, deaggro = 19,
  range = 2.5, damage = 22, cooldown = 1.5,
  windup = 0.75, recover = 0.7,
  radius = 9,
  boss = "Ogro dos Picos",
  poise = 0.16,
  special = {
    every = 3, windup = 1.2, radius = 5.0, mult = 1.6,
    at = "self", clip = "swordheavy,punch,attack", color = "#ff4a2a",
    label = "O Ogro ergue os punhos…",
  },
  enrage = { at = 0.4, speed = 1.2, cooldown = 0.7,
             toast = "O Ogro entra em FÚRIA!" },
  -- FASES: aos 66 % ganha a INVESTIDA (corredor vermelho: sai da linha); aos
  -- 33 % o sismo alarga e gela quem apanha.
  phases = {
    { at = 0.66, toast = "O Ogro baixa a cabeça — vai investir!",
      charge = { min = 5, max = 14, cooldown = 7, windup = 1.0, speed = 11,
                 mult = 1.8, recover = 1.6 } },
    { at = 0.33, toast = "O gelo dos Picos sobe-lhe pelos punhos!", color = "#8cd9ff",
      charge = { min = 5, max = 14, cooldown = 5, windup = 0.9, speed = 12,
                 mult = 1.9, recover = 1.5 },
      special = {
        every = 3, windup = 1.2, radius = 6.5, mult = 1.7,
        at = "self", clip = "swordheavy,punch,attack", color = "#8cd9ff",
        label = "O Ogro ergue os punhos — o chão gela!",
        on_hit = function() rules.apply_status("frost", 4) end,
      } },
  },
  gate = function()
    -- Exclui o PRÓPRIO ogro: o gate corre ANTES de ele se registar, por isso
    -- `<= 1` abria com um inimigo ainda vivo na serra. `except_kind` também
    -- cobre um registo antigo do chefe (hot-reload/estado que sobreviveu).
    return rules.alive_in_region(2, "boss") == 0
  end,
  awake_toast = "Um rugido ecoa dos Picos Gelados — o ogro despertou!",
}

function on_player_attack(px, pz)
  fsm.alert()
end

function on_update(dt)
  fsm.update(dt)
end
