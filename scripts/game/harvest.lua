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

-- A berry_bush gerada pelo Scatter perdeu o `resource-kind`/`resource-yield`
-- do template (só o `<GameObject>` os declara; as instâncias `<GltfScene>`
-- não). Sem este fallback a colheita dava XP e o popup "Frutas", mas zero
-- frutas para o cofre. Só o estilo `pick` usa este par.
local PICK_FALLBACK_KIND = "berry"
local PICK_FALLBACK_YIELD = 3
local POPUP_COLOR = "#ffd35a"

local LABEL = { fall = "Cortar", pick = "Colher frutas" }
-- As tabelas de FX são indexadas pela FERRAMENTA (`tool_of`), não pelo
-- estilo: `CLIP["chop"]` era nil e cada golpe numa árvore rebentava em
-- `viber.clip_duration(nil)` (a árvore nunca caía).
local CLIP = { chop = "chop", mine = "mine", pick = "gather" }
local HIT_SFX = { chop = "chop_hit", mine = "mine_hit", pick = "harvest" }
local BREAK_SFX = { chop = "chop_break", mine = "mine_break", pick = "harvest" }
local HIT_BURST = { chop = "leaves", mine = "sparks", pick = "leaves" }
local BREAK_BURST = { chop = "leaves", mine = "ground-dust", pick = "leaves" }

-- `break-style: shatter` é a pedra E o tronco caído. O tipo do recurso
-- decide a ferramenta: sem isto a lenha (resource-kind="wood") era picada
-- com a picareta, com o prompt "Minerar" e o som/FX de pedra.
local function tool_of(style, kind)
  if style == "fall" then
    return "chop"
  end
  if style == "pick" then
    return "pick"
  end
  if kind == "wood" or kind == "dark-wood" then
    return "chop"
  end
  return "mine"
end

local function speed_of(tool)
  return tool == "pick" and PICK_SPEED or SPEED
end

