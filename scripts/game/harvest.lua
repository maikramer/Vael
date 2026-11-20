-- game/harvest.lua: a COLHEITA é do jogo, em Lua.
--
-- A engine só transporta DADOS autorais: `vael.player_nearby` devolve as
-- entidades nomeadas à volta do herói com a tabela `props` (o
-- `destructible="…"`, `resource-kind` e `resource-yield` dos XMLs).
-- O parser do component-string, os golpes, o loot e o XP vivem aqui.
-- FX/remoção usam primitivas genéricas: `vael.prompt`, `vael.play_clip`,
-- `vael.burst`, `vael.sound`, `vael.entity_despawn` e `vael.topple`.
--
-- Host: `world/gameplay.xml` (`tag="always-active"` — a colheita não pode
-- congelar no LOD de 45 m).

local rules = vael.load("lib/rules.lua")
local feel
do
  local ok, mod = pcall(vael.load, "lib/feel.lua")
  if ok and type(mod) == "table" and type(mod.blow) == "function" then
    feel = mod
  end
end

local SWING_COOLDOWN = 0.4
local RANGE_DEFAULT = 3.5
local SPEED = 1.4
local PICK_SPEED = 1.0
-- No clip chop de 1,50 s a lâmina cruza a frente do herói aos 0,43 s.
local CHOP_DURATION = 0.90
local MINE_DURATION = 0.98
-- Referência de movimento e fallback para nós antigos sem colisor.
local MINE_CONTACT = MINE_DURATION * 0.41
local CHOP_CONTACT = CHOP_DURATION * (0.43 / 1.50)
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
-- `vael.clip_duration(nil)` (a árvore nunca caía).
local CLIP = { chop = "chop", mine = "mine", pick = "gather" }
local HIT_SFX = { chop = "chop_hit", mine = "mine_hit", pick = "harvest" }
local BREAK_SFX = { chop = "chop_break", mine = "mine_break", pick = "harvest" }
local HIT_BURST = { mine = "sparks", pick = "leaves" }
local BREAK_BURST = { mine = "ground-dust", pick = "leaves" }

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
  local d = vael.clip_duration and vael.clip_duration(clip)
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
  local key = vael.ui.get("prompt.key")
  local label = vael.ui.get("prompt.label")
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
  if vael.interact_range then
    return vael.interact_range(base)
  end
  return base * 0.5
end

-- Dados de colheita de uma entidade de `vael.player_nearby`: os props
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
  -- O `resource-yield` é texto autoral sem tipagem: 0/negativo/não finito
  -- deixava o popup a mentir "(bolsa cheia)" e um valor fraccionário entrava
  -- no cofre como 2,5. O template (`<ResourceNode yield>`) já usa inteiro >= 1.
  local amount = tonumber(props["resource-yield"])
  if not amount or amount ~= amount or amount < 1 or amount == math.huge then
    amount = is_berry and PICK_FALLBACK_YIELD or 1
  else
    amount = math.floor(amount)
  end
  -- `hits` não finito nunca parte o prop; 0/negativo partia ao primeiro golpe.
  local hits = tonumber(c.hits)
  if not hits or hits ~= hits or hits == math.huge then
    hits = 3
  else
    hits = math.max(1, math.floor(hits))
  end
  -- O `parse_color` da engine aceita 3 ou 6 dígitos (não 7/8): uma cor
  -- "válida" para o regex antigo mas inválida para o `vael.damage_number`
  -- rebentava o `break_prop` DEPOIS de creditar loot/XP — o prop ficava no
  -- mundo e podia ser colhido outra vez, para sempre.
  local popup_color = c["popup-color"]
  if type(popup_color) ~= "string"
      or not (popup_color:match("^#%x%x%x$") or popup_color:match("^#%x%x%x%x%x%x$")) then
    popup_color = POPUP_COLOR
  end
  -- `vael.interact_range` REJEITA um raio negativo/não finito (erro em cada
  -- frame, a colheita inteira morria) e 0 deixava o prop inalcançável.
  local range = tonumber(c.range)
  if not range or range ~= range or range <= 0 or range == math.huge then
    range = RANGE_DEFAULT
  end
  -- `vael.set_scale` (no `on_spawned` do substituto) só aceita 0.05..20;
  -- um `replace-scale` fora do intervalo rebentava a callback e o toco
  -- ficava com a escala por omissão.
  local replace_scale = tonumber(c["replace-scale"])
  if not replace_scale or replace_scale ~= replace_scale
      or replace_scale < 0.05 or replace_scale > 20 then
    replace_scale = 1
  end
  local height = tonumber(c["cut-height"])
  if not height or not (height > 0 and height < math.huge) then height = 0.5 end
  return {
    id = entity.id,
    name = entity.name or "",
    x = entity.x,
    y = entity.y,
    z = entity.z,
    distance = entity.distance or 0,
    hits = hits,
    cut_height = math.clamp(height, 0.05, 3),
    style = style,
    popup = c["popup-text"] or "",
    popup_color = popup_color,
    range = range,
    kind = kind,
    amount = amount,
    -- Troca visual opcional: a berry_bush com frutas vira a versão `bare`
    -- no mesmo sítio. `replace-scale` é a escala UNIFORME do prop original
    -- (as instâncias do Scatter têm escalas diferentes).
    replacement = c.replacement or "",
    replace_scale = replace_scale,
  }
