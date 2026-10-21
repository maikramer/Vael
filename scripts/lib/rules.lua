-- Regras do Vael: HP, cofre, XP, quests, veneno, passivas e biomas.
-- TUDO em `viber.game()` (save) e `viber.game().enemies` (registro vivo).
-- A engine só expõe primitivas; não conhece vida, ouro nem missões.

local M = {}

local QUEST_FILES = {
  "quests/city_quests.json",
  "quests/dark_forest_quests.json",
  "quests/desert_quests.json",
  "quests/fantasy_quests.json",
  "quests/mountain_quests.json",
  "quests/story_quests.json",
  "quests/swamp_quests.json",
}

-- Chefes iguais a `combat::kill_xp`. O resto é 15, o mesmo piso do melee:
-- tabelas diferentes faziam a barra (regras) e os talentos (engine) subirem
-- em níveis distintos.
local KILL_XP = {
  boss = 300,
  ["bog-warden"] = 180,
  ["sand-worm"] = 160,
  witch = 140,
}
local STACK = 99
local RESOURCES = { gold = true, wood = true, stone = true }
-- Passivas do diário: os valores vivem aqui e em `SKILL_EFFECTS`; a engine
-- não tem árvore de talentos. O diário só gasta o ponto.
local SKILL_IDS = {
  "vitality1", "strength1", "agility1", "precision1",
  "vitality2", "strength2", "agility2", "precision2",
}
local REVEAL_SECS = 2.5
-- Veneno gravado no save: empurra-se UMA vez para a engine por sessão.
-- (A flag em viber.game() sobreviveria ao load e nunca reaplicava.)
local venom_pushed = false
-- `vault_add` espelha na engine, e a engine devolve um evento `collect`.
-- Sem este crédito o mesmo monte entrava outra vez no cofre do jogo.
local echo = {}

local function g()
  return viber.game()
end

local function num(key, default)
  local v = g()[key]
  if type(v) ~= "number" then
    return default or 0
  end
  return v
end

local function ensure_xp()
  local game = g()
  if type(game.xp) ~= "number" then game.xp = 0 end
  if type(game.xp_next) ~= "number" or game.xp_next < 1 then game.xp_next = 100 end
  if type(game.level) ~= "number" or game.level < 1 then game.level = 1 end
  if type(game.skill_points) ~= "number" then game.skill_points = 0 end
end

-- HP do herói vive no JOGO (`viber.game()`), não em `Health` da engine.
local function ensure_hp()
  local game = g()
  if type(game.max_hp) ~= "number" or game.max_hp < 1 then game.max_hp = 100 end
  if type(game.hp) ~= "number" then game.hp = game.max_hp end
  if game.hp > game.max_hp then game.hp = game.max_hp end
  if not game.spawn_pos then
    local has, x, y, z = viber.player_position()
    if has then
      game.spawn_pos = { x = x, y = y, z = z }
    end
  end
end

-- Passivas do diário (mesmos valores da antiga tabela da engine).
local SKILL_EFFECTS = {
  vitality1 = { max_hp = 20 },
  strength1 = { damage = 6 },
  agility1 = { speed = 0.10 },
  precision1 = { crit = 0.08 },
  vitality2 = { max_hp = 30 },
  strength2 = { damage = 10 },
  agility2 = { speed = 0.15 },
  precision2 = { crit = 0.10 },
}

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
  if x and y and z then
    g().spawn_pos = { x = x, y = y, z = z }
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

