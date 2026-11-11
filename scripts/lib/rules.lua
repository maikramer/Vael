-- Regras do Vael: HP, cofre, XP, quests, veneno, passivas e biomas.
-- TUDO em `viber.game()` (save) e `viber.game().enemies` (registro vivo).
-- A engine só expõe primitivas; não conhece vida, ouro nem missões.

local M = {}

-- Resolvedor do NLU: texto do jogador -> ids do jogo (lib/parley.lua).
local parley = viber.load("lib/parley.lua")
-- Registo de ações para o juiz (lib/judge.lua).
local actions = viber.load("lib/actions.lua")
-- Equipamento (lib/gear.lua): efeitos dos itens equipados e a arma na mão.
local gear = viber.load("lib/gear.lua")
local clock = viber.load("lib/clock.lua")
-- Tradução das frases dos scripts (toasts, prompts, balões): instala uma
-- vez os embrulhos de saída. Em PT não faz nada.
do
  local ok, i18n = pcall(viber.load, "lib/i18n.lua")
  if ok and type(i18n) == "table" and i18n.install then
    pcall(i18n.install)
  end
end

-- Declarada cedo (a definição vem mais abaixo) para os handlers de quest
-- poderem tocar o som no herói e não na origem do controller.
local sound_at_player
-- Autosave PENDENTE (segundos de uptime): ver `award_kill`/`quest_turn_in`.
-- Vive no `tick` e não num `viber.after`: o timer pertence à entidade que
-- chamou (a criatura, no caminho do FSM) e morre com ela, e um save a meio
-- da morte gravava um cadáver no slot 0.
local save_at

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

-- O tecto das pilhas e o kit de arranque vivem em `src/vault.rs`; aqui fica
-- só o `stack_max` global que `M.stack_max` publica.
local STACK = tonumber(ITEMS_DATA.stack_max) or 99
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

