-- Listas do diário: missões, mochila e talentos. A loja é game/shop.lua.
-- A engine não lê o cofre nem o diário de quests.

local rules = viber.load("lib/rules.lua")

local M = {}

-- Skills e itens vêm do conteúdo (`data/progression.json` e
-- `data/items.json`, via `rules`), não de tabelas duplicadas aqui.

local BIOME_LABEL = {
  city = "Discórdia",
  dark_forest = "Floresta Sombria",
  ["dark-forest"] = "Floresta Sombria",
  desert = "Ermo",
  swamp = "Pântano",
  frozen_peaks = "Picos Gelados",
  ["frozen-peaks"] = "Picos Gelados",
}

local function learned(id)
  return viber.game()["skill." .. id] == true
end

-- O diário é alimentado pelo UiRoot: sem posição, o erro/level-up tocava na
-- origem da UI (praça), não no herói.
local function sfx(name)
  local has, x, y, z = viber.player_position()
  if has then
    viber.sound(name, { x = x, y = y, z = z })
  else
    viber.sound(name)
  end
end

function M.learn(id)
  local def = rules.skill(id)
  if not def or learned(id) then
    return
  end
  local requires = def.requires or ""
  if requires ~= "" and not learned(requires) then
    viber.toast("Falta o talento anterior.")
    sfx("error")
    return
  end
  if (viber.game().skill_points or 0) < 1 then
    viber.toast("Sem pontos de talento.")
    sfx("error")
    return
  end
  viber.game().skill_points = (viber.game().skill_points or 0) - 1
  viber.game()["skill." .. id] = true
  sfx("levelup")
  viber.toast(def.label)
end

local function status_text(state)
  if state == "ready" then return "pronta a entregar", "ready" end
  if state == "active" then return "em curso", "active" end
  if state == "done" then return "concluída", "done" end
  return "por aceitar", "pending"
end

local TARGET_LABEL = {
  wolf = "lobos", bandit = "bandidos", goblin = "goblins", shade = "sombras",
  slime = "limos", scorpion = "escorpiões", bogling = "boglings",
  witch = "a Bruxa do Bosque", ["sand-worm"] = "o Verme da Areia",
  ["bog-warden"] = "o Guardião do Lodo", boss = "o Ogro dos Picos",
  stone = "pedras", wood = "madeira", ["dark-wood"] = "madeira escura",
  ["bog-moss"] = "musgo do pântano", nota = "Notas",
}

local function target_label(target)
  return TARGET_LABEL[target] or target
end

local function objective_line(def, state)
  -- Pronta: o que falta é o relato de uma palavra ao dador.
  if state == "ready" and type(def.parley) == "table" then
    local p = def.parley.report
    local ex = p and p.example
    if ex and ex ~= "" then
      return "diz ao NPC: «" .. ex .. "»"
    end
  end
  local obj = def.objective or {}
  local kind = obj.type or ""
  local count = obj.count or 1
  local target = target_label(obj.target or "")
  if kind == "kill" then return "derrotar " .. count .. "× " .. target end
  if kind == "collect" then return "reunir " .. count .. "× " .. target end
  if kind == "visit" then return "visitar " .. count .. " locais" end
  return kind
end

local last = {}

local function publish(name, rows)
  local parts = {}
  for i, row in ipairs(rows) do
    -- Todos os campos RENDERIZADOS: label/bioma/objetivo podem mudar (locale,
    -- progresso) sem id/status/count mudarem, e a lista ficava velha.
    -- `requires` é o requisito mostrado nos talentos e `kind` decide a classe
    -- da célula da mochila; nenhum dos dois estava na assinatura.
    parts[i] = table.concat({
      row.id or "", row.title or "", row.label or "", row.status or "",
      row.status_text or "", row.objective or "", row.biome_name or "",
      row.count or "", row.progress or "", row.progress_text or "",
      row.tracked or "", row.requires or "", row.kind or "",
    }, "\2")
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
  local tracked = viber.game().tracked_quest
  for _, def in ipairs(rules.quest_defs()) do
    local state = rules.quest_state(def.id)
    -- O diário listava as 28 missões desde o início (o estado vazio era
    -- inalcançável e o jogador via spoilers dos chefes/finais).
    if state ~= "not_taken" and state ~= "unknown" then
      local text, class = status_text(state)
      local cur, need = rules.quest_progress(def.id)
      local frac = need > 0 and (cur / need) or 0
      local progress_text = cur .. "/" .. need
      if state == "done" then
        frac = 1
        -- Entregar uma quest de collect esvazia o cofre e o progresso
        -- voltava a 0/N com a barra cheia.
        progress_text = need .. "/" .. need
      end
      -- A missão seguida tem de se ler na lista: o clique já a alternava
      -- (travel.track) mas o diário reabria sem qualquer marca, e o jogador
      -- só tinha o toast de 2 s para saber o que estava a seguir.
      local following = tracked == def.id
        and (state == "active" or state == "ready")
      quests[#quests + 1] = {
        id = def.id,
        title = def.title or def.id,
        status = class,
        status_text = text,
        objective = objective_line(def, state),
        biome_name = BIOME_LABEL[def.biome or ""] or (def.biome or ""),
        progress = string.format("%.2f", frac),
        progress_text = progress_text,
        tracked = following and "tracked" or "",
      }
    end
  end
  publish("quests", quests)

  local bag = {}
  for _, item in ipairs(rules.items()) do
    local id = item.id
    local count = rules.vault_get(id)
    if count > 0 or item.kind == "resource" then
      bag[#bag + 1] = {
        id = id,
        label = item.label or id,
        count = tostring(count),
        kind = item.kind == "resource" and "resource" or "item",
        status = count == 0 and "empty" or "",
      }
    end
  end
  publish("bag", bag)

  local skills = {}
  for _, def in ipairs(rules.skills()) do
    local requires = def.requires or ""
    local status, text = "locked", "bloqueada"
    if learned(def.id) then
      status, text = "done", "aprendida"
    elseif requires == "" or learned(requires) then
      status, text = "ready", "disponível"
    end
    local required = rules.skill(requires)
    skills[#skills + 1] = {
      id = def.id,
      label = def.label,
      status = status,
      status_text = text,
      requires = requires == "" and "sem requisitos"
        or (required and required.label or requires),
    }
  end
  publish("skills", skills)
end

return M
