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
    -- Sem id, `entity_position()` cairia no controller do loot (origem do
    -- mundo) e o número saía na praça. O fallback certo é o HERÓI.
    if id then
      local ok, ex, ey, ez = viber.entity_position(id)
      if ok then
        x, y, z = ex, ey, ez
      end
    end
    if x == nil then
      local pok, px, py, pz = viber.player_position()
      if pok then
        x, y, z = px, py, pz
      end
    end
  end
  -- Sem cadáver nem player (troca de cena) o pagamento continua, mas não há
  -- sítio para o efeito. `(0,0,0)` é a origem do MUNDO (debaixo do terreno e,
  -- dentro de uma casa, o meio da sala): o número e o burst saíam lá. Só se
  -- posiciona o que tem coordenadas; o som cai na origem do controller.
  local has_pos = x ~= nil and y ~= nil and z ~= nil
  local gold = math.random(entry[1], entry[2])
  -- O número mostra o que ENTROU: com o cofre no tecto (ou um save
  -- negativo que a `vault_add` corrige) "+N ouro" mentia.
  local gold_in = rules.vault_add("gold", gold) or 0
  if gold_in > 0 and has_pos then
    viber.damage_number("+" .. gold_in .. " ouro", { color = GOLD_COLOR, x = x, y = y + 1.6, z = z })
  end
  if has_pos then
    viber.burst("sparkle", x, y + 0.8, z, 10)
    viber.sound("coin", { x = x, y = y, z = z })
  else
    -- Sem coordenadas, a origem do controller (a praça) é melhor que (0,0,0).
    viber.sound("coin")
  end
  local potions, antidotes = roll_count(entry[3]), roll_count(entry[4])
  if potions > 0 then
    -- A mochila pára nas 99: "+2 poções" com o stack cheio era um número
    -- sobre um item que não entrou.
    local added = rules.item_add("potion", potions) or 0
    if added > 0 and has_pos then
      local label = added == 1 and " poção" or " poções"
      viber.damage_number("+" .. added .. label, { color = ITEM_COLOR, x = x, y = y + 2.2, z = z })
    end
    if added < potions then
      -- O que não coube também não pode desaparecer em silêncio.
      local missing = potions - added
      local label = missing == 1 and " poção" or " poções"
      local verb = missing == 1 and " não entrou." or " não entraram."
      viber.toast("Mochila cheia: " .. missing .. label .. verb)
    end
  end
  if elite then
    viber.toast("Elite derrotado!")
    if has_pos then
      viber.burst("sparkle", x, y + 1.0, z, 24)
    end
    if math.random() < 0.35 then
      if (rules.item_add("bomb", 1) or 0) > 0 then
        if has_pos then
          viber.damage_number("+1 bomba", { color = ITEM_COLOR, x = x, y = y + 3.0, z = z })
        end
      else
        -- A chance saiu e a bomba perdeu-se em silêncio: avisa que foi a bolsa.
        viber.toast("A bomba do elite não coube na mochila (99).")
      end
    end
  end
  if antidotes > 0 then
    local added = rules.item_add("antidote", antidotes) or 0
    if added > 0 and has_pos then
      local label = added == 1 and " antídoto" or " antídotos"
      viber.damage_number("+" .. added .. label, { color = ITEM_COLOR, x = x, y = y + 2.6, z = z })
    end
    if added < antidotes then
      local missing = antidotes - added
      local label = missing == 1 and " antídoto" or " antídotos"
      local verb = missing == 1 and " não entrou." or " não entraram."
      viber.toast("Mochila cheia: " .. missing .. label .. verb)
    end
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
