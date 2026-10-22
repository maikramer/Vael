-- Regras do Vael: HP, cofre, XP, quests, veneno, passivas e biomas.
-- TUDO em `viber.game()` (save) e `viber.game().enemies` (registro vivo).
-- A engine só expõe primitivas; não conhece vida, ouro nem missões.

local M = {}

-- Declarada cedo (a definição vem mais abaixo) para os handlers de quest
-- poderem tocar o som no herói e não na origem do controller.
local sound_at_player

-- Catálogo de conteúdo (data/*.json) — editável pelos plugins do editor.
-- `pcall`: um JSON ausente não pode impedir o jogo de arrancar; o log diz o
-- que falhou e os defaults abaixo mantêm o jogo jogável.
local function load_data(path, fallback)
  local ok, data = pcall(viber.load_json, path)
  if ok and type(data) == "table" then
    return data
  end
  viber.log("conteúdo: falha a ler " .. path .. (ok and "" or (" (" .. tostring(data) .. ")")))
  return fallback or {}
end

local ITEMS_DATA = load_data("data/items.json", {})
local PROGRESSION = load_data("data/progression.json", {})

local STACK = tonumber(ITEMS_DATA.stack_max) or 99
local RESOURCE_CAP = tonumber(ITEMS_DATA.resource_cap) or 1000000
local STARTING = ITEMS_DATA.starting or { gold = 30, potion = 1 }
local ITEM_ORDER = {}
local ITEM_BY_ID = {}
local RESOURCES = {}
for _, item in ipairs(ITEMS_DATA.items or {}) do
  if type(item) == "table" and type(item.id) == "string" and item.id ~= "" then
    ITEM_BY_ID[item.id] = item
    ITEM_ORDER[#ITEM_ORDER + 1] = item
    if item.kind == "resource" then
      RESOURCES[item.id] = true
    end
  end
end
table.sort(ITEM_ORDER, function(a, b)
  return (tonumber(a.order) or 0) < (tonumber(b.order) or 0)
end)

local XP = PROGRESSION.xp or {}
local XP_BASE = math.max(1, tonumber(XP.base) or 100)
local XP_GROWTH = tonumber(XP.growth) or 1.5
local SKILL_POINTS_PER_LEVEL = math.max(0, tonumber(XP.skill_points_per_level) or 1)
-- Chefes iguais a `combat::kill_xp`. O resto usa o piso do melee; tabelas
-- diferentes faziam a barra (regras) e os talentos (engine) subirem em
-- níveis distintos.
local KILL_XP = PROGRESSION.kill_xp or {}
local KILL_XP_DEFAULT = math.max(0, tonumber(KILL_XP.default) or 15)
-- Passivas do diário: os valores vivem em `data/progression.json`; a engine
-- não tem árvore de talentos. O diário só gasta o ponto.
local SKILL_ORDER = {}
local SKILL_BY_ID = {}
local SKILL_IDS = {}
local SKILL_EFFECTS = {}
for _, skill in ipairs(PROGRESSION.skills or {}) do
  if type(skill) == "table" and type(skill.id) == "string" and skill.id ~= "" then
    SKILL_BY_ID[skill.id] = skill
    SKILL_ORDER[#SKILL_ORDER + 1] = skill
    SKILL_IDS[#SKILL_IDS + 1] = skill.id
    SKILL_EFFECTS[skill.id] = skill.effects or {}
  end
end
local TALENT_EVERY = math.max(0, tonumber(PROGRESSION.song_seed_talent_every) or 5)

-- Fallback para projetos sem `quests/index.json` (o ficheiro é a lista que
-- o editor atualiza ao criar/apagar quests).
local QUEST_FILES = {
  "quests/city_quests.json",
  "quests/dark_forest_quests.json",
  "quests/desert_quests.json",
  "quests/fantasy_quests.json",
  "quests/mountain_quests.json",
  "quests/story_quests.json",
  "quests/swamp_quests.json",
}
local function quest_files()
  local ok, data = pcall(viber.load_json, "quests/index.json")
  if ok and type(data) == "table" and #data > 0 then
    local out = {}
    for _, name in ipairs(data) do
      if type(name) == "string" and name ~= "" then
        out[#out + 1] = "quests/" .. name
      end
    end
    if #out > 0 then
      return out
    end
  end
  return QUEST_FILES
end
local REVEAL_SECS = 2.5
-- `scan_visits` faz `viber.find` por cada marco das quests de visita. A
-- 60 Hz eram dezenas de buscas por segundo para uma mudança que só pode
-- acontecer quando o herói anda; 5 Hz chegam.
local SCAN_PERIOD = 0.2
local scan_acc = 0
-- `vault_add` espelha na engine, e a engine devolve um evento `collect`.
-- Sem este crédito o mesmo monte entrava outra vez no cofre do jogo.
local echo = {}

local function g()
  return viber.game()
end

-- NaN/±inf não são números úteis: um save corrompido não pode envenenar a
-- economia nem o HP para sempre.
local function finite(v)
  return type(v) == "number" and v == v and v ~= math.huge and v ~= -math.huge
end

local function num(key, default)
  local v = g()[key]
  if not finite(v) then
    return default or 0
  end
  return v
end

local function ensure_xp()
  local game = g()
  if not finite(game.xp) then game.xp = 0 end
  if not finite(game.xp_next) or game.xp_next < 1 then game.xp_next = XP_BASE end
  if not finite(game.level) or game.level < 1 then game.level = 1 end
  if not finite(game.skill_points) then game.skill_points = 0 end
end

-- O save é JSON PLANO: uma tabela `spawn_pos` era descartada em silêncio e o
-- jogador renascia onde carregou, não no último descanso. Guardam-se três
-- números; a tabela antiga ainda é lida para migrar saves da mesma sessão.
local function stored_spawn(game)
  if finite(game.spawn_x) and finite(game.spawn_y) and finite(game.spawn_z) then
    return { x = game.spawn_x, y = game.spawn_y, z = game.spawn_z }
  end
  local s = game.spawn_pos
  if type(s) == "table" and finite(s.x) and finite(s.y) and finite(s.z) then
    return { x = s.x, y = s.y, z = s.z }
  end
  return nil
end

-- HP do herói vive no JOGO (`viber.game()`), não em `Health` da engine.
local function ensure_hp()
  local game = g()
  if not finite(game.max_hp) or game.max_hp < 1 then game.max_hp = 100 end
  if not finite(game.hp) then game.hp = game.max_hp end
  -- Negativo não é vida: um save corrompido deixava o herói "vivo" a -N
  -- (barra a 0, criaturas a ignorarem-no como cadáver e sem ecrã de morte).
  if game.hp < 0 then game.hp = 0 end
  if game.hp > game.max_hp then game.hp = game.max_hp end
end

-- Um ponto é "de chão" quando está ao nível do terreno. Sem isto o primeiro
-- frame de um "Novo Jogo" gravava a pose que a cena anterior deixou no
-- transform (o acampamento do título, y≈8, dentro de `main`, y≈39) como
-- último descanso: o respawn caía a 30 m do sítio certo.
local function grounded(x, y, z)
  if not viber.ground_below then
    return true
  end
  -- De muito acima para apanhar a superfície. Num mundo sem terreno (QA) é
  -- nil e não há como validar — aceita-se.
  local top = viber.ground_below(x, y + 1000.0, z)
  if top == nil then
    return true
  end
  return math.abs(y - top) <= 3.0
end

-- Ponto de renascimento inicial. Só `main` (ou um mundo avulso) o define: um
-- save carregado dentro de uma casa não pode gravar as coordenadas LOCAIS do
-- interior como spawn do mundo grande. Fora do `ensure_hp` (chamado pelo
-- melee/hotbar): a consulta ao terreno corre no máximo uma vez por frame.
local function capture_spawn(game)
  if stored_spawn(game) then
    return
  end
  local scene = viber.scene_name and viber.scene_name() or nil
  if scene ~= nil and scene ~= "main" then
    return
  end
  local has, x, y, z = viber.player_position()
  if has and finite(x) and finite(y) and finite(z) and grounded(x, y, z) then
    game.spawn_x, game.spawn_y, game.spawn_z = x, y, z
  end
end

-- Passivas do diário: `SKILL_EFFECTS` é construída a partir de
-- `data/progression.json` no topo deste módulo.

local function learned_stats()
  local game = g()
  local stats = { bonus_damage = 0, crit_bonus = 0, speed_mult = 1.0, max_hp_bonus = 0 }
  for _, id in ipairs(SKILL_IDS) do
    if game["skill." .. id] == true then
      local eff = SKILL_EFFECTS[id]
      if eff then
        stats.bonus_damage = stats.bonus_damage + (eff.damage or 0)
        stats.crit_bonus = stats.crit_bonus + (eff.crit or 0)
        stats.speed_mult = stats.speed_mult + (eff.speed or 0)
        stats.max_hp_bonus = stats.max_hp_bonus + (eff.max_hp or 0)
      end
    end
  end
  return stats
end

function M.set_spawn(x, y, z)
  if finite(x) and finite(y) and finite(z) then
    local game = g()
    game.spawn_x, game.spawn_y, game.spawn_z = x, y, z
  end
end

function M.player_hp()
  ensure_hp()
  return true, g().hp, g().max_hp
end

function M.player_stats()
  ensure_hp()
  return M.stats or learned_stats()
end

local defs, defs_by_id
local function load_defs()
  if defs then
    return defs
  end
  defs, defs_by_id = {}, {}
  for _, path in ipairs(quest_files()) do
    local ok, data = pcall(viber.load_json, path)
    if ok and type(data) == "table" then
      for _, def in ipairs(data) do
        -- Ids repetidos entre ficheiros: a primeira definição ganha. O
        -- índice também torna `quest_state`/`publish` O(1) — eram varreduras
        -- lineares de 28 defs por cada missão, todos os frames.
        if def.id and not defs_by_id[def.id] then
          defs_by_id[def.id] = def
          defs[#defs + 1] = def
        end
      end
    end
  end
  table.sort(defs, function(a, b) return a.id < b.id end)
  return defs
end

local function find_def(id)
  load_defs()
  return defs_by_id[id]
end

local function qkey(id)
  return "q." .. id
end

function M.quest_defs()
  return load_defs()
end

function M.quest_def(id)
  return find_def(id)
end

function M.items()
  return ITEM_ORDER
end

function M.item_def(id)
  return ITEM_BY_ID[id]
end

function M.item_label(id)
  local def = ITEM_BY_ID[id]
  return def and def.label or id
end

function M.stack_max()
  return STACK
end

function M.skills()
  return SKILL_ORDER
end

function M.skill(id)
  return SKILL_BY_ID[id]
end

function M.talent_every()
  return TALENT_EVERY
end

function M.quest_progress(id)
  local def = find_def(id)
  local need = 1
  if def and def.objective and def.objective.count then
    -- `count: 0`/ausente não pode dividir por zero no HUD.
    need = math.max(1, math.floor(tonumber(def.objective.count) or 1))
  end
  return num("q." .. id .. ".n", 0), need
end

function M.quest_state(id)
  local state = g()[qkey(id)]
  if state == "active" or state == "ready" or state == "done" or state == "not_taken" then
    return state
  end
  return find_def(id) and "not_taken" or "unknown"
end

local function objective(def)
  return def.objective or {}
end

-- `count: 0`/ausente é uma quest de um passo, não uma divisão por zero nem
-- um objetivo que nasce pronto (`progress >= 0`).
local function count_of(obj)
  return math.max(1, math.floor(tonumber((obj or {}).count) or 1))
end

local function progress_of(id)
  return num(qkey(id) .. ".n", 0)
end

local function set_progress(id, n)
  g()[qkey(id) .. ".n"] = n
end

local function mark_ready(def)
  local obj = objective(def)
  local need = count_of(obj)
  if progress_of(def.id) >= need and M.quest_state(def.id) == "active" then
    g()[qkey(def.id)] = "ready"
    viber.toast("Objetivo completo: " .. (def.title or def.id))
    sound_at_player("notification")
  end
end

local function repeatable(def)
  return def.npc == "notice_board"
end

local function clear_visits(def)
  for place in tostring((def.objective or {}).target or ""):gmatch("%S+") do
    g()[qkey(def.id) .. ".v." .. place] = nil
  end
end

function M.quest_accept(id)
  local def = find_def(id)
  if not def or M.quest_state(id) ~= "not_taken" then
    return false
  end
  g()[qkey(id)] = "active"
  set_progress(id, 0)
  clear_visits(def)
  viber.toast("Quest aceite: " .. (def.title or id))
  sound_at_player("quest_accept")
  M.sync_collect()
  return true
end

local function grant_rewards(def)
  local rewards = def.rewards or {}
  if rewards.gold and rewards.gold > 0 then
    M.vault_add("gold", rewards.gold)
  end
  if rewards.xp and rewards.xp > 0 then
    M.add_xp(rewards.xp)
  end
  -- Devolve quantos itens NÃO couberam (tecto de 99 por stack): o toast de
  -- entrega dizia "+5 poções" e a mochila só aceitava 2, em silêncio.
  local missing = 0
  if type(rewards.items) == "table" then
    for _, item in ipairs(rewards.items) do
      local name, count = tostring(item):match("^([^:]+):?(%d*)$")
      local want = tonumber(count) or 1
      if name and name ~= "" and want > 0 then
        local added = M.item_add(name, want) or 0
        if added < want then
          missing = missing + (want - added)
        end
      end
    end
  end
  return missing
end

function M.quest_turn_in(id)
  if M.quest_state(id) ~= "ready" then
    return false
  end
  local def = find_def(id)
  if not def then
    return false
  end
  local obj = objective(def)
  if obj.type == "collect" then
    local need = count_of(obj)
    if not M.vault_take(obj.target or "", need) then
      g()[qkey(id)] = "active"
      viber.toast("Já não tens o que a missão pede.")
      return false
    end
  end
  if repeatable(def) then
    g()[qkey(id)] = "not_taken"
    set_progress(id, 0)
  else
    g()[qkey(id)] = "done"
    -- O cartão do HUD e a bússola não podem continuar a "seguir" uma missão
    -- fechada: o diário já não deixa voltar a escolhê-la (só aceita
    -- active/ready), por isso a marca ficava no save para sempre.
    if g().tracked_quest == id then
      g().tracked_quest = nil
    end
  end
  local missing = grant_rewards(def)
  local gold = (def.rewards or {}).gold or 0
  local msg = "Quest entregue: " .. (def.title or id)
    .. (gold > 0 and (" (+" .. gold .. " ouro)") or "")
  if missing > 0 then
    msg = msg .. " — " .. missing .. " item(ns) não coube(ram) na mochila."
  end
  viber.toast(msg)
  sound_at_player("quest_complete")
  viber.save_slot(0)
  return true
end

-- bandit-archer conta como bandit. Substring ao contrário ("a" dentro de
-- "bandit", ou "boss" dentro de um nome qualquer) contava abates a mais.
local function kill_matches(kind, target)
  if kind == "" or target == "" then
    return false
  end
  if kind == target then
    return true
  end
  return kind:sub(1, #target + 1) == target .. "-"
end

function M.kind_matches(kind, target)
  return kill_matches(kind, target)
end

function M.report_kill(kind)
  if not kind or kind == "" then
    return
  end
  for _, def in ipairs(load_defs()) do
    if M.quest_state(def.id) == "active" then
      local obj = objective(def)
      if obj.type == "kill" and kill_matches(kind, obj.target or "") then
        local need = count_of(obj)
        set_progress(def.id, math.min(need, progress_of(def.id) + 1))
        mark_ready(def)
      end
    end
  end
end

-- Rótulo humano de um marco. O toast mostrava o id crú da entidade
-- ("forest-crossroads-well"); o mapa já tem o nome autoral em
-- `lib/landmarks.lua`, por isso a tabela é carregada uma vez, à primeira
-- visita (sem acoplar o `rules` ao módulo no arranque).
local place_labels
local function place_label(place)
  if not place_labels then
    place_labels = {}
    local ok, marks = pcall(viber.load, "lib/landmarks.lua")
    if ok and type(marks) == "table" and type(marks.LIST) == "table" then
      for _, mark in ipairs(marks.LIST) do
        if mark.name and mark.label then
          place_labels[mark.name] = mark.label
        end
      end
    end
  end
  return place_labels[place] or place
end

function M.report_visit(place)
  if not place or place == "" then
    return
  end
  for _, def in ipairs(load_defs()) do
    if M.quest_state(def.id) == "active" then
      local obj = objective(def)
      if obj.type == "visit" then
        local listed = false
        for token in tostring(obj.target or ""):gmatch("%S+") do
          if token == place then
            listed = true
            break
          end
        end
        local mark = qkey(def.id) .. ".v." .. place
        if listed and g()[mark] ~= true then
          g()[mark] = true
          local n = 0
          for token in tostring(obj.target or ""):gmatch("%S+") do
            if g()[qkey(def.id) .. ".v." .. token] == true then
              n = n + 1
            end
          end
          set_progress(def.id, math.min(count_of(obj), n))
          mark_ready(def)
          viber.toast("Marco visto: " .. place_label(place))
        end
      end
    end
  end
end

function M.scan_visits()
  local has, px, _, pz = viber.player_position()
  if not has then
    return
  end
  for _, def in ipairs(load_defs()) do
    if M.quest_state(def.id) == "active" then
      local obj = objective(def)
      if obj.type == "visit" then
        local radius = obj.radius or 12
        for token in tostring(obj.target or ""):gmatch("%S+") do
          if g()[qkey(def.id) .. ".v." .. token] ~= true then
            local id = viber.find(token)
            if id then
              local pok, x, _, z = viber.entity_position(id)
              if pok then
                local dx, dz = x - px, z - pz
                if dx * dx + dz * dz <= radius * radius then
                  M.report_visit(token)
                end
              end
            end
          end
        end
      end
    end
  end
end

function M.report_collect(item, amount)
  M.item_add(item, amount or 1)
  M.sync_collect()
end

function M.sync_collect()
  for _, def in ipairs(load_defs()) do
    local state = M.quest_state(def.id)
    if state == "active" or state == "ready" then
      local obj = objective(def)
      if obj.type == "collect" then
        local need = count_of(obj)
        local have = M.vault_get(obj.target or "")
        set_progress(def.id, math.min(have, need))
        if have >= need then
          mark_ready(def)
        elseif state == "ready" then
          g()[qkey(def.id)] = "active"
        end
      end
    end
  end
end

-- Contagem guardada no cofre, nunca negativa: um save com `item.wood = -3`
-- aparecia na mochila a -3 e o `vault_add` devolvia mais do que somou.
local function held(key)
  return math.max(0, num(key, 0))
end

function M.vault_get(kind)
  if not kind or kind == "" then
    return 0
  end
  return held("item." .. kind)
end

-- Compat: se a engine ainda expuser `viber.vault_*`, espelha-se. A fonte da
-- verdade é SEMPRE `viber.game()`; um jogo novo não precisa da API antiga.
local function engine_fn(name)
  local fn = viber[name]
  if type(fn) == "function" then
    return fn
  end
  return nil
end

local function mirror_add(kind, amount)
  if amount <= 0 then
    return
  end
  local fn = RESOURCES[kind] and engine_fn("vault_add") or engine_fn("item_add")
  if not fn then
    return
  end
  echo[kind] = (echo[kind] or 0) + amount
  fn(kind, amount)
end

local function mirror_take(kind, amount)
  local fn = engine_fn("vault_take")
  if amount > 0 and fn then
    fn(kind, amount)
  end
end

function M.pull_engine()
  local tracked = {
    "gold", "wood", "stone", "dark-wood", "bog-moss", "nota",
    "potion", "antidote", "bomb", "korok-seed", "berry",
  }
  local vault_get = engine_fn("vault_get")
  local item_count = engine_fn("item_count")
  if g().eco_pulled ~= true then
    g().eco_pulled = true
    if vault_get or item_count then
      for _, kind in ipairs(tracked) do
        local fn = RESOURCES[kind] and vault_get or item_count
        if fn then
          local n = fn(kind) or 0
          if n > 0 then
            g()["item." .. kind] = n
          end
        end
      end
      -- Sem `return`: um cofre nativo vazio mas sem chaves no save ainda
      -- precisa do kit de arranque (o `if` abaixo decide).
    end
  end
  -- Kit de arranque (`data/items.json`) quando o save ainda não gravou
  -- NENHUMA destas chaves — um 0 explícito (ouro gasto) não volta a encher.
  -- Vale também com cofre nativo vazio.
  if g()["item.gold"] == nil and g()["item.potion"] == nil then
    for id, count in pairs(STARTING) do
      if type(id) == "string" and finite(count) and count > 0 then
        g()["item." .. id] = count
      end
    end
  end
end

function M.vault_add(kind, amount, mirror)
  amount = amount or 1
  if not finite(amount) or amount <= 0 or not kind or kind == "" then
    return 0
  end
  local key = "item." .. kind
  local have = held(key)
  local def = ITEM_BY_ID[kind]
  local cap = RESOURCES[kind] and RESOURCE_CAP
    or (def and tonumber(def.stack))
    or STACK
  local nextv = math.min(cap, have + amount)
  local added = nextv - have
  if added <= 0 then
    return 0
  end
  g()[key] = nextv
  if mirror ~= false then
    mirror_add(kind, added)
  end
  M.sync_collect()
  return added
end

function M.vault_take(kind, amount)
  amount = amount or 1
  if not kind or kind == "" or not finite(amount) or amount <= 0 then
    return false
  end
  local key = "item." .. kind
  local have = held(key)
  if have < amount then
    return false
  end
  g()[key] = have - amount
  mirror_take(kind, amount)
  M.sync_collect()
  return true
end

function M.item_add(id, amount)
  return M.vault_add(id, amount or 1)
end

function M.item_count(id)
  return M.vault_get(id)
end

function M.add_xp(gain, mirror)
  gain = gain or 0
  if not finite(gain) or gain <= 0 then
    return
  end
  -- `false` = a engine já creditou este ganho (abate do melee). O resto
  -- (baú, marco, colheita) também sobe o XP que dá pontos de talento.
  local add_xp = engine_fn("add_xp")
  if mirror ~= false and add_xp then
    add_xp(math.floor(gain))
  end
  ensure_xp()
  local game = g()
  game.xp = game.xp + gain
  while game.xp >= game.xp_next do
    game.xp = game.xp - game.xp_next
    game.xp_next = math.max(1, math.ceil(game.xp_next * XP_GROWTH))
    game.level = game.level + 1
    game.skill_points = game.skill_points + SKILL_POINTS_PER_LEVEL
  end
end

function M.add_skill_points(n)
  n = n or 0
  -- NaN/inf somavam-se a `skill_points` e o `ensure_xp` seguinte zerava a
  -- carteira toda (incluindo os pontos já ganhos).
  if not finite(n) or n <= 0 then
    return
  end
  ensure_xp()
  g().skill_points = num("skill_points", 0) + n
  -- O HUD e o diário gastam `viber.game().skill_points`. O espelho nativo
  -- só existe se a engine ainda expuser a função.
  local add_points = engine_fn("add_skill_points")
  if add_points then
    add_points(math.floor(n))
  end
end

function M.reveal_biome(name)
  if name and name ~= "" then
    g()["biome." .. name] = true
  end
end

function M.biome_revealed(name)
  return g()["biome." .. name] == true
end

local function region_of(x, z)
  if math.abs(x) <= 120 and math.abs(z) <= 120 then
    return 0
  end
  if math.abs(x) >= math.abs(z) then
    return x >= 0 and 3 or 4
  end
  return z >= 0 and 1 or 2
end

-- Registro vivo dos hostis: cada script de criatura mantém
-- `viber.game().enemies[id]` (kind/hp/max/pos/forward/t). O jogador lê daqui;
-- a engine não tem lista de hostis nem Health.
-- Hostis a `range` de um PONTO (o centro da explosão, não o herói).
function M.hostiles_near(cx, cz, range)
  local game = g()
  local now = viber.time()
  local out = {}
  for id, e in pairs(game.enemies or {}) do
    if type(id) == "number" and (e.hp or 0) > 0 and e.t and e.t <= now and (now - e.t) < 1.0 then
      local dx, dz = (e.x or cx) - cx, (e.z or cz) - cz
      local dist = math.sqrt(dx * dx + dz * dz)
      if dist <= (range or 2.4) then
        out[#out + 1] = {
          id = id,
          kind = e.kind or "creature",
          x = e.x, y = e.y, z = e.z,
          hp = e.hp, max = e.max,
          distance = dist,
          forward_x = e.fx, forward_z = e.fz,
        }
      end
    end
  end
  table.sort(out, function(a, b) return a.distance < b.distance end)
  return out
end

function M.hostiles(range)
  ensure_hp()
  local ok, px, _, pz = viber.player_position()
  if not ok then
    return {}
  end
  return M.hostiles_near(px, pz, range)
end

-- Registro INTEIRO de hostis vivos (sem filtro de distância). O waypoint
-- de uma quest de abate precisa de saber onde está a presa, não só a que
-- está ao alcance do melee.
function M.hostiles_all()
  local now = viber.time()
  local out = {}
  for id, e in pairs(g().enemies or {}) do
    if type(id) == "number" and (e.hp or 0) > 0
        and e.t and e.t <= now and (now - e.t) < 1.0 then
      out[#out + 1] = {
        id = id,
        kind = e.kind or "creature",
        x = e.x, y = e.y, z = e.z,
        hp = e.hp, max = e.max,
      }
    end
  end
  return out
end

function M.damage_enemy(id, amount)
  ensure_hp()
  local e = (g().enemies or {})[id]
  if not e or (e.hp or 0) <= 0 then
    return false, e
  end
  if not finite(amount) or amount < 0 then
    amount = 0
  end
  e.hp = math.max(0, e.hp - amount)
  return e.hp <= 0, e
end

-- Dano em área do JOGO: falloff linear (cheio → metade na borda), knockback
-- e números. A morte é decidida pelo script da criatura no frame seguinte.
function M.radial_damage(x, z, radius, damage, knockback)
  local hits = 0
  -- A busca E o falloff medem-se do centro da explosão. Antes usavam
  -- `h.distance` (distância ao herói): a bomba largada à frente atingia
  -- inimigos atrás do jogador e ignorava os que estavam na borda real.
  for _, h in ipairs(M.hostiles_near(x, z, radius)) do
    local dx, dz = (h.x or x) - x, (h.z or z) - z
    local dist = math.sqrt(dx * dx + dz * dz)
    local falloff = 1.0 - 0.5 * (dist / math.max(radius, 1e-3))
    local dmg = (damage or 0) * falloff
    M.damage_enemy(h.id, dmg)
    if viber.hit_flash then
      viber.hit_flash(h.id)
    end
    if knockback and knockback > 0 and viber.entity_knockback then
      local len = math.sqrt(dx * dx + dz * dz)
      if len > 1e-3 then
        viber.entity_knockback(h.id, dx / len, dz / len, knockback * falloff)
      end
    end
    viber.damage_number("-" .. math.floor(dmg), {
      color = "#ffd35a", x = h.x, y = (h.y or 0) + 1.6, z = h.z,
    })
    hits = hits + 1
  end
  return hits
end

-- Gate de chefe: lê o REGISTO inteiro (sem o filtro de frescura de
-- `hostiles`, que só via criaturas perto do herói e deixava o gate abrir
-- com os guardiões vivos longe do LOD). `except_kind` exclui o próprio
-- chefe: ele pode já ter registo de uma sessão anterior e não pode contar
-- como "inimigo que falta matar".
function M.alive_in_region(idx, except_kind)
  local n = 0
  for id, e in pairs(g().enemies or {}) do
    if type(id) == "number" and (e.hp or 0) > 0
        and (except_kind == nil or e.kind ~= except_kind)
        and region_of(e.x or 0, e.z or 0) == idx then
      n = n + 1
    end
  end
  return n
end

function M.apply_status(kind, secs)
  if kind == "venom" then
    local dur = math.max(num("venom", 0), secs or 0)
    g().venom = dur
  end
end

function M.status_clear(kind)
  if kind == nil or kind == "venom" then
    g().venom = 0
    g().venom_tick = 0
  end
end

function M.poisoned()
  return num("venom", 0) > 0
end

function M.kill_xp(kind)
  return KILL_XP[kind] or KILL_XP_DEFAULT
end

function M.award_kill(kind, id)
  M.add_xp(M.kill_xp(kind))
  M.report_kill(kind)
  if M.on_kill then
    M.on_kill(kind, id)
  end
  -- Chefe abatido = marco de progresso: autosave no slot 0. Num timer
  -- curto: o `fsm` paga o loot DEPOIS de `award_kill`, e o save síncrono
  -- gravava o estado sem a bolsa/poções do chefe (morrer ou Carregar
  -- perdia-as).
  if KILL_XP[kind] then
    viber.after(0.05, function()
      viber.save_slot(0)
    end)
  end
end

-- Direção do dano: quatro faixas no HUD. O atacante pode estar à frente,
-- atrás, à esquerda ou à direita do FORWARD do herói; usa-se o produto
-- interno (frente/trás) contra o produto externo (lados).
local DMG_EDGES = { "dmg-front", "dmg-back", "dmg-left", "dmg-right" }
local dmg_flash = 0

local function clear_dmg_edges()
  for _, id in ipairs(DMG_EDGES) do
    viber.ui.set_visible(id, false)
  end
end

local function flash_damage_direction(sx, sz)
  if not sx or not sz then
    return
  end
  local has, px, _, pz = viber.player_position()
  if not has then
    return
  end
  local fx, fz = viber.player_forward()
  local dx, dz = sx - px, sz - pz
  local len = math.sqrt(dx * dx + dz * dz)
  if len < 0.25 then
    return
  end
  dx, dz = dx / len, dz / len
  local dot = fx * dx + fz * dz
  local cross = fx * dz - fz * dx
  local id
  if math.abs(dot) >= math.abs(cross) then
    id = dot > 0 and "dmg-front" or "dmg-back"
  else
    id = cross > 0 and "dmg-right" or "dmg-left"
  end
  dmg_flash = dmg_flash + 1
  local token = dmg_flash
  clear_dmg_edges()
  viber.ui.set_visible(id, true)
  viber.after(0.35, function()
    if token == dmg_flash then
      viber.ui.set_visible(id, false)
    end
  end)
end

-- Guarda/parry [L] — regra do jogo, em Lua. O dano dos inimigos passa
-- TODO por `damage_player`; a janela de parry é curta e o bloqueio reduz.
local GUARD_SECS = 0.5
local PARRY_WINDOW = 0.2
local GUARD_REDUCTION = 0.25
-- Timers de SESSÃO, não de save: `viber.time()` recomeça em 0 e um
-- `guard_until` gravado deixava o herói invencível (parry eterno) até o
-- relógio novo alcançar o timestamp antigo.
local guard_until, guard_start, guard_lock = 0, nil, 0

function M.guard()
  local now = viber.time()
  -- Não REABRE a guarda enquanto ela está activa: premir [L] em cadência
  -- reiniciava `guard_start` todos os frames e a janela de parry (0,2 s)
  -- ficava permanentemente aberta. Entre guardas há uma pequena folga.
  if now < guard_lock or now < guard_until then
    return
  end
  guard_until, guard_start = now + GUARD_SECS, now
  guard_lock = now + GUARD_SECS + 0.2
end

local function guarding()
  return viber.time() < guard_until
end

-- Números/SFX sobre o HERÓI. O host das regras vive em (0, 38.73, 0):
-- sem x/y/z o dano e o som apareciam na origem do mundo.
function sound_at_player(name)
  local has, x, y, z = viber.player_position()
  if has then
    viber.sound(name, { x = x, y = y, z = z })
  else
    viber.sound(name)
  end
end

local function player_fx(text, color, sfx)
  local has, x, y, z = viber.player_position()
  local opts = { color = color }
  if has then
    opts.x, opts.y, opts.z = x, y + 2.0, z
  end
  viber.damage_number(text, opts)
  if sfx then
    sound_at_player(sfx)
  end
end

local function start_death(game)
  if num("dying", 0) > 0 or game.death_pending then
    return
  end
  game.dying = 2.2
  local id = viber.player_id()
  if id then
    viber.play_clip("death", { id = id })
  end
  sound_at_player("game_over")
end

-- Devolve true só quando o dano ENTROU: os `on_attack`/`on_hit` das
-- criaturas (veneno) não podem disparar contra um parry/i-frame.
function M.damage_player(amount, sx, sz)
  -- Menu/diário/mapa abertos: o jogador parou para ler. As criaturas já
  -- suspendem o golpe (lib/fsm.lua); sem este portão uma seta já no ar
  -- continuava a tirar vida por baixo do menu.
  if viber.menu_open() then
    return false
  end
  if num("iframes", 0) > 0 or num("dying", 0) > 0 or g().death_pending then
    return false
  end
  ensure_hp()
  local game = g()
  amount = math.max(0, amount or 0)
  if not finite(amount) or amount <= 0 then
    return false
  end
  local now = viber.time()
  if guarding() then
    local parried = guard_start and (now - guard_start) <= PARRY_WINDOW
    if parried then
      -- Aparo: o golpe não entra. Larga a guarda e trava o [L] por
      -- GUARD_SECS — bater L em cadência anulava todo o dano para sempre.
      guard_until, guard_start = 0, nil
      guard_lock = now + GUARD_SECS
      game.iframes = math.max(num("iframes", 0), 0.25)
      viber.toast("ESQUIVA")
      player_fx("ESQUIVA", "#9fd8ff", "block")
      return false
    end
    amount = amount * GUARD_REDUCTION
    viber.toast("Bloqueaste")
    sound_at_player("block")
  end
  game.hp = math.max(0, game.hp - amount)
  player_fx("-" .. math.floor(amount), "#ff6b6b", "hurt")
  flash_damage_direction(sx, sz)
  viber.shake(0.18)
  game.iframes = 0.35
  if game.hp <= 0 then
    start_death(game)
  end
  return true
end

function M.heal_player(amount)
  ensure_hp()
  if not finite(amount) or amount <= 0 then
    return
  end
  local game = g()
  -- Cadáver/ecrã de morte: uma poção ou uma fogueira não ressuscitam o herói
  -- no sítio onde caiu (o `tick` via `hp > 0` e fechava o ecrã de morte).
  if num("dying", 0) > 0 or game.death_pending then
    return
  end
  game.hp = math.min(game.max_hp, game.hp + amount)
end

function M.grant_iframes(secs)
  g().iframes = math.max(num("iframes", 0), secs or 0)
end

local function sync_passives()
  ensure_hp()
  local game = g()
  local stats = learned_stats()
  M.stats = stats
  local new_max = 100 + stats.max_hp_bonus
  if game.max_hp ~= new_max then
    local delta = new_max - (game.max_hp or 100)
    game.max_hp = new_max
    game.hp = math.max(0, math.min(new_max, (game.hp or new_max) + delta))
  end
  if viber.set_player_speed and M._speed ~= stats.speed_mult then
    M._speed = stats.speed_mult
    viber.set_player_speed(stats.speed_mult)
  end
end

-- Renasce no último descanso. O ecrã de morte chama isto em "Tentar de novo";
-- mundos sem o modal (QA) usam o fallback de 5 s no `tick`.
function M.respawn()
  local game = g()
  local s = stored_spawn(game) or { x = 0, y = 150, z = 0 }
  game.hp = game.max_hp or 100
  game.dying = 0
  game.death_pending = false
  game.death_wait = 0
  game.venom = 0
  game.venom_tick = 0
  game.iframes = 2.0
  viber.ui.open("death", false)
  viber.ui.set_visible("hud", true)
  viber.toast("Renasces no último descanso.")
  sound_at_player("levelup")
  -- Morrer dentro de uma casa e "Tentar de novo" teleportava para as
  -- coordenadas do mundo grande DENTRO da cena do interior (fora das
  -- paredes, a cair). O respawn troca primeiro para `main`; o teleporte
  -- corre no `scene_loaded`, já com o mundo certo carregado.
  local scene = viber.scene_name and viber.scene_name() or nil
  if scene and scene ~= "main" and scene ~= "menu" and viber.change_scene then
    game.pending_respawn = true
    local ok = viber.change_scene("main")
    if ok then
      return
    end
    game.pending_respawn = false
  end
  viber.teleport_player(s.x, s.y, s.z)
end

function M.tick(dt)
  -- `viber.time()` recomeça no load, mas os timers de sessão (guarda, flashes
  -- do HUD) são locals do módulo e sobrevivem. Um `guard_lock` no "futuro"
  -- do relógio novo trancava o [L] durante minutos; o mesmo para as janelas
  -- de "recente". Relógio para trás = sessão de jogo nova.
  local now = viber.time()
  if M._last_time and now < M._last_time then
    guard_until, guard_start, guard_lock = 0, nil, 0
    M._xp_until, M._quest_until = nil, nil
  end
  M._last_time = now
  -- Novo Jogo/save de outra partida: flashes, guarda e passivas recomeçam.
  local sid = tonumber(g().session_id) or 0
  if M._session ~= sid then
    M._session = sid
    M._xp_sig, M._quest_sig = nil, nil
    M._xp_until, M._quest_until = nil, nil
    M._speed = nil
    guard_until, guard_start, guard_lock = 0, nil, 0
  end
  M.pull_engine()
  sync_passives()
  ensure_hp()
  -- `ensure_xp` antes dos metadados: o save gravado no meio do frame não
  -- pode levar "nível nil/NaN" (a barra recuperava só no `publish`).
  ensure_xp()
  local game = g()
  capture_spawn(game)
  -- Metadados do save: o menu de continuar lê `playtime_secs`/`save_title`
  -- do world_kv sem abrir o resto do estado. Via `num`: um save editado à
  -- mão com `playtime_secs` textual rebentava o `tick` TODOS os frames.
  game.playtime_secs = num("playtime_secs", 0) + dt
  game.save_title = "Vael — nível " .. (game.level or 1)
  game.save_level = game.level or 1
  game.save_gold = M.vault_get("gold") or 0
  game.iframes = math.max(0, num("iframes", 0) - dt)

  -- Veneno: 4 dps, tick de 1 s. Não corre durante a morte (o cadáver ficava
  -- a 0 HP sem `dying` e nenhum caminho o curava/matava), nem com um menu
  -- aberto (ler o diário não pode matar), e um frame longo aplica TODOS os
  -- ticks devidos, não só um.
  if num("venom", 0) > 0 and not viber.menu_open() then
    game.venom = math.max(0, game.venom - dt)
    if num("dying", 0) <= 0 then
      game.venom_tick = (game.venom_tick or 0) + dt
      local ticks = math.floor(game.venom_tick)
      if ticks > 0 then
        game.venom_tick = game.venom_tick - ticks
        local dmg = ticks * 4
        game.hp = math.max(0, game.hp - dmg)
        player_fx("-" .. dmg, "#7ed957")
        if game.hp <= 0 then
          start_death(game)
        end
      end
    end
  else
    game.venom_tick = 0
  end

  -- Um save carregado com o herói vivo não pode reabrir o ecrã de morte.
  if game.death_pending and (game.hp or 0) > 0 then
    game.death_pending = false
    game.dying = 0
    viber.ui.open("death", false)
  end

  -- 0 HP sem `dying` (save corrompido ou gravado a meio de uma morte): o
  -- herói ficava invisível para as criaturas (`hero_hp <= 0`) e nunca morria.
  if game.hp <= 0 and num("dying", 0) <= 0 and not game.death_pending then
    start_death(game)
  end

  -- Morte do herói: 2,2 s caído e depois o ecrã de morte. O jogador escolhe
  -- Tentar de novo / Carregar / Título; sem modal (mundos QA), renasce em 5 s.
  if num("dying", 0) > 0 then
    game.dying = math.max(0, game.dying - dt)
    if game.dying <= 0 and not game.death_pending then
      game.death_pending = true
      game.death_wait = 5.0
      -- A morte fecha os menus abertos; o ecrã de morte é o topo.
      viber.ui.open("menu", false)
      viber.ui.open("region-map", false)
      viber.ui.open("travel", false)
      viber.ui.open("profiler-win", false)
      viber.ui.open("death", true)
    end
  elseif game.death_pending then
    if viber.ui.is_open("death") then
      game.death_wait = 5.0
    else
      game.death_wait = math.max(0, (game.death_wait or 0) - dt)
      if game.death_wait <= 0 then
        M.respawn()
      end
    end
  end

  for _, ev in ipairs(viber.events()) do
    if ev.type == "kill" and ev.name then
      -- Mesmo caminho do abate do FSM: XP + quests + `on_kill` (as Notas da
      -- story) + autosave de chefe. Duplicar aqui `add_xp`/`report_kill`
      -- deixava um abate vindo da engine sem Nota e sem save.
      M.award_kill(ev.name, ev.entity)
      sound_at_player("enemy_death")
    elseif ev.type == "hit" then
      if ev.faction == "player" and ev.entity then
        -- Um `amount` NaN/inf do projétil envenenava `entry.hp` (a criatura
        -- desaparecia de `hostiles`); `damage_enemy` sanea o valor.
        M.damage_enemy(ev.entity, ev.amount or 0)
        if ev.x then
          viber.damage_number("-" .. tostring(math.floor(ev.amount or 0)), {
            x = ev.x, y = (ev.y or 0) + 1.8, z = ev.z, color = "#8cd9ff",
          })
        end
      else
        M.damage_player(ev.amount or 0, ev.x, ev.z)
      end
    elseif ev.type == "collect" and ev.item then
      -- Compat: evento do cofre da engine (se ainda existir).
      local amount = ev.amount or 1
      local pending = echo[ev.item] or 0
      local skip = math.min(pending, amount)
      echo[ev.item] = pending - skip
      local rest = amount - skip
      if rest > 0 then
        M.vault_add(ev.item, rest, false)
        M.add_xp(30)
      end
    elseif ev.type == "scene_loaded" then
      -- A cena recriou as criaturas com bits NOVOS. As entradas da instância
      -- anterior ficavam órfãs no registo (mesma sessão ⇒ não "stale") e
      -- `alive_in_region` contava-as para sempre: depois de uma ida à vila o
      -- gate do ogro nunca abria. O registo é VIVO; os scripts da cena que
      -- acabou de carregar voltam a registar-se.
      game.enemies = {}
      game.elites = {}
      if game.pending_respawn then
        -- Respawn pedido dentro de uma cena própria: agora que `main`
        -- carregou, o spawn guardado volta a ser coordenadas válidas.
        game.pending_respawn = false
        local s = stored_spawn(game)
        if s then
          viber.teleport_player(s.x, s.y, s.z)
          game.iframes = math.max(num("iframes", 0), 2.0)
        end
      end
    end
  end
  scan_acc = scan_acc + dt
  if scan_acc >= SCAN_PERIOD then
    scan_acc = 0
    if not viber.menu_open() then
      M.scan_visits()
    end
  end
  M.sync_collect()
  M.publish()
end

function M.publish()
  ensure_xp()
  ensure_hp()
  local game = g()
  local hp, max_hp = game.hp, game.max_hp
  local frac = max_hp > 0 and math.max(0, math.min(1, hp / max_hp)) or 0
  viber.ui.set("health", frac)
  viber.ui.set("health.text", string.format("%d/%d", math.floor(hp + 0.5), math.floor(max_hp + 0.5)))
  viber.ui.set("health.value", hp)
  viber.ui.set("health.low", frac <= 0.3)
  viber.ui.set("status.venom", num("venom", 0) > 0)
  viber.ui.set("xp", game.xp / game.xp_next)
  viber.ui.set("xp.text", string.format("%d/%d", math.floor(game.xp), math.floor(game.xp_next)))
  viber.ui.set("level", game.level)
  viber.ui.set("level.text", tostring(game.level))
  viber.ui.set("talent.points", game.skill_points)
  viber.ui.set("talent.ready", game.skill_points > 0)
  viber.ui.set("gold", M.vault_get("gold"))
  viber.ui.set("wood", M.vault_get("wood"))
  viber.ui.set("stone", M.vault_get("stone"))
  viber.ui.set("berry", M.vault_get("berry"))
  viber.ui.set("korok-seed", M.vault_get("korok-seed"))
  viber.ui.set("potion", M.vault_get("potion"))
  viber.ui.set("antidote", M.vault_get("antidote"))
  viber.ui.set("bomb", M.vault_get("bomb"))
  viber.ui.set("potion.empty", M.vault_get("potion") < 1)
  viber.ui.set("antidote.empty", M.vault_get("antidote") < 1)
  -- Netplay: o estado vem do `VIBER_NET` da engine; o jogo só mostra.
  local net_role = (viber.net_role and viber.net_role()) or "off"
  viber.ui.set("net.on", net_role ~= "off")
  viber.ui.set("net.label", net_role == "host" and "HOST"
    or (net_role == "join" and "CLIENTE" or ""))
  viber.ui.set("bomb.empty", M.vault_get("bomb") < 1)
  local title, progress, text = "", 0, ""
  local best, best_score
  -- O diário permite SEGUIR uma missão (`tracked_quest`); o cartão do HUD
  -- tem de mostrar essa, não apenas a de maior progresso. Sem isto o botão
  -- de seguir só mudava a seta.
  local tracked = game.tracked_quest
  local tracked_def
  if tracked then
    local def = find_def(tracked)
    local state = def and M.quest_state(tracked)
    if def and (state == "active" or state == "ready") then
      tracked_def = def
    end
  end
  for _, def in ipairs(load_defs()) do
    local state = M.quest_state(def.id)
    if state == "active" or state == "ready" then
      local obj = objective(def)
      local need = count_of(obj)
      local have = progress_of(def.id)
      local score = (state == "ready" and 2 or 0) + (need > 0 and have / need or 0)
      if tracked_def and def.id == tracked_def.id then
        score = 1e9 -- a missão seguida ganha sempre
      end
      if not best_score or score > best_score then
        best, best_score = def, score
        title = def.title or def.id
        progress = need > 0 and math.min(1, have / need) or 0
        text = string.format("%d/%d", have, need)
        if state == "ready" then
          text = "pronta"
          progress = 1
        end
      end
    end
  end
  viber.ui.set("quest.title", title)
  viber.ui.set("quest", progress)
  viber.ui.set("quest.text", text)
  viber.ui.set("quest.active", title ~= "")
  viber.ui.set("abilities.active", true)
  viber.ui.set("vitals.active", true)
  -- Uma vez: o analyze avisa "unknown binding" se ninguém escreveu o nome.
  -- Todos os frames apagava a barra de alvo, o chefe e as recargas quando
  -- este script corria depois dos outros.
  if not M._hud_seeded then
    M._hud_seeded = true
    viber.ui.set("target", 0)
    viber.ui.set("target.alive", false)
    viber.ui.set("target.name", "")
    viber.ui.set("combo", false)
    viber.ui.set("combo.text", "")
    viber.ui.set("quests.others", false)
  end
  -- O número de XP e o brilho da missão só aparecem com a classe `shown`/
  -- `fresh`. Forçar false todos os frames apagava-os para sempre. Alvo,
  -- chefe, recargas e as outras missões são doutros scripts: zerá-los aqui
  -- ganhava a corrida e escondia a barra.
  local xp_sig = string.format("%d/%d/%d", game.level, math.floor(game.xp), math.floor(game.xp_next))
  if M._xp_sig ~= xp_sig then
    if M._xp_sig ~= nil then
      M._xp_until = viber.time() + REVEAL_SECS
    end
    M._xp_sig = xp_sig
  end
  viber.ui.set("xp.recent", (M._xp_until or 0) > viber.time())
  local quest_sig = title .. "\0" .. text
  if M._quest_sig ~= quest_sig then
    if M._quest_sig ~= nil then
      M._quest_until = viber.time() + REVEAL_SECS
    end
    M._quest_sig = quest_sig
  end
  viber.ui.set("quest.recent", (M._quest_until or 0) > viber.time())
end

return M
