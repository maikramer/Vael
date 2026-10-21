-- Listas do diário: missões, mochila e talentos. A loja é game/shop.lua.
-- A engine não lê o cofre nem o diário de quests.

local rules = viber.load("lib/rules.lua")

local M = {}

local SKILLS = {
  { id = "vitality1", label = "Vitalidade I (+20 HP)", requires = "" },
  { id = "strength1", label = "Força I (+6 dano)", requires = "" },
  { id = "agility1", label = "Agilidade I (+10% velocidade)", requires = "" },
  { id = "precision1", label = "Precisão I (+8% crítico)", requires = "" },
  { id = "vitality2", label = "Vitalidade II (+30 HP)", requires = "vitality1" },
  { id = "strength2", label = "Força II (+10 dano)", requires = "strength1" },
  { id = "agility2", label = "Agilidade II (+15% velocidade)", requires = "agility1" },
  { id = "precision2", label = "Precisão II (+10% crítico)", requires = "precision1" },
}

local BAG = {
  "gold", "wood", "stone", "dark-wood", "bog-moss", "potion", "antidote",
  "bomb", "korok-seed", "nota", "wolf_pelt", "cactus_fiber", "silk_cloth",
  "moss_potion", "iron_axe", "nature_amulet", "blessed_rod", "ancient_relic",
}

local LABELS = {
  gold = "Ouro", wood = "Madeira", stone = "Pedra",
  ["dark-wood"] = "Madeira escura", ["bog-moss"] = "Musgo do pântano",
  potion = "Poção", antidote = "Antídoto", bomb = "Bomba",
  ["korok-seed"] = "Semente de canção", nota = "Nota",
  wolf_pelt = "Pele de lobo", cactus_fiber = "Fibra de cacto",
  silk_cloth = "Seda", moss_potion = "Poção de musgo",
  iron_axe = "Machado de ferro", nature_amulet = "Amuleto da natureza",
  blessed_rod = "Vara abençoada", ancient_relic = "Relíquia ancestral",
}

local function learned(id)
  return viber.game()["skill." .. id] == true
end

function M.learn(id)
  local def
  for _, row in ipairs(SKILLS) do
    if row.id == id then
      def = row
      break
    end
  end
  if not def or learned(id) then
    return
  end
  if def.requires ~= "" and not learned(def.requires) then
    viber.toast("Falta o talento anterior.")
    viber.sound("error")
    return
  end
  if (viber.game().skill_points or 0) < 1 then
    viber.toast("Sem pontos de talento.")
    viber.sound("error")
    return
  end
  viber.game().skill_points = (viber.game().skill_points or 0) - 1
  viber.game()["skill." .. id] = true
  viber.sound("levelup")
  viber.toast(def.label)
end

local function status_text(state)
  if state == "ready" then return "pronta a entregar", "ready" end
  if state == "active" then return "em curso", "active" end
  if state == "done" then return "concluída", "done" end
  return "por aceitar", "pending"
end

local function objective_line(def)
  local obj = def.objective or {}
  local kind = obj.type or ""
  local count = obj.count or 1
  local target = obj.target or ""
  if kind == "kill" then return "derrotar " .. count .. "× " .. target end
  if kind == "collect" then return "reunir " .. count .. "× " .. target end
  if kind == "visit" then return "visitar " .. count .. " locais" end
  return kind
end

local last = {}

local function publish(name, rows)
  local parts = {}
  for i, row in ipairs(rows) do
    parts[i] = (row.id or "") .. (row.status or "") .. (row.count or "") .. (row.progress_text or "")
  end
  local sig = table.concat(parts, "\1")
  if last[name] == sig then
    return
  end
  last[name] = sig
  viber.ui.list(name, rows)
end

function M.refresh()
  local quests = {}
  for _, def in ipairs(rules.quest_defs()) do
    local state = rules.quest_state(def.id)
    local text, class = status_text(state)
    local cur, need = rules.quest_progress(def.id)
    local frac = need > 0 and (cur / need) or 0
    if state == "done" then frac = 1 end
    quests[#quests + 1] = {
      id = def.id,
      title = def.title or def.id,
      status = class,
      status_text = text,
      objective = objective_line(def),
      biome_name = def.biome or "",
      progress = string.format("%.2f", frac),
      progress_text = cur .. "/" .. need,
    }
  end
  publish("quests", quests)

  local bag = {}
  for _, id in ipairs(BAG) do
    local count = rules.vault_get(id)
    if count > 0 or id == "gold" or id == "wood" or id == "stone" then
      bag[#bag + 1] = {
        id = id,
        label = LABELS[id] or id,
        count = tostring(count),
        kind = (id == "gold" or id == "wood" or id == "stone") and "resource" or "item",
        status = count == 0 and "empty" or "",
      }
    end
  end
  publish("bag", bag)

  local skills = {}
  for _, def in ipairs(SKILLS) do
    local status, text = "locked", "bloqueada"
    if learned(def.id) then
      status, text = "done", "aprendida"
    elseif def.requires == "" or learned(def.requires) then
      status, text = "ready", "disponível"
    end
    skills[#skills + 1] = {
      id = def.id,
      label = def.label,
      status = status,
      status_text = text,
      requires = def.requires == "" and "sem requisitos" or def.requires,
    }
  end
  publish("skills", skills)
end

return M