-- A curva de XP (base/growth/pontos por nível) vive em `src/xp.rs`.
-- Chefes iguais a `combat::kill_xp`. O resto usa o piso do melee; tabelas
-- diferentes faziam a barra (regras) e os talentos (engine) subirem em
-- níveis distintos.
local KILL_XP = PROGRESSION.kill_xp or {}
local KILL_XP_DEFAULT = math.max(0, tonumber(KILL_XP.default) or 15)
-- Passivas do diário: os valores vivem em `data/progression.json`; a engine
-- não tem árvore de talentos. O diário só gasta o ponto.
local SKILL_ORDER = {}
local SKILL_BY_ID = {}
for _, skill in ipairs(PROGRESSION.skills or {}) do
  if type(skill) == "table" and type(skill.id) == "string" and skill.id ~= "" then
    SKILL_BY_ID[skill.id] = skill
    SKILL_ORDER[#SKILL_ORDER + 1] = skill
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

-- A autoridade Rust (`VAEL_RULES_RUST=1`, `src/rules_authority.rs`) possui o
-- tick de vitais/status/morte. A flag é escrita em `viber.game()` pelo Rust
-- antes do primeiro frame; o `M.tick` salta só as secções portadas.
local function rust_vitals()
  return g().vael_rust_vitals == true
end

-- Progressão em Rust (`src/xp.rs`, `viber.vael`): saneamento de saves
-- antigos/corrompidos e a aritmética de níveis.
local function ensure_xp()
  viber.vael.ensure_xp()
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

-- Passivas do diário (talentos + equipamento) em Rust (`src/gear.rs`).
local function learned_stats()
  return viber.vael.player_stats()
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
  return learned_stats()
end

-- Efeito genérico somado (talentos + equipamento); 0 quando ninguém o dá.
function M.stat(key)
  return viber.vael.stat(key)
end

-- Arma na mão (números do melee) e equipamento: ver lib/gear.lua.
function M.weapon()
  return gear.weapon()
end

function M.gear()
  return gear
end

-- Equipa/tira um item da mochila. Devolve "on" | "off" | nil.
function M.equip_toggle(id)
  local result = gear.toggle(id)
  if result then
    actions.record("equipar", "item", id)
  end
  return result
end

-- Cantil de cura [E]: cargas que só voltam no descanso (fogueira, curandeiro,
-- renascer). A cura deixou de ser grátis de 12 em 12 s.
local FLASK_BASE = 2

function M.flask_max()
  return math.max(1, FLASK_BASE + math.floor(M.stat("flasks")))
end

function M.flasks()
  local n = g().flasks
  local max = M.flask_max()
  if not finite(n) then
    return max
  end
  return math.max(0, math.min(max, math.floor(n)))
end

function M.flask_use()
  local n = M.flasks()
  if n < 1 then
    return false
  end
  g().flasks = n - 1
  return true
end

function M.flask_refill()
  g().flasks = M.flask_max()
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
        -- `type(def) == "table"`: um JSON com uma entrada primária (editor/
        -- hand-edit) rebentava aqui e o erro subia pelo `tick` TODOS os
        -- frames — vida, quests e morte ficavam congelados. As listas de
        -- itens e talentos já têm a mesma guarda.
        if type(def) == "table" and type(def.id) == "string" and def.id ~= ""
            and not defs_by_id[def.id] then
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

-- ETAPAS: uma quest com `stages: [ {type,target,count,title?}, … ]` cumpre
-- os objetivos por ordem; `q.<id>.stage` guarda a etapa actual (1-based). Sem
-- `stages`, vale o `objective` único de sempre.
local function stage_index(def)
  local stages = def.stages
  if type(stages) ~= "table" or #stages == 0 then
    return nil
  end
  local at = math.floor(num(qkey(def.id) .. ".stage", 1))
  return math.max(1, math.min(#stages, at))
end

local function objective(def)
  local at = stage_index(def)
  if at then
    local stage = def.stages[at]
    return type(stage) == "table" and stage or {}
  end
  return def.objective or {}
end

-- Estado/progresso/objetivo/disponibilidade/cadeia vivem em `src/quest.rs`
-- (`viber.vael.quest_*`); estes embrulhos só mantêm a assinatura das regras.
-- `find_def`/`objective`/`load_defs`/`quest_def(s)` continuam em Lua: a UI, o
-- `report_kill` e o `scan_visits` leem as tabelas do JSON.
function M.quest_progress(id)
  local r = viber.vael.quest_progress(id)
  return r.have, r.need
end

-- Objetivo EM CURSO (a etapa actual numa quest com `stages`) e a posição
-- dela: `obj, etapa, total`. O diário, a seta e os dadores leem daqui.
function M.quest_objective(id)
  local r = viber.vael.quest_objective(id)
  return r.obj, r.at, r.total
end

-- `requires`: id (ou lista de ids) de quests que têm de estar ENTREGUES antes
-- de esta poder ser aceite — cadeias e conteúdo de pós-jogo.
function M.quest_available(id)
  return viber.vael.quest_available(id)
end

-- Cadeia de um dador: enquanto a quest estiver entregue e tiver `next`, o
-- NPC passa a oferecer a seguinte.
function M.quest_chain(id)
  return viber.vael.quest_chain(id)
end

function M.quest_state(id)
  return viber.vael.quest_state(id)
end

-- `count: 0`/ausente é uma quest de um passo, não uma divisão por zero nem
-- um objetivo que nasce pronto (`progress >= 0`).
local function count_of(obj)
  return math.max(1, math.floor(tonumber((obj or {}).count) or 1))
end

-- Meta efetiva: `q.<id>.need` sobrepõe o `count` do JSON (entrega parcial
-- de collect). Sem override, vale o objetivo original.
local function need_of(def)
  local override = tonumber(g()[qkey(def.id) .. ".need"])
  if override and override >= 1 then
    return math.floor(override)
  end
  return count_of(objective(def))
end

local function set_need(id, n)
  if n and n >= 1 then
    g()[qkey(id) .. ".need"] = math.floor(n)
  else
    g()[qkey(id) .. ".need"] = nil
  end
end

local function progress_of(id)
  return num(qkey(id) .. ".n", 0)
end

local function set_progress(id, n)
  g()[qkey(id) .. ".n"] = n
end

local clear_visits

local function mark_ready(def)
  local need = need_of(def)
  if progress_of(def.id) >= need and M.quest_state(def.id) == "active" then
    -- Etapa intermédia: avança para a seguinte em vez de ficar pronta.
    local at = stage_index(def)
    if at and at < #def.stages then
      g()[qkey(def.id) .. ".stage"] = at + 1
      set_progress(def.id, 0)
      clear_visits(def)
      local nxt = objective(def)
      g()[qkey(def.id) .. ".need"] = count_of(nxt)
      viber.toast("Etapa " .. at .. "/" .. #def.stages .. " cumprida: " .. (def.title or def.id))
      sound_at_player("notification")
      -- A etapa nova pode já estar satisfeita (o item está na mochila).
      M.sync_collect()
      return
    end
    g()[qkey(def.id)] = "ready"
    viber.toast("Objetivo completo: " .. (def.title or def.id))
    sound_at_player("notification")
  end
end

-- Quests de decisão (`scripts/npc/decision-quest.lua`): o puzzle marca o
-- objetivo quando o jogador resolve; o Rust grava o progresso e promove a
-- etapa seguinte ou a "ready". O toast e o som ficam aqui.
function M.quest_set_progress(id, n)
  local r = viber.vael.quest_set_progress(id, n)
  if not r.ok then
    return false
  end
  if r.stage_advanced then
    viber.toast("Etapa " .. r.stage .. "/" .. r.total .. " cumprida: " .. r.title)
    sound_at_player("notification")
  end
  if r.ready then
    viber.toast("Objetivo completo: " .. r.title)
    sound_at_player("notification")
  end
  M.sync_collect()
  return true
end

function clear_visits(def)
  -- Apaga TODOS os `q.<id>.v.*`, não só os locais do objectivo actual: um
  -- save antigo (ou um objectivo editado no editor, que troca/renomeia
  -- locais) guardava marcadores órfãos que nunca mais eram lidos. Recolhe
  -- primeiro: escrever durante o `pairs` é comportamento indefinido.
  local prefix = qkey(def.id) .. ".v."
  local stale = {}
  for key in pairs(g()) do
    if type(key) == "string" and key:sub(1, #prefix) == prefix then
      stale[#stale + 1] = key
    end
  end
  for _, key in ipairs(stale) do
    g()[key] = nil
  end
end

-- `accept`/`turn_in` correm em `src/quest.rs`; o embrulho só faz os efeitos
-- locais (toast, som, `sync_collect`, `save_at`).
function M.quest_accept(id)
  if not viber.vael.quest_accept(id) then
    return false
  end
  local def = find_def(id)
  viber.toast("Quest aceite: " .. (def and def.title or id))
  sound_at_player("quest_accept")
  M.sync_collect()
  return true
end

function M.quest_turn_in(id)
  local r = viber.vael.quest_turn_in(id)
  if r.reason == "missing_items" then
    viber.toast("Já não tens o que a missão pede.")
    return false
  end
  if not r.ok then
    return false
  end
  local msg = "Quest entregue: " .. r.title
    .. (r.gold > 0 and (" (+" .. r.gold .. " ouro)") or "")
  if r.missing_items > 0 then
    msg = msg .. " — " .. r.missing_items .. " item(ns) não coube(ram) na mochila."
  end
  viber.toast(msg)
  sound_at_player("quest_complete")
  -- Save pelo `tick` (ver `save_at`): a entrega também acontece nos 2,2 s de
  -- `dying` (o [E] ainda chega aos NPCs) e o save síncrono gravava hp 0 +
  -- `dying` no slot 0 — o "Continuar" seguinte abria o ecrã de morte.
  save_at = viber.time()
  M.sync_collect()
  return true
end

-- Bloco `parley` de uma quest (prompt/expect/partial).
function M.quest_parley(id)
  local def = find_def(id)
  return def and def.parley or nil
end

-- O relato casa com o alvo? (não muda estado; o quadro usa para filtrar)
function M.parley_matches(id, text, parsed)
  local def = find_def(id)
  -- `parley.report` também tem de ser tabela: um JSON editado à mão com
  -- `parley` mas sem `report` rebentava no `pick_for` do quadro de avisos
  -- (o lint do editor só avisa, o jogo não pode indexar nil).
  if not def or type(def.parley) ~= "table"
      or type(def.parley.report) ~= "table" then
    return false
  end
  return parley.match_expect(parley.resolve(text, parsed), def.parley.report.expect)
end

-- Entrega com relato de uma palavra. Em collect `partial`, um número menor
-- que a meta consome o que existe e reduz o que falta.
function M.quest_turn_in_parley(id, text, parsed)
  local def = find_def(id)
  if not def then
    return false, "Missão desconhecida."
  end
  -- Só uma missão PRONTA entrega: a resposta do NLU chega uns frames depois
  -- e a entrega parcial consome itens — um relato atrasado não pode gastar o
  -- cofre de uma missão que entretanto fechou (ou que nunca foi aceite).
  if M.quest_state(id) ~= "ready" then
    return false, "Essa missão não está pronta a entregar."
  end
  if type(def.parley) ~= "table" or type(def.parley.report) ~= "table" then
    -- Sem bloco de relato (quest antiga/conteúdo sem `report`), a entrega
    -- normal decide — em vez de indexar `def.parley.report.expect` (nil).
    return M.quest_turn_in(id), nil
  end
  local res = parley.resolve(text, parsed)
  local ok = parley.match_expect(res, def.parley.report.expect)
  if not ok then
    return false, def.parley.report.hint or "Não te ouvi dizer o nome."
  end
  local obj = objective(def)
  if def.parley.partial and obj.type == "collect" and res.quantidade then
    local need = need_of(def)
    local n = math.floor(res.quantidade)
    if n > 0 and n < need then
      local have = M.vault_get(obj.target or "")
      if have < n then
        return false, "Só tens " .. have .. "."
      end
      if not M.vault_take(obj.target or "", n) then
        return false, "Não tens o que dizes."
      end
      set_need(id, need - n)
      set_progress(id, 0)
      g()[qkey(id) .. ".said"] = text
      M.sync_collect()
      local msg = "Entregaste " .. n .. " — faltam " .. math.max(0, need - n) .. "."
      sound_at_player("notification")
      -- `false` (e não `true, "partial"`): para TODOS os dadores actuais um
      -- `ok` verdadeiro é entrega FINAL — o quadro de avisos dizia as falas
      -- de conclusão ("a fogueira agradece") e o NPC "Entregue." com a
      -- missão ainda em curso. O `msg` explica o progresso parcial.
      return false, msg
    end
  end
  g()[qkey(id) .. ".said"] = text
  local turned = M.quest_turn_in(id)
  if not turned then
    return false, "Ainda não tens o que a missão pede."
  end
  return true, nil
end

-- Entrega com respostas do d1: cada expect tem uma pergunta "rN" (noul).
function M.quest_turn_in_decision(id, text, answers)
  local def = find_def(id)
  if not def then
    return false, "Missão desconhecida."
  end
  if M.quest_state(id) ~= "ready" then
    return false, "Essa missão não está pronta a entregar."
  end
  local report = type(def.parley) == "table" and def.parley.report or nil
  if type(report) ~= "table" then
    return M.quest_turn_in(id), nil
  end
  for i = 1, #(report.expect or {}) do
    local a = answers and answers["r" .. i]
    if not a or (tonumber(a.noul) or 0) < 0.5 then
      return false, report.hint or "Não te ouvi dizer o nome."
    end
  end
  g()[qkey(id) .. ".said"] = text
  local turned = M.quest_turn_in(id)
  if not turned then
    return false, "Ainda não tens o que a missão pede."
  end
  return true, nil
end

-- O relato casa com o alvo? (para o quadro de avisos, sem mudar estado)
function M.parley_matches_decision(id, answers)
  local def = find_def(id)
  if not def or type(def.parley) ~= "table"
      or type(def.parley.report) ~= "table" then
    return false
  end
  local expect = def.parley.report.expect or {}
  for i = 1, #expect do
    local a = answers and answers["r" .. i]
    if not a or (tonumber(a.noul) or 0) < 0.5 then
      return false
    end
  end
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

-- `elite` = a criatura abatida era elite: conta também para objetivos com
-- `target: "elite"` (caçadas de pós-jogo), seja qual for a espécie.
function M.report_kill(kind, elite)
  if not kind or kind == "" then
    return
  end
  for _, def in ipairs(load_defs()) do
    if M.quest_state(def.id) == "active" then
      local obj = objective(def)
      if obj.type == "kill" and (kill_matches(kind, obj.target or "")
          or (elite and obj.target == "elite")) then
        local need = need_of(def)
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
  -- Regista a visita ANTES do filtro de quests: o juiz quer saber o que o
  -- herói fez, não só o que contava para uma missão ativa.
  actions.record("visitar", "local", place)
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
          set_progress(def.id, math.min(need_of(def), n))
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
        -- `tonumber`: um `radius` textual/não finito num JSON editado à mão
        -- rebentava (`"12" * "12"` até ia, mas `true`/tabela não) e o erro
        -- subia pelo `tick` TODOS os frames — vida, quests e autosave
        -- congelados. O `count` do objetivo já é convertido em `count_of`.
        local radius = tonumber(obj.radius) or 12
        if not finite(radius) or radius <= 0 then
          radius = 12
        end
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

-- Objetivos `type: "craft"`: contam o que o herói forjou (lib/craft.lua).
function M.report_craft(item, amount)
  if not item or item == "" then
    return
  end
  for _, def in ipairs(load_defs()) do
    if M.quest_state(def.id) == "active" then
      local obj = objective(def)
      if obj.type == "craft" and obj.target == item then
        local need = need_of(def)
        set_progress(def.id, math.min(need, progress_of(def.id) + (amount or 1)))
        mark_ready(def)
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
        local need = need_of(def)
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

-- Cofre em Rust (`src/vault.rs`, `viber.vael`): contagem nunca negativa,
-- recursos em `resource_cap` e os outros itens no `stack`/`stack_max`.
-- Aqui ficam só os embrulhos: espelho na API antiga da engine (quando
-- existe) e `sync_collect` (objetivos de recolha).

function M.vault_get(kind)
  return viber.vael.vault_get(kind)
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
  -- Kit de arranque (`data/items.json`) só quando o save ainda não gravou
  -- `item.gold` nem `item.potion`; um 0 explícito (ouro gasto) não enche.
  viber.vael.seed_starting()
  g().eco_pulled = true
end

function M.vault_add(kind, amount, mirror)
  local added = viber.vael.vault_add(kind, amount)
  if added > 0 and mirror ~= false then
    mirror_add(kind, added)
  end
  M.sync_collect()
  return added
end

function M.vault_take(kind, amount)
  local ok = viber.vael.vault_take(kind, amount)
  if ok then
    -- `amount or 1` como no `vault_add`/`vault_take` antigos.
    mirror_take(kind, amount or 1)
    M.sync_collect()
  end
  return ok
end

function M.item_add(id, amount)
  return M.vault_add(id, amount or 1)
end

function M.item_count(id)
  return M.vault_get(id)
end

function M.add_xp(gain, mirror)
  -- `mirror` mantém-se na assinatura por compatibilidade; o espelho na
  -- engine antiga morreu (o saneamento e a aritmética são do Rust).
  viber.vael.add_xp(gain)
end

function M.add_skill_points(n)
  viber.vael.add_skill_points(n)
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
          -- Escudo erguido (lib/fsm.lua `shield`): o melee decide o bloqueio.
          shield = e.guard and e.shield or nil,
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
  -- Invulnerável (chefe enterrado, transição de fase): o golpe não entra.
  if e.invuln then
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

-- ESTADOS do herói, guiados por tabela. `dps` = dano por segundo (tick de
-- 1 s); `slow` = multiplicador da velocidade enquanto dura. O tempo vive em
-- `viber.game()[kind]` (viaja no save); `<kind>_resist` (talentos/itens)
-- encurta a duração.
local STATUS = {
  venom = { dps = 4, color = "#7ed957" },
  burn = { dps = 6, color = "#ff8c47" },
  frost = { slow = 0.6 },
}
local STATUS_ORDER = { "venom", "burn", "frost" }

function M.apply_status(kind, secs)
  if not STATUS[kind] or not finite(secs) or secs <= 0 then
    return
  end
  local resist = math.max(0, math.min(0.9, M.stat(kind .. "_resist")))
  g()[kind] = math.max(num(kind, 0), secs * (1 - resist))
end

function M.status_clear(kind)
  for _, name in ipairs(STATUS_ORDER) do
    if kind == nil or kind == name then
      g()[name] = 0
      g()[name .. "_tick"] = 0
    end
  end
end

function M.status_active(kind)
  return STATUS[kind] ~= nil and num(kind, 0) > 0
end

function M.poisoned()
  return num("venom", 0) > 0
end

-- Algum estado que um remédio limpa (a poção de musgo trata todos).
function M.afflicted()
  for _, name in ipairs(STATUS_ORDER) do
    if num(name, 0) > 0 then
      return true
    end
  end
  return false
end

-- ESTADOS nas criaturas: o registo vivo guarda os segundos que faltam
-- (`burn`, `frost`, `shock`); o `lib/fsm.lua` aplica o efeito no update da
-- própria criatura (dano por segundo, lentidão, atordoamento).
function M.enemy_status(id, kind, secs)
  local e = (g().enemies or {})[id]
  if not e or (e.hp or 0) <= 0 or not finite(secs) or secs <= 0 then
    return false
  end
  if kind ~= "burn" and kind ~= "frost" and kind ~= "shock" then
    return false
  end
  e[kind] = math.max(e[kind] or 0, secs)
  return true
end

function M.kill_xp(kind)
  -- `tonumber`: um `kill_xp.wolf: "20"` textual (JSON editado à mão) passava
  -- pelo `finite` do `add_xp` como não-numérico e o abate deixava de dar XP,
  -- em silêncio. O `default` já era convertido assim no arranque do módulo.
  return tonumber(KILL_XP[kind]) or KILL_XP_DEFAULT
end

function M.award_kill(kind, id)
  M.add_xp(M.kill_xp(kind))
  local elites = g().elites
  M.report_kill(kind, id ~= nil and type(elites) == "table" and elites[id] == true)
  actions.record("matar", "criatura", kind)
  if M.on_kill then
    M.on_kill(kind, id)
  end
  -- Chefe abatido = marco de progresso: autosave no slot 0, gravado pelo
  -- `tick` (`save_at`). Adiado: o `fsm` paga o loot DEPOIS de `award_kill`,
  -- e o save síncrono gravava o estado sem a bolsa/poções do chefe (morrer
  -- ou Carregar perdia-as). O flush só grava com o herói de pé.
  if KILL_XP[kind] then
    save_at = viber.time() + 0.05
  end
end

-- Direção do dano: quatro faixas no HUD. O atacante pode estar à frente,
-- atrás, à esquerda ou à direita do FORWARD do herói; usa-se o produto
-- interno (frente/trás) contra o produto externo (lados).
local DMG_EDGES = { "dmg-front", "dmg-back", "dmg-left", "dmg-right" }
-- Faixa acesa e instante de apagar. O apagar é do `tick`: um `viber.after`
-- pertence ao DONO do golpe (a criatura) e morre com ele — um despawn ou
-- uma troca de cena dentro dos 0,35 s deixava a seta de dano presa no HUD.
local dmg_edge, dmg_until = nil, 0

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
  clear_dmg_edges()
  viber.ui.set_visible(id, true)
  dmg_edge, dmg_until = id, viber.time() + 0.35
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

-- Morrer custa: um quarto do ouro fica numa bolsa no sítio da queda e
-- espera lá pelo herói (uma só — a bolsa de uma morte anterior perde-se).
-- Só no mundo grande: as coordenadas de um interior não valem em `main`.
local DEATH_GOLD_FRAC = 0.25
local PURSE_RADIUS = 2.5

local function drop_purse(game)
  local scene = viber.scene_name and viber.scene_name() or nil
  if scene ~= nil and scene ~= "main" then
    return
  end
  local has, x, y, z = viber.player_position()
  if not has or not finite(x) or not finite(y) or not finite(z) then
    return
  end
  local lost = math.floor(M.vault_get("gold") * DEATH_GOLD_FRAC)
  if lost < 1 or not M.vault_take("gold", lost) then
    return
  end
  game.lost_gold = lost
  game.lost_x, game.lost_y, game.lost_z = x, y, z
end

local purse_fx_t = 0

local function tick_purse(game, dt)
  local lost = num("lost_gold", 0)
  if lost < 1 then
    return
  end
  local scene = viber.scene_name and viber.scene_name() or nil
  if (scene ~= nil and scene ~= "main") or (game.hp or 0) <= 0 then
    return
  end
  local has, px, _, pz = viber.player_position()
  local x, y, z = game.lost_x, game.lost_y, game.lost_z
  if not has or not finite(x) or not finite(y) or not finite(z) then
    return
  end
  local dx, dz = px - x, pz - z
  local d2 = dx * dx + dz * dz
  if d2 <= PURSE_RADIUS * PURSE_RADIUS then
    game.lost_gold = 0
    local back = M.vault_add("gold", lost) or 0
    viber.damage_number("+" .. back .. " ouro", { color = "#ffd35a", x = x, y = y + 1.6, z = z })
    viber.burst("sparkle", x, y + 0.8, z, 16)
    viber.sound("coin", { x = x, y = y, z = z })
    viber.toast("Recuperaste a bolsa: +" .. back .. " de ouro.")
    return
  end
  -- Brilho periódico: a bolsa lê-se de longe (a 70 m já se vê o anel).
  purse_fx_t = purse_fx_t - dt
  if purse_fx_t <= 0 and d2 <= 70 * 70 then
    purse_fx_t = 1.6
    viber.burst("sparkle", x, y + 0.6, z, 8)
    viber.ring(x, z, 1.2, "#ffd35a")
  end
end

-- Onde está a bolsa perdida (a seta/mapa podem apontá-la): x, y, z, ouro.
function M.lost_purse()
  local lost = num("lost_gold", 0)
  local game = g()
  if lost < 1 or not finite(game.lost_x) or not finite(game.lost_z) then
    return nil
  end
  return game.lost_x, game.lost_y or 0, game.lost_z, lost
end

local function start_death(game)
  if num("dying", 0) > 0 or game.death_pending then
    return
  end
  game.dying = 2.2
  drop_purse(game)
  local id = viber.player_id()
  if id then
    viber.play_clip("death", { id = id })
  end
  sound_at_player("game_over")
end

-- Título (ou cena de menu) por cima: o mundo atrás é decoração. O
-- `rules_host` não corre o `tick` nesses frames, mas os scripts das
-- criaturas continuam a correr e o FSM só conhece `menu_open` — sem este
-- portão um golpe tirava vida por baixo do véu e, como o `tick` não avança,
-- o `dying` nunca chegava ao ecrã de morte (herói "morto" atrás do título).
local function title_blocking()
  if viber.scene_name and viber.scene_name() == "menu" then
    return true
  end
  return viber.ui ~= nil and viber.ui.is_open ~= nil
    and viber.ui.is_open("title") == true
end

-- MODO DEV (`scripts/game/devmode.lua`): invulnerabilidade do [F2] e do voo.
-- Vive no MÓDULO, não em `viber.game()`, de propósito: um cheat ligado numa
-- sessão de debug não pode ficar gravado no save.
M.god = false

function M.set_god(on)
  M.god = on == true
  -- A autoridade Rust (`VAEL_RULES_RUST=1`) faz o DoT e precisa de ver o god.
  -- Fora dela fica só no módulo (um cheat não viaja para o save).
  if rust_vitals() then
    g().vael_god = M.god
  end
end

-- Devolve true só quando o dano ENTROU: os `on_attack`/`on_hit` das
-- criaturas (veneno) não podem disparar contra um parry/i-frame.
function M.damage_player(amount, sx, sz)
  -- Menu/diário/mapa abertos: o jogador parou para ler. As criaturas já
  -- suspendem o golpe (lib/fsm.lua); sem este portão uma seta já no ar
  -- continuava a tirar vida por baixo do menu.
  if viber.menu_open() or title_blocking() then
    return false
  end
  if M.god then
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
  -- Defesa (armadura + talentos): corta uma fração do dano, até 60 %.
  local defense = math.max(0, math.min(0.6, M.stat("defense")))
  amount = amount * (1 - defense)
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
  local stats = M.player_stats()
  local new_max = 100 + stats.max_hp_bonus
  if game.max_hp ~= new_max then
    local delta = new_max - (game.max_hp or 100)
    game.max_hp = new_max
    game.hp = math.max(0, math.min(new_max, (game.hp or new_max) + delta))
  end
  -- O gelo abranda o herói enquanto dura (multiplica as passivas).
  local speed = stats.speed_mult * ((num("frost", 0) > 0) and STATUS.frost.slow or 1.0)
  if viber.set_player_speed and M._speed ~= speed then
    M._speed = speed
    viber.set_player_speed(speed)
  end
end

-- Renasce no último descanso. O ecrã de morte chama isto em "Tentar de novo";
-- mundos sem o modal (QA) usam o fallback de 5 s no `tick`.
function M.respawn()
  -- Um save parcial/corrompido pode não ter `max_hp`/`hp` numéricos: sem
  -- isto o `respawn` deixava `hp` textual e o `publish` do mesmo frame
  -- rebentava nos `%d`.
  ensure_hp()
  local game = g()
  local s = stored_spawn(game) or { x = 0, y = 150, z = 0 }
  game.hp = game.max_hp or 100
  game.dying = 0
  game.death_pending = false
  game.death_wait = 0
  M.status_clear()
  M.flask_refill()
  game.iframes = 2.0
  viber.ui.open("death", false)
  viber.ui.set_visible("hud", true)
  if num("lost_gold", 0) >= 1 then
    viber.toast("Renasces no último descanso. A tua bolsa ("
      .. math.floor(num("lost_gold", 0)) .. " de ouro) ficou onde caíste.")
  else
    viber.toast("Renasces no último descanso.")
  end
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
  -- Os timers de sessão (guarda, flashes do HUD, autosave pendente) são
  -- locals do módulo e sobrevivem ao `viber.game()`. Um LOAD na MESMA cena
  -- troca a tabela do jogo mas não muda `session_id` nem recua o relógio da
  -- engine (é o uptime): o sinal — o mesmo que combat/abilities/hotbar
  -- usam — é o tempo de jogo a recuar.
  local now = viber.time()
  local playtime = num("playtime_secs", 0)
  local loaded = M._playtime ~= nil and playtime < M._playtime - 1e-3
  M._playtime = playtime
  if M._last_time and now < M._last_time then
    loaded = true
  end
  M._last_time = now
  -- Novo Jogo/save de outra partida: flashes, guarda e passivas recomeçam.
  local sid = tonumber(g().session_id) or 0
  if M._session ~= sid or loaded then
    M._session = sid
    M._xp_sig, M._quest_sig = nil, nil
    M._xp_until, M._quest_until = nil, nil
    M._speed = nil
    guard_until, guard_start, guard_lock = 0, nil, 0
    -- Nada da linha temporal abandonada: um autosave de chefe pendente não
    -- pode gravar por cima do save que acabámos de carregar, e a faixa de
    -- dano acesa não fica no HUD da partida nova.
    save_at = nil
    if dmg_edge then
      viber.ui.set_visible(dmg_edge, false)
      dmg_edge, dmg_until = nil, 0
    end
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
  -- Com a autoridade Rust os i-frames descem no `rules_authority.rs`; o
  -- resto do `tick` (metadados, eventos, visitas, save) continua aqui.
  if not rust_vitals() then
    game.iframes = math.max(0, num("iframes", 0) - dt)
  end

  -- Apaga a faixa de dano no tick (ver `dmg_edge`): o `viber.after` antigo
  -- era do atacante e um despawn dentro dos 0,35 s deixava-a acesa.
  if dmg_edge and now >= dmg_until then
    viber.ui.set_visible(dmg_edge, false)
    dmg_edge, dmg_until = nil, 0
  end

  -- Estados com dano por segundo (veneno, queimadura), tick de 1 s. Não
  -- correm durante a morte (o cadáver ficava a 0 HP sem `dying` e nenhum
  -- caminho o curava/matava), nem com um menu aberto (ler o diário não pode
  -- matar), nem com o MODO DEV ligado (este dano não passa por
  -- `damage_player`), e um frame longo aplica TODOS os ticks devidos.
  -- `num`: um timer textual (save editado à mão) rebentava a soma e o erro
  -- subia pelo `tick` em todos os frames.
  -- Com `VAEL_RULES_RUST=1` este bloco passa para `src/rules_authority.rs`.
  if not rust_vitals() then
    local frozen = viber.menu_open() or M.god
    for _, kind in ipairs(STATUS_ORDER) do
      local spec = STATUS[kind]
      local tick_key = kind .. "_tick"
      if num(kind, 0) > 0 and not frozen then
        -- À chuva a queimadura apaga-se ao dobro da velocidade.
        local rate = (kind == "burn" and clock.raining()) and 2 or 1
        game[kind] = math.max(0, num(kind, 0) - dt * rate)
        if spec.dps and num("dying", 0) <= 0 then
          game[tick_key] = num(tick_key, 0) + dt
          local ticks = math.floor(game[tick_key])
          if ticks > 0 then
            game[tick_key] = game[tick_key] - ticks
            local dmg = ticks * spec.dps
            game.hp = math.max(0, game.hp - dmg)
            player_fx("-" .. dmg, spec.color)
            if game.hp <= 0 then
              start_death(game)
            end
          end
        end
      elseif num(kind, 0) <= 0 then
        game[tick_key] = 0
      end
    end
  end
  tick_purse(game, dt)

  -- Morte: com `VAEL_RULES_RUST=1` os contadores (`dying`, `death_pending`,
  -- `death_wait`) e o respawn de fallback são do `src/rules_authority.rs`; a
  -- UI de morte observa as mesmas chaves em `ui/death.lua`. O `M.respawn`
  -- manual (botão "Tentar de novo") e o `M.start_death` do dano direto ficam
  -- no Lua.
  if not rust_vitals() then
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
        -- `num`: idem para o contador do respawn automático.
        game.death_wait = math.max(0, num("death_wait", 0) - dt)
        if game.death_wait <= 0 then
          M.respawn()
        end
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
  -- Autosave de chefe pendente (ver `award_kill`): grava só com o herói de
  -- pé. Um save a meio da morte substituía o slot 0 por um cadáver e o
  -- "Continuar" seguinte abria o ecrã de morte — o menu já se recusa a
  -- gravar em `dying`/`death_pending`, e o autosave tem de fazer o mesmo.
  if save_at and now >= save_at
      and (game.hp or 0) > 0 and num("dying", 0) <= 0 and not game.death_pending then
    save_at = nil
    viber.save_slot(0)
  end
  M.publish()
end

function M.publish()
  ensure_xp()
  ensure_hp()
  local game = g()
  -- Vida, status, XP, talentos e economia são publicados pelo `VaelHudPlugin`
  -- (Rust). O Lua só publica o que ainda não tem dono em Rust: netplay, alvo,
  -- chefe, recargas e quests.others.
  local net_role = (viber.net_role and viber.net_role()) or "off"
  viber.ui.set("net.on", net_role ~= "off")
  viber.ui.set("net.label", net_role == "host" and "HOST"
    or (net_role == "join" and "CLIENTE" or ""))
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
      local need = need_of(def)
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