end

-- Prop mais próximo DENTRO do alcance efetivo autorado.
local function nearest(st)
  local list = vael.player_nearby(RANGE_DEFAULT * 2, 64) or {}
  local best
  for _, entity in ipairs(list) do
    local prop = prop_data(entity)
    local native_tool = prop and vael.vael.harvest_tool_focus and tool_of(prop.style, prop.kind) ~= "pick"
    if prop and not native_tool and not st.broken[prop.id] and prop.id ~= st.just_felled then
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
    vael.prompt("J", label_of(prop.style, prop.kind))
    st.prompting = true
  elseif st.prompting then
    -- Só limpa o que ESTE controller publicou: um prompt nativo (mercador,
    -- árvore de frutas) é do collect da engine e não pode ser pisado.
    vael.prompt("", "")
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
  local aim = tool == "mine" and prop.id or nil
  if st.mine_aim ~= aim and vael.vael and vael.vael.aim_pick then
    vael.vael.aim_pick(aim)
    st.mine_aim = aim
  end
  if st.tool == tool then
    return
  end
  st.tool = tool
  vael.ui.set("harvest.tool", tool)
end

local function swing(st, prop)
  local tool = tool_of(prop.style, prop.kind)
  local clip = CLIP[tool]
  local duration = tool == "chop" and CHOP_DURATION or tool == "mine" and MINE_DURATION
  local contact = tool == "chop" and CHOP_CONTACT or MINE_CONTACT
  st.cooldown = duration and duration * 0.86 or SWING_COOLDOWN
  if duration then st.grip_t = duration end
  st.swing = {
    t = duration and contact or impact_delay(clip, tool),
    range = effective_range(prop.range),
    cut_height = prop.cut_height,
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
  if tool == "mine" and vael.vael and vael.vael.begin_mine then
    vael.vael.begin_mine(prop.id)
    st.swing.probe = true
  end
  local pid = vael.player_id()
  if pid then
    if duration then
      vael.play_clip(clip, { duration = duration, blend = 0.065, id = pid })
      local ok, px, _, pz = vael.player_position()
      if ok and vael.vael and vael.vael.attack_motion then
        vael.vael.attack_motion({ secs = contact + 0.08, slow = 0.25,
          fx = prop.x - px, fz = prop.z - pz })
      end
    else
      vael.play_clip(clip, { speed = speed_of(tool), id = pid })
    end
  end
end

local function break_prop(st, sw, emit)
  local style = sw.style or "burst"
  local tool = tool_of(style, sw.kind)
  -- A lista de substitutos é FECHADA: um `replacement` com gralha não pode
  -- chamar `spawn_prototype` (warn) nem ficar gravado no save como stump.
  local replacement = REPLACEMENTS[sw.replacement or ""] and (sw.replacement or "") or ""
  -- Loot e XP: regra do jogo, não da engine. `vault_add` já sincroniza os
  -- objetivos `collect` das missões. O crédito é TENTADO antes de partir o
  -- prop: a pilha de frutas/madeira escura tem teto de 99 e o `vault_add`
  -- aceita só a parte que cabe. Partir o prop e marcar "harvested" com a
  -- pilha cheia perdia o resto para sempre (e pagava XP por nada); o
  -- crédito parcial é devolvido e o prop fica no mundo para depois de
  -- fazer espaço. O teste é o próprio `vault_add` para valer também nos
  -- recursos, cujo tecto (`resource_cap`) não é o `stack_max`.
  local got = 0
  if sw.kind ~= "" and sw.amount > 0 then
    got = rules.vault_add(sw.kind, sw.amount) or 0
    if got < sw.amount then
      if got > 0 then
        rules.vault_take(sw.kind, got)
      end
      if emit then emit(false) end
      vael.damage_number("Bolsa cheia", {
        color = "#ffb347", x = sw.x, y = (sw.y or 0) + 1.4, z = sw.z,
      })
      vael.sound("error", { x = sw.x, y = sw.y, z = sw.z })
      return
    end
  end
  if emit then emit(true) end
  if tool ~= "chop" then
    vael.burst(BREAK_BURST[tool] or "ground-dust", sw.x, (sw.y or 0) + 0.8, sw.z, 24)
  end
  if vael.vael and vael.vael.collect_burst then
    local vkind = sw.kind ~= "" and sw.kind or (tool == "pick" and "berry" or tool == "chop" and "wood" or "stone")
    local count = math.clamp(got > 0 and got or (sw.amount or 3), 2, 6)
    vael.vael.collect_burst(vkind, sw.x, (sw.y or 0) + 0.75, sw.z, count)
  end
  local fell = replacement == "" and style == "fall" and vael.vael and vael.vael.fell_tree
  vael.sound(fell and "chop_hit" or BREAK_SFX[tool] or "chop_break", { x = sw.x, y = sw.y, z = sw.z })
  if sw.popup ~= "" then
    local text = sw.popup
    if got > 0 then
      text = "+" .. got .. " " .. text
    end
    vael.damage_number(text, {
      color = sw.popup_color or POPUP_COLOR, x = sw.x, y = (sw.y or 0) + 1.4, z = sw.z,
    })
  end
  rules.add_xp(HARVEST_XP)
  -- Persistência: o load volta a spawnar o prop; o controller apaga-o no
  -- arranque se o nome estiver no save (props nomeados). Guarda-se também a
  -- pose/escala do substituto: a árvore sem frutas é um PROTÓTIPO e não
  -- sobrevive sozinha a um reload.
  if sw.name ~= "" and not fell then
    local game = vael.game()
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
    vael.entity_despawn(sw.id)
    vael.spawn_prototype(replacement, px, pz, {
      y = py,
      on_spawned = function(bits)
        if py ~= nil then
          vael.set_pose(px, py, pz, 0, bits)
        end
        vael.set_scale(scale, bits)
      end,
    })
  elseif style == "fall" then
    -- Árvore procedural: `vael.vael.fell_tree` (Rust) move a árvore para uma
    -- raiz com a pose global e pede o `FellTree` do bevy_flora — toco, queda e
    -- tronco deitado. O `vael.topple` da engine fica como fallback para
    -- props antigos (tomba e despawna, sem toco).
    if vael.vael and vael.vael.fell_tree then
      vael.vael.fell_tree(sw.id)
    else
      vael.topple(sw.id)
    end
  else
    vael.entity_despawn(sw.id)
  end
  if fell then
    -- A estação perde os props no pós-frame. O mesmo id volta a ser colhível após o rebrote.
    st.just_felled = sw.id
    st.fell_guard = 0.4
  else
    st.broken[sw.id] = true
  end
  -- Partir o prop muda o sítio. O lascar intermédio não pára o tempo.
  if feel then
    feel.blow("heavy")
  end
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
  local exists, x, y, z = vael.entity_position(sw.id)
  if not exists then
    st.hits[sw.id] = nil
    -- O alvo em cache também já não existe: o prompt não pode ficar no ecrã.
    st.near = nil
    return
  end
  local hits = (st.hits[sw.id] or 0) + 1
  local tool = tool_of(sw.style, sw.kind)
  local emit
  if tool == "chop" or tool == "mine" then
    local ok, px, py, pz = vael.player_position()
    if not ok then return end
    local dx, dy, dz = px - x, py - y, pz - z
    local dodge = vael.game().hero_dodge
    local dodge_left = type(dodge) == "table" and ((tonumber(dodge.until_t) or 0) - vael.time()) or 0
    local reach = sw.range or effective_range(RANGE_DEFAULT)
    if dx * dx + dy * dy + dz * dz > reach * reach then
      -- O [J] acertou no prop e o passo saiu do alcance: sem linha, a tecla
      -- parece partida.
      vael.damage_number("Fora de alcance", {
        color = "#c8c2b4", x = x, y = (y or 0) + 1.3, z = z,
      })
      return
    end
    if dodge_left > 0 and dodge_left < 1 then return end
    local fx = vael.vael and vael.vael[tool == "chop" and "wood_hit" or "rock_hit"]
    if fx then
      emit = function(final_hit) fx(sw.id, x, y, z, dx, dz, sw.cut_height or 0.5, final_hit) end
    end
  end
  if hits >= sw.hits then
    break_prop(st, sw, emit)
    return
  end
  if emit then emit(false) end
  if vael.vael and vael.vael.hit_flash then
    vael.vael.hit_flash(sw.id)
  end
  st.hits[sw.id] = hits
  vael.sound(HIT_SFX[tool] or "whoosh", {
    x = sw.x, y = sw.y, z = sw.z,
  })
  if tool ~= "pick" then return end
  vael.burst(HIT_BURST[tool] or "sparks", sw.x, (sw.y or 0) + 1.0, sw.z, 7)
  if vael.hit_flash then
    vael.hit_flash(sw.id)
  end
end

-- A cada frame, apaga props nomeados que o save diz estarem colhidos e repõe
-- o substituto (a árvore sem frutas) na pose guardada. NÃO é one-shot: o
-- botão Carregar do menu pode trazer o save DEPOIS do arranque do script.
-- `st.reaped_names` evita repetir o spawn quando a colheita acabou de
-- acontecer nesta sessão (a chave `harvested.*` já lá está).
local function reap_saved(st)
  local game = vael.game()
  st.reaped_names = st.reaped_names or {}
  local flora_names = {}
  for key in pairs(game) do
    if type(key) == "string" then
      local name = key:match("^flora%-fell%.([^@]+)@.+%.dx$")
      if name then flora_names[name] = true end
    end
  end
  for key in pairs(game) do
    if type(key) == "string" and key:sub(1, 10) == "harvested." then
      local name = key:sub(11)
      -- Árvore tombada com o `bevy_flora` (`flora-fell.<nome>.*`): o restauro
      -- é do Rust (`reap_saved`), que a deita com toco em vez de a apagar.
      local flora_fell = name ~= "" and game["flora-fell." .. name .. ".dx"] ~= nil
      if name ~= "" and not flora_names[name] and not flora_fell and not st.reaped_names[name] then
        -- Só conta como reposto quando a entidade EXISTE nesta cena. Marcar
        -- antes de a encontrar deixava a árvore colhida de fora do save:
        -- ao entrar na cena onde ela vive, `reaped_names` já a saltava e a
        -- árvore com frutas reaparecia.
        local id = vael.find(name)
        if id then
          st.reaped_names[name] = true
          vael.entity_despawn(id)
          local proto = game["stump." .. name .. ".proto"]
          local x = game["stump." .. name .. ".x"]
          local y = game["stump." .. name .. ".y"]
          local z = game["stump." .. name .. ".z"]
          local scale = game["stump." .. name .. ".scale"]
          if type(proto) == "string" and REPLACEMENTS[proto]
              and type(x) == "number" and type(z) == "number" then
            vael.spawn_prototype(proto, x, z, {
              y = type(y) == "number" and y or nil,
              on_spawned = function(bits)
                if type(y) == "number" then
                  vael.set_pose(x, y, z, 0, bits)
                end
                if type(scale) == "number" then
                  vael.set_scale(scale, bits)
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
  -- Portão da migração: com `VAEL_HARVEST_RUST=1` a colheita corre em Rust
  -- (`src/harvest.rs`) e este controller cala-se por inteiro. Sem o flag o
  -- caminho é o de sempre (nada abaixo mudou).
  if vael.game().vael_harvest_rust == true then
    return
  end
  local st = vael.state()
  st.hits = st.hits or {}
  st.broken = st.broken or {}
  st.reap_t = (st.reap_t or 0) - dt
  if st.reap_t <= 0 then
    st.reap_t = 1.0
    reap_saved(st)
  end
  st.grip_t = math.max(0, (st.grip_t or 0) - dt)
  st.fell_guard = math.max(0, (st.fell_guard or 0) - dt)
  if st.fell_guard == 0 then st.just_felled = nil end
  st.cooldown = math.max(0, (st.cooldown or 0) - dt)
  -- Título aberto (vive sobre `main` no arranque directo): [J] é da UI e o
  -- herói está parado atrás do véu. Sem o portão, machado/picareta batiam
  -- por baixo do título — XP e itens para uma partida que nem começou.
  -- (`vael.menu_open()` já cobre diário/mapa/cinemática; o título não.)
  if (vael.scene_name and vael.scene_name() == "menu")
      or (vael.ui and vael.ui.is_open and vael.ui.is_open("title")) then
    st.swing = nil
    st.grip_t = 0
    publish_tool(st, nil)
    publish(st, nil)
    return
  end
  -- Menu aberto ou herói caído: o golpe que estava armado não aterra por
  -- baixo (antes o `impact` corria primeiro e a árvore caía com o diário
  -- aberto ou depois da morte).
  if vael.menu_open() or vael.ui.get("boat.grip") == "oars" then
    st.swing = nil
    st.grip_t = 0
    publish_tool(st, nil)
    publish(st, nil)
    return
  end
  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    st.swing = nil
    st.grip_t = 0
    publish_tool(st, nil)
    publish(st, nil)
    return
  end
  if st.swing then
    st.swing.t = st.swing.t - dt
    local contact = st.swing.probe and vael.vael.mine_contact(st.swing.id) or "unshaped"
    if contact == "hit" or (contact == "unshaped" and st.swing.t <= 0) then
      impact(st)
    elseif contact == "miss" or st.swing.t < -0.65 then
      st.swing = nil
    end
  end
  -- O machado e a picareta usam o foco físico publicado antes de ler o ataque.
  if vael.vael.harvest_tool_focus and vael.vael.harvest_tool_focus() then
    st.near, st.tool, st.prompting, st.scan_t = nil, nil, false, 0
    return
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
  if st.grip_t <= 0 then publish_tool(st, prop) end
  publish(st, prop)
  -- O rato é do combate ([J]/clique). A colheita responde só ao J: com o
  -- clique também a colher, um clique perto de uma árvore disparava o golpe
  -- de espada E o machado no mesmo frame.
  local pressed = vael.input.pressed("j")
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
    if not target and prop and not other_j_prompt(prop.style, prop.kind) then
      local ok, hx, hy, hz = vael.player_position()
      if ok then
        vael.damage_number("Fora de alcance", {
          color = "#c8c2b4", x = hx, y = hy + 1.6, z = hz,
        })
      end
    end
    return
  end
  st.near = target
  publish_tool(st, target)
  swing(st, target)
end