local function label_of(style, kind)
  if style == "fall" or kind == "wood" or kind == "dark-wood" then
    return LABEL.fall
  end
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
local function other_j_prompt(style, kind)
  local key = viber.ui.get("prompt.key")
  local label = viber.ui.get("prompt.label")
  return key == "J" and label ~= "" and label ~= label_of(style, kind)
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
  local raw_kind = props["resource-kind"]
  if (not raw or raw == "") and (not raw_kind or raw_kind == "") then
    return nil
  end
  local c = parse_components(raw)
  local style = (c["break-style"] or "burst"):lower()
  local kind = raw_kind or ""
  -- As berry_bush do Scatter não trazem `resource-kind`/`resource-yield`;
  -- o estilo `pick` (ou o substituto `berry-bush-bare`) diz o que são e o
  -- template autorava 3 frutas.
  local is_berry = style == "pick" or c.replacement == "berry-bush-bare"
  if kind == "" and is_berry then
    kind = PICK_FALLBACK_KIND
  end
  local amount = tonumber(props["resource-yield"])
  if not amount then
    amount = is_berry and PICK_FALLBACK_YIELD or 1
  end
  local popup_color = c["popup-color"]
  if type(popup_color) ~= "string" or not popup_color:match("^#%x%x%x%x%x%x%x?%x?$") then
    popup_color = POPUP_COLOR
  end
  return {
    id = entity.id,
    name = entity.name or "",
    x = entity.x,
    y = entity.y,
    z = entity.z,
    distance = entity.distance or 0,
    hits = tonumber(c.hits) or 3,
    style = style,
    popup = c["popup-text"] or "",
    popup_color = popup_color,
    range = tonumber(c.range) or RANGE_DEFAULT,
    kind = kind,
    amount = amount,
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

local function publish(st, prop)
  if prop then
    viber.prompt("J", label_of(prop.style, prop.kind))
    st.prompting = true
  elseif st.prompting then
    -- Só limpa o que ESTE controller publicou: um prompt nativo (mercador,
    -- árvore de frutas) é do collect da engine e não pode ser pisado.
    viber.prompt("", "")
    st.prompting = false
  end
end

-- O `game/equipment.lua` lê `harvest.tool` para pôr a ferramenta na mão
-- (chop→machado, mine→picareta). `pick` (frutas) é de mãos livres.
local function publish_tool(st, prop)
  local tool = ""
  if prop then
    tool = tool_of(prop.style, prop.kind)
    if tool == "pick" then
      tool = ""
    end
  end
  if st.tool == tool then
    return
  end
  st.tool = tool
  viber.ui.set("harvest.tool", tool)
end

local function swing(st, prop)
  local tool = tool_of(prop.style, prop.kind)
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
    popup_color = prop.popup_color or POPUP_COLOR,
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
end

local function break_prop(st, sw)
  local style = sw.style or "burst"
  local tool = tool_of(style, sw.kind)
  -- A lista de substitutos é FECHADA: um `replacement` com gralha não pode
  -- chamar `spawn_prototype` (warn) nem ficar gravado no save como stump.
  local replacement = REPLACEMENTS[sw.replacement or ""] and (sw.replacement or "") or ""
  viber.burst(BREAK_BURST[tool] or "ground-dust", sw.x, (sw.y or 0) + 0.8, sw.z, 24)
  viber.sound(BREAK_SFX[tool] or "chop_break", { x = sw.x, y = sw.y, z = sw.z })
  -- Loot e XP: regra do jogo, não da engine. `vault_add` já sincroniza os
  -- objetivos `collect` das missões. O crédito vem ANTES do popup: a pilha
  -- de frutas tem teto de 99, e mostrar "+3" quando só entrou 1 (ou nada)
  -- era mentira — e o prop já foi consumido.
  local got = 0
  if sw.kind ~= "" and sw.amount > 0 then
    got = rules.vault_add(sw.kind, sw.amount) or 0
  end
  if sw.popup ~= "" then
    local text = sw.popup
    if got > 0 then
      text = "+" .. got .. " " .. text
    elseif sw.kind ~= "" then
      text = text .. " (bolsa cheia)"
    end
    viber.damage_number(text, {
      color = sw.popup_color or POPUP_COLOR, x = sw.x, y = (sw.y or 0) + 1.4, z = sw.z,
    })
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
    if replacement ~= "" then
      game["stump." .. sw.name .. ".proto"] = replacement
      game["stump." .. sw.name .. ".x"] = sw.x
      game["stump." .. sw.name .. ".y"] = sw.y or 0
      game["stump." .. sw.name .. ".z"] = sw.z
      game["stump." .. sw.name .. ".scale"] = sw.replace_scale or 1
    end
  end
  if replacement ~= "" then
    -- A árvore com frutas sai e a versão sem frutas entra na mesma pose.
    -- O protótipo é spawned no pós-frame; `on_spawned` acerta pose/escala.
    local px, py, pz = sw.x, sw.y, sw.z
    local scale = sw.replace_scale or 1
    viber.entity_despawn(sw.id)
    viber.spawn_prototype(replacement, px, pz, {
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
  -- O `st.near` em cache (10 Hz) ainda apontava para este prop: sem isto o
  -- prompt/ferramenta ficavam no ecrã até à varredura seguinte.
  st.near = nil
  st.scan_t = 0
end

local function impact(st)
  local sw = st.swing
  st.swing = nil
  if not sw then
    return
  end
  -- O prop pode ter desaparecido entre o golpe e o impacto (outro sistema,
  -- despawn, troca de cena): topple/despawn com bits mortos rebenta o chunk.
  local exists = viber.entity_position(sw.id)
  if not exists then
    st.hits[sw.id] = nil
    -- O alvo em cache também já não existe: o prompt não pode ficar no ecrã.
    st.near = nil
    return
  end
  local hits = (st.hits[sw.id] or 0) + 1
  local tool = tool_of(sw.style, sw.kind)
  if hits >= sw.hits then
    break_prop(st, sw)
    return
  end
  st.hits[sw.id] = hits
  viber.sound(HIT_SFX[tool] or "whoosh", {
    x = sw.x, y = sw.y, z = sw.z,
  })
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
        -- Só conta como reposto quando a entidade EXISTE nesta cena. Marcar
        -- antes de a encontrar deixava a árvore colhida de fora do save:
        -- ao entrar na cena onde ela vive, `reaped_names` já a saltava e a
        -- árvore com frutas reaparecia.
        local id = viber.find(name)
        if id then
          st.reaped_names[name] = true
          viber.entity_despawn(id)
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
  -- Menu aberto ou herói caído: o golpe que estava armado não aterra por
  -- baixo (antes o `impact` corria primeiro e a árvore caía com o diário
  -- aberto ou depois da morte).
  if viber.menu_open() then
    st.swing = nil
    publish_tool(st, nil)
    publish(st, nil)
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    st.swing = nil
    publish_tool(st, nil)
    publish(st, nil)
    return
  end
  if st.swing then
    st.swing.t = st.swing.t - dt
    if st.swing.t <= 0 then
      impact(st)
    end
  end
  -- `player_nearby` varre o snapshot de ~79k entidades nomeadas. A 60 Hz
  -- era o maior custo de CPU deste controller; 10 Hz chegam para o prompt
  -- (o golpe revalida o alvo no frame do J).
  st.scan_t = (st.scan_t or 0) - dt
  if st.scan_t <= 0 then
    st.scan_t = 0.1
    st.near = nearest(st)
  end
  local prop = st.near
  if prop and other_j_prompt(prop.style, prop.kind) then
    -- Outro [J] nativo é dono do prompt: nem publica nem limpa.
    prop = nil
    st.prompting = false
  end
  publish_tool(st, prop)
  publish(st, prop)
  -- O rato é do combate ([J]/clique). A colheita responde só ao J: com o
  -- clique também a colher, um clique perto de uma árvore disparava o golpe
  -- de espada E o machado no mesmo frame.
  local pressed = viber.input.pressed("j")
  if not pressed or not prop then
    return
  end
  if st.cooldown > 0 or st.swing then
    return
  end
  -- O `st.near` é um retrato de há até 0,1 s: um passo/dash podia deixar o
  -- alvo fora de alcance (o golpe aterrava à distância). Revalida no frame
  -- do J — uma varredura por pressão, não por frame.
  local target = nearest(st)
  if not target or other_j_prompt(target.style, target.kind) then
    return
  end
  st.near = target
  swing(st, target)
end
