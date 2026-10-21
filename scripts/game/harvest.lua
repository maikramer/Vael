-- game/harvest.lua: a COLHEITA é do jogo, em Lua.
--
-- A engine só transporta DADOS autorais: `viber.player_nearby` devolve as
-- entidades nomeadas à volta do herói com a tabela `props` (o
-- `destructible="…"`, `resource-kind` e `resource-yield` dos XMLs).
-- O parser do component-string, os golpes, o loot e o XP vivem aqui.
-- FX/remoção usam primitivas genéricas: `viber.prompt`, `viber.play_clip`,
-- `viber.burst`, `viber.sound`, `viber.entity_despawn` e `viber.topple`.
--
-- Host: `world/gameplay.xml` (`tag="always-active"` — a colheita não pode
-- congelar no LOD de 45 m).

local rules = viber.load("lib/rules.lua")

local SWING_COOLDOWN = 0.4
local RANGE_DEFAULT = 3.5
local SPEED = 1.4
local PICK_SPEED = 1.0
local IMPACT_FRACTION = 0.35
local IMPACT_MIN, IMPACT_MAX = 0.12, 0.45
local FALLBACK_DURATION = 0.5
local HARVEST_XP = 30
-- Protótipos que podem substituir um prop colhido. A lista é fechada: um
-- save antigo com um nome desconhecido não pode rebentar `spawn_prototype`.
local REPLACEMENTS = { ["berry-bush-bare"] = true }

local LABEL = { fall = "Cortar", pick = "Colher frutas" }
-- As tabelas de FX são indexadas pela FERRAMENTA (`tool_of`), não pelo
-- estilo: `CLIP["chop"]` era nil e cada golpe numa árvore rebentava em
-- `viber.clip_duration(nil)` (a árvore nunca caía).
local CLIP = { chop = "chop", mine = "mine", pick = "gather" }
local HIT_SFX = { chop = "chop_hit", mine = "mine_hit", pick = "harvest" }
local BREAK_SFX = { chop = "chop_break", mine = "mine_break", pick = "harvest" }
local HIT_BURST = { chop = "leaves", mine = "sparks", pick = "leaves" }
local BREAK_BURST = { chop = "leaves", mine = "ground-dust", pick = "leaves" }

local function tool_of(style)
  if style == "fall" then
    return "chop"
  end
  if style == "pick" then
    return "pick"
  end
  return "mine"
end

local function speed_of(tool)
  return tool == "pick" and PICK_SPEED or SPEED
end

local function label_of(style)
  return LABEL[style] or "Minerar"
end

local function duration_of(clip)
  local d = viber.clip_duration and viber.clip_duration(clip)
  if d and d > 0 then
    return d
  end
  return FALLBACK_DURATION
end

local function impact_delay(clip, tool)
  local d = duration_of(clip)
  local delay = d * IMPACT_FRACTION / speed_of(tool)
  return math.min(IMPACT_MAX, math.max(IMPACT_MIN, delay))
end

-- O prompt de outro [J] (um `ScriptInteraction` mais próximo) tem prioridade
-- sobre a colheita — lê-se o prompt que a engine publicou este frame.
local function other_j_prompt(style)
  local key = viber.ui.get("prompt.key")
  local label = viber.ui.get("prompt.label")
  return key == "J" and label ~= "" and label ~= label_of(style)
end

-- `destructible="hits: 3; break-style: fall; …"` é um component-string
-- autorado no mundo; o parser é do JOGO (a engine só transporta o texto).
local function parse_components(value)
  local out = {}
  for key, val in tostring(value or ""):gmatch("([%w%-_]+)%s*:%s*([^;]+)") do
    out[key:lower()] = val:match("^%s*(.-)%s*$")
  end
  return out
end

local function effective_range(base)
  if viber.interact_range then
    return viber.interact_range(base)
  end
  return base * 0.5
end