local defs
local function load_defs()
  if defs then
    return defs
  end
  defs = {}
  for _, path in ipairs(QUEST_FILES) do
    local ok, data = pcall(viber.load_json, path)
    if ok and type(data) == "table" then
      for _, def in ipairs(data) do
        if def.id then
          defs[#defs + 1] = def
        end
      end
    end
  end
  table.sort(defs, function(a, b) return a.id < b.id end)
  return defs
end

local function find_def(id)
  for _, def in ipairs(load_defs()) do
    if def.id == id then
      return def
    end
  end
  return nil
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

function M.quest_progress(id)
  local def = find_def(id)
  local need = 1
  if def and def.objective and def.objective.count then
    need = def.objective.count
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

local function progress_of(id)
  return num(qkey(id) .. ".n", 0)
end

local function set_progress(id, n)
  g()[qkey(id) .. ".n"] = n
end

local function mark_ready(def)
  local obj = objective(def)
  local need = obj.count or 1
  if progress_of(def.id) >= need and M.quest_state(def.id) == "active" then
    g()[qkey(def.id)] = "ready"
    viber.toast("Objetivo completo: " .. (def.title or def.id))
    viber.sound("notification")
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
  viber.sound("quest_accept")
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
  if type(rewards.items) == "table" then
    for _, item in ipairs(rewards.items) do
      local name, count = tostring(item):match("^([^:]+):?(%d*)$")
      if name and name ~= "" then
        M.item_add(name, tonumber(count) or 1)
      end
    end
  end
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
    local need = obj.count or 1
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
  end
  grant_rewards(def)
  local gold = (def.rewards or {}).gold or 0
  viber.toast("Quest entregue: " .. (def.title or id) .. (gold > 0 and (" (+" .. gold .. " ouro)") or ""))
  viber.sound("quest_complete")
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

function M.report_kill(kind)
  if not kind or kind == "" then
    return
  end
  for _, def in ipairs(load_defs()) do
    if M.quest_state(def.id) == "active" then
      local obj = objective(def)
      if obj.type == "kill" and kill_matches(kind, obj.target or "") then
        local need = obj.count or 1
        set_progress(def.id, math.min(need, progress_of(def.id) + 1))
        mark_ready(def)
      end
    end
  end
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
          set_progress(def.id, n)
          mark_ready(def)
          viber.toast("Marco visto: " .. place)
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
        local need = obj.count or 1
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

function M.vault_get(kind)
  return num("item." .. kind, 0)
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
    "potion", "antidote", "bomb", "korok-seed",
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
      return
    end
  end
  -- Sem leitura do cofre nativo. Kit de arranque (economy::STARTING_GOLD /
  -- STARTING_POTIONS) quando o save ainda não gravou estas chaves — um 0
  -- explícito (ouro gasto) não volta a encher.
  if not vault_get and not item_count
      and g()["item.gold"] == nil and g()["item.potion"] == nil then
    g()["item.gold"] = 30
    g()["item.potion"] = 1
  end
end

function M.vault_add(kind, amount, mirror)
  amount = amount or 1
  if type(amount) ~= "number" or amount ~= amount or amount <= 0 or not kind or kind == "" then
    return 0
  end
  local key = "item." .. kind
  local have = num(key, 0)
  local cap = RESOURCES[kind] and 1000000 or STACK
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
  if type(amount) ~= "number" or amount ~= amount or amount <= 0 then
    return false
  end
  local key = "item." .. kind
  local have = num(key, 0)
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
  if gain <= 0 then
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
    game.xp_next = math.max(1, math.ceil(game.xp_next * 1.5))
    game.level = game.level + 1
    game.skill_points = game.skill_points + 1
  end
end

function M.add_skill_points(n)
  n = n or 0
  if n <= 0 then
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
function M.hostiles(range)
  ensure_hp()
  local game = g()
  local ok, px, py, pz = viber.player_position()
  if not ok then
    return {}
  end
  local now = viber.time()
  local out = {}
  for id, e in pairs(game.enemies or {}) do
    if (e.hp or 0) > 0 and e.t and e.t <= now and (now - e.t) < 1.0 then
      local dx, dz = (e.x or px) - px, (e.z or pz) - pz
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

function M.damage_enemy(id, amount)
  ensure_hp()
  local e = (g().enemies or {})[id]
  if not e or (e.hp or 0) <= 0 then
    return false, e
  end
  e.hp = math.max(0, e.hp - (amount or 0))
  return e.hp <= 0, e
end

function M.alive_in_region(idx)
  local list = M.hostiles(4000)
  local n = 0
  for _, h in ipairs(list) do
    if region_of(h.x or 0, h.z or 0) == idx and (h.hp or 0) > 0 then
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
  return KILL_XP[kind] or 15
end

function M.award_kill(kind, id)
  M.add_xp(M.kill_xp(kind))
  M.report_kill(kind)
  if M.on_kill then
    M.on_kill(kind, id)
  end
end

-- Guarda/parry [L] — regra do jogo, em Lua. O dano dos inimigos passa
-- TODO por `damage_player`; a janela de parry é curta e o bloqueio reduz.
local GUARD_SECS = 0.5
local PARRY_WINDOW = 0.2
local GUARD_REDUCTION = 0.25

function M.guard()
  local game = g()
  game.guard_until = viber.time() + GUARD_SECS
  game.guard_start = viber.time()
end

local function guarding()
  return viber.time() < (g().guard_until or 0)
end

function M.damage_player(amount)
  if num("iframes", 0) > 0 or num("dying", 0) > 0 then
    return
  end
  ensure_hp()
  local game = g()
  amount = math.max(0, amount or 0)
  if guarding() and amount > 0 then
    local parried = viber.time() - (game.guard_start or 0) <= PARRY_WINDOW
    if parried then
      -- Aparo: o golpe não entra e o herói ganha um instante de i-frames.
      game.iframes = math.max(num("iframes", 0), 0.25)
      viber.toast("ESQUIVA")
      viber.sound("block")
      viber.damage_number("ESQUIVA", { color = "#9fd8ff" })
      return
    end
    amount = amount * GUARD_REDUCTION
    viber.toast("Bloqueaste")
    viber.sound("block")
  end
  game.hp = math.max(0, game.hp - amount)
  viber.damage_number("-" .. math.floor(amount), { color = "#ff6b6b" })
  viber.sound("hurt")
  viber.shake(0.18)
  game.iframes = 0.35
  if game.hp <= 0 then
    game.dying = 2.2
    local id = viber.player_id()
    if id then
      viber.play_clip("death", { id = id })
    end
    viber.sound("game_over")
  end
end

function M.heal_player(amount)
  ensure_hp()
  local game = g()
  game.hp = math.min(game.max_hp, game.hp + math.max(0, amount or 0))
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
  if viber.set_player_speed then
    viber.set_player_speed(stats.speed_mult)
  end
end

function M.tick(dt)
  M.pull_engine()
  sync_passives()
  ensure_hp()
  local game = g()
  game.iframes = math.max(0, num("iframes", 0) - dt)

  -- Veneno: 4 dps, tick de 1 s (antes era a engine).
  if num("venom", 0) > 0 then
    game.venom = math.max(0, game.venom - dt)
    game.venom_tick = (game.venom_tick or 0) + dt
    if game.venom_tick >= 1.0 then
      game.venom_tick = game.venom_tick % 1.0
      game.hp = math.max(0, game.hp - 4)
      viber.damage_number("-4", { color = "#7ed957" })
    end
  else
    game.venom_tick = 0
  end

  -- Morte do herói: 2,2 s caído e renasce no último ponto seguro.
  if num("dying", 0) > 0 then
    game.dying = math.max(0, game.dying - dt)
    if game.dying <= 0 then
      local s = game.spawn_pos or { x = 0, y = 150, z = 0 }
      viber.teleport_player(s.x, s.y, s.z)
      game.hp = game.max_hp
      game.venom = 0
      game.venom_tick = 0
      game.iframes = 2.0
      viber.toast("Renasces no último descanso.")
      viber.sound("levelup")
    end
  end

  for _, ev in ipairs(viber.events()) do
    if ev.type == "kill" and ev.name then
      M.add_xp(M.kill_xp(ev.name))
      M.report_kill(ev.name)
      viber.sound("enemy_death")
    elseif ev.type == "hit" then
      if ev.faction == "player" and ev.entity then
        local enemy = (g().enemies or {})[ev.entity]
        if enemy and (enemy.hp or 0) > 0 then
          enemy.hp = math.max(0, enemy.hp - (ev.amount or 0))
        end
        if ev.x then
          viber.damage_number("-" .. tostring(math.floor(ev.amount or 0)), {
            x = ev.x, y = (ev.y or 0) + 1.8, z = ev.z, color = "#8cd9ff",
          })
        end
      else
        M.damage_player(ev.amount or 0)
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
    elseif ev.type == "harvest" then
      -- A engine partiu o prop e publicou item/quantidade/posição.
      local item, amount = ev.item or "", ev.amount or 0
      if item ~= "" and amount > 0 then
        M.vault_add(item, amount, false)
        M.add_xp(30)
        viber.damage_number("+" .. amount .. " " .. item, {
          x = ev.x, y = (ev.y or 0) + 1.6, z = ev.z, color = "#8fe3a0",
        })
      end
    end
  end
  M.scan_visits()
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
  for _, def in ipairs(load_defs()) do
    local state = M.quest_state(def.id)
    if state == "active" or state == "ready" then
      local obj = objective(def)
      local need = obj.count or 1
      local have = progress_of(def.id)
      local score = (state == "ready" and 2 or 0) + (need > 0 and have / need or 0)
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
