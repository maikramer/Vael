local rules = viber.load("lib/rules.lua")
-- game/loot.lua: DESPOJOS dos abates — o jogo decide, em Lua, o que cai.
--
-- A engine já dá XP, cadáver e o progresso de quest no abate; o ouro vinha
-- só de quests e da venda de recursos, pelo que caçar não pagava nada.
-- Este controller ouve os eventos `kill` (`viber.events()`, fila própria)
-- e paga no sítio do cadáver: ouro por tipo de criatura (intervalo) e uma
-- CHANCE de consumível — poção para toda a gente, antídoto sobretudo dos
-- venenosos. Os chefes pagam uma bolsa grande e sempre poções.
--
-- Host: `world/gameplay.xml` (`tag="always-active"` — abates longe do raio
-- de 45 m de ativação continuam a pagar).

-- kind (nome do script da criatura) → { ouro mín, ouro máx, p(poção), p(antídoto) }
local TABLE = {
  slime      = { 1, 2, 0.04, 0.00 },
  wolf       = { 2, 4, 0.06, 0.00 },
  bogling    = { 2, 4, 0.06, 0.08 },
  goblin     = { 3, 6, 0.08, 0.00 },
  scorpion   = { 3, 5, 0.05, 0.18 },
  shade      = { 4, 7, 0.10, 0.00 },
  bandit     = { 5, 9, 0.10, 0.05 },
  ["bandit-archer"] = { 6, 10, 0.12, 0.05 },
  -- Chefes: bolsa + 2 poções garantidas (p > 1 = garantido, parte inteira).
  witch        = { 60, 90, 2.0, 1.0 },
  ["sand-worm"]  = { 70, 100, 2.0, 1.0 },
  ["bog-warden"] = { 80, 110, 2.0, 2.0 },
  boss         = { 150, 200, 3.0, 1.0 },
}

local M = {}

local GOLD_COLOR = "#ffd35a"
local ITEM_COLOR = "#8fe3a0"

-- Rolagem: `p` ≥ 1 dá floor(p) garantidos + a fração como chance extra.
local function roll_count(p)
  local n = math.floor(p)
  if math.random() < p - n then n = n + 1 end
  return n
end

function M.pay(kind, id, x, y, z)
  local entry = TABLE[kind]
  if not entry then return end
  -- Elites (lib/fsm.lua regista-os): ouro ×3, poção garantida e chance de
  -- bomba — vale a pena o risco.
  local elites = viber.game().elites
  -- `elites[nil]` derruba o script no frame do abate e o ouro não cai.
  local elite = id ~= nil and elites and elites[id]
  if elite then
    elites[id] = nil
    entry = { entry[1] * 3, entry[2] * 3, entry[3] + 1, entry[4] }
  end
  if x == nil then
    local ok, ex, ey, ez = viber.entity_position(id)
    if ok then
      x, y, z = ex, ey, ez
    else
      local pok, px, py, pz = viber.player_position()
      if pok then
        x, y, z = px, py, pz
      end
    end
  end
  -- Sem cadáver nem player (troca de cena) o pagamento continua; só não há
  -- sítio para o número. `nil + 1.6` rebentava depois do ouro já creditado.
  if x == nil or y == nil or z == nil then
    x, y, z = x or 0, y or 0, z or 0
  end
  local gold = math.random(entry[1], entry[2])
  rules.vault_add("gold", gold)
  viber.damage_number("+" .. gold .. " ouro", { color = GOLD_COLOR, x = x, y = y + 1.6, z = z })
  viber.burst("sparkle", x, y + 0.8, z, 10)
  viber.sound("coin", { x = x, y = y, z = z })
  local potions, antidotes = roll_count(entry[3]), roll_count(entry[4])
  if potions > 0 then
    rules.item_add("potion", potions)
    viber.damage_number("+" .. potions .. " poção", { color = ITEM_COLOR, x = x, y = y + 2.2, z = z })
  end
  if elite then
    viber.toast("Elite derrotado!")
    viber.burst("sparkle", x, y + 1.0, z, 24)
    if math.random() < 0.35 then
      rules.item_add("bomb", 1)
      viber.damage_number("+1 bomba", { color = ITEM_COLOR, x = x, y = y + 3.0, z = z })
    end
  end
  if antidotes > 0 then
    rules.item_add("antidote", antidotes)
    viber.damage_number("+" .. antidotes .. " antídoto", { color = ITEM_COLOR, x = x, y = y + 2.6, z = z })
  end
end

function on_update(dt)
  for _, ev in ipairs(viber.events()) do
    if ev.type == "kill" then
      M.pay(ev.name, ev.entity)
    end
  end
end

return M