-- Dados de colheita de uma entidade de `viber.player_nearby`: os props
-- autorais (`destructible`, `resource-kind`, `resource-yield`).
local function prop_data(entity)
  local props = entity.props
  if not props then
    return nil
  end
  local raw = props.destructible
  local kind = props["resource-kind"]
  if (not raw or raw == "") and (not kind or kind == "") then
    return nil
  end
  local c = parse_components(raw)
  return {
    id = entity.id,
    name = entity.name or "",
    x = entity.x,
    y = entity.y,
    z = entity.z,
    distance = entity.distance or 0,
    hits = tonumber(c.hits) or 3,
    style = (c["break-style"] or "burst"):lower(),
    popup = c["popup-text"] or "",
    range = tonumber(c.range) or RANGE_DEFAULT,
    kind = kind or "",
    amount = tonumber(props["resource-yield"]) or 1,
    -- Troca visual opcional: a berry_bush com frutas vira a versão `bare`
    -- no mesmo sítio. `replace-scale` é a escala UNIFORME do prop original
    -- (as instâncias do Scatter têm escalas diferentes).
    replacement = c.replacement or "",
    replace_scale = tonumber(c["replace-scale"]) or 1,
  }
end

-- Prop mais próximo DENTRO do alcance efetivo autorado.
local function nearest(st)
  local list = viber.player_nearby(RANGE_DEFAULT * 2, 64) or {}
  local best
  for _, entity in ipairs(list) do
    local prop = prop_data(entity)
    if prop and not st.broken[prop.id] then
      if prop.distance <= effective_range(prop.range) then
        if not best or prop.distance < best.distance then
          best = prop
        end
      end
    end
  end
  return best
end

local function publish(prop)
  if prop then
    viber.prompt("J", label_of(prop.style))
  else
    viber.prompt("", "")
  end
end

local function swing(st, prop)
  local tool = tool_of(prop.style)
  local clip = CLIP[tool]
  st.cooldown = SWING_COOLDOWN
  st.swing = {
    t = impact_delay(clip, tool),
    id = prop.id,
    hits = prop.hits or 3,
    style = prop.style or "burst",
    kind = prop.kind or "",
    amount = prop.amount or 0,
    popup = prop.popup or "",
    name = prop.name or "",
    x = prop.x,
    y = prop.y,
    z = prop.z,
    replacement = prop.replacement or "",
    replace_scale = prop.replace_scale or 1,
  }
  local pid = viber.player_id()
  if pid then
    viber.play_clip(clip, { speed = speed_of(tool), id = pid })
  end
  viber.sound(HIT_SFX[tool] or "whoosh", {
    x = prop.x, y = prop.y, z = prop.z,
  })
end

local function break_prop(st, sw)
  local style = sw.style or "burst"
  local tool = tool_of(style)
  viber.burst(BREAK_BURST[tool] or "ground-dust", sw.x, (sw.y or 0) + 0.8, sw.z, 24)
  viber.sound(BREAK_SFX[tool] or "chop_break", { x = sw.x, y = sw.y, z = sw.z })
  if sw.popup ~= "" then
    local text = sw.popup
    if tool == "pick" and sw.amount > 1 then
      text = "+" .. sw.amount .. " " .. text
    end
    viber.damage_number(text, {
      color = "#ffd35a", x = sw.x, y = (sw.y or 0) + 1.4, z = sw.z,
    })
  end
  -- Loot e XP: regra do jogo, não da engine. `vault_add` já sincroniza os
  -- objetivos `collect` das missões.
  if sw.kind ~= "" and sw.amount > 0 then
    rules.vault_add(sw.kind, sw.amount)
  end
  rules.add_xp(HARVEST_XP)
  -- Persistência: o load volta a spawnar o prop; o controller apaga-o no
  -- arranque se o nome estiver no save (props nomeados). Guarda-se também a
  -- pose/escala do substituto: a árvore sem frutas é um PROTÓTIPO e não
  -- sobrevive sozinha a um reload.
  if sw.name ~= "" then
    local game = viber.game()
    game["harvested." .. sw.name] = true
    st.reaped_names = st.reaped_names or {}
    st.reaped_names[sw.name] = true
    if sw.replacement ~= "" then
      game["stump." .. sw.name .. ".proto"] = sw.replacement
      game["stump." .. sw.name .. ".x"] = sw.x
      game["stump." .. sw.name .. ".y"] = sw.y or 0
      game["stump." .. sw.name .. ".z"] = sw.z
      game["stump." .. sw.name .. ".scale"] = sw.replace_scale or 1
    end
  end
  if sw.replacement ~= "" then
    -- A árvore com frutas sai e a versão sem frutas entra na mesma pose.
    -- O protótipo é spawned no pós-frame; `on_spawned` acerta pose/escala.
    local px, py, pz = sw.x, sw.y, sw.z
    local scale = sw.replace_scale or 1
    viber.entity_despawn(sw.id)
    viber.spawn_prototype(sw.replacement, px, pz, {
      y = py,
      on_spawned = function(bits)
        if py ~= nil then
          viber.set_pose(px, py, pz, 0, bits)
        end
        viber.set_scale(scale, bits)
      end,
    })
  elseif style == "fall" then
    viber.topple(sw.id)
  else
    viber.entity_despawn(sw.id)
  end
  st.broken[sw.id] = true
  st.hits[sw.id] = nil
end

local function impact(st)
  local sw = st.swing
  st.swing = nil
  if not sw then
    return
  end
  local hits = (st.hits[sw.id] or 0) + 1
  if hits >= sw.hits then
    break_prop(st, sw)
    return
  end
  st.hits[sw.id] = hits
  local tool = tool_of(sw.style)
  viber.burst(HIT_BURST[tool] or "sparks", sw.x, (sw.y or 0) + 1.0, sw.z, 7)
  if viber.hit_flash then
    viber.hit_flash(sw.id)
  end
end

-- A cada frame, apaga props nomeados que o save diz estarem colhidos e repõe
-- o substituto (a árvore sem frutas) na pose guardada. NÃO é one-shot: o
-- botão Carregar do menu pode trazer o save DEPOIS do arranque do script.
-- `st.reaped_names` evita repetir o spawn quando a colheita acabou de
-- acontecer nesta sessão (a chave `harvested.*` já lá está).
local function reap_saved(st)
  local game = viber.game()
  st.reaped_names = st.reaped_names or {}
  for key in pairs(game) do
    if type(key) == "string" and key:sub(1, 10) == "harvested." then
      local name = key:sub(11)
      if name ~= "" and not st.reaped_names[name] then
        st.reaped_names[name] = true
        local id = viber.find(name)
        if id then
          viber.entity_despawn(id)
        end
        local proto = game["stump." .. name .. ".proto"]
        local x = game["stump." .. name .. ".x"]
        local y = game["stump." .. name .. ".y"]
        local z = game["stump." .. name .. ".z"]
        local scale = game["stump." .. name .. ".scale"]
        if type(proto) == "string" and REPLACEMENTS[proto]
            and type(x) == "number" and type(z) == "number" then
          viber.spawn_prototype(proto, x, z, {
            y = type(y) == "number" and y or nil,
            on_spawned = function(bits)
              if type(y) == "number" then
                viber.set_pose(x, y, z, 0, bits)
              end
              if type(scale) == "number" then
                viber.set_scale(scale, bits)
              end
            end,
          })
        end
      end
    end
  end
end

function on_update(dt)
  local st = viber.state()
  st.hits = st.hits or {}
  st.broken = st.broken or {}
  st.reap_t = (st.reap_t or 0) - dt
  if st.reap_t <= 0 then
    st.reap_t = 1.0
    reap_saved(st)
  end
  st.cooldown = math.max(0, (st.cooldown or 0) - dt)
  if st.swing then
    st.swing.t = st.swing.t - dt
    if st.swing.t <= 0 then
      impact(st)
    end
  end
  if viber.menu_open() then
    viber.prompt("", "")
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    viber.prompt("", "")
    return
  end
  local prop = nearest(st)
  if prop and other_j_prompt(prop.style) then
    prop = nil
  end
  publish(prop)
  local pressed = viber.input.pressed("j") or viber.input.pressed("lmb")
  if not pressed or not prop then
    return
  end
  if st.cooldown > 0 or st.swing then
    return
  end
  swing(st, prop)
end
