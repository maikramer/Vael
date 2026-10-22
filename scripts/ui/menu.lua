-- simple-rpg — comportamento do menu.
--
-- Missões, mochila e talentos vêm de ui/journal.lua. A loja vem de
-- game/shop.lua. Este script trata dos cliques.

local journal = viber.load("ui/journal.lua")
local travel = viber.load("game/travel.lua")
local rules = viber.load("lib/rules.lua")

local SKILLS = {
  "vitality1", "strength1", "agility1", "precision1",
  "vitality2", "strength2", "agility2", "precision2",
}

-- Acessibilidade: a engine expõe `viber.a11y`/`a11y_set`; o menu só desenha
-- as linhas e manda a alteração. `interact_hold` é lido pelo diálogo nativo.
local A11Y_ROWS = {
  { id = "shake", label = "Tremor da câmara" },
  { id = "hitstop", label = "Hit-stop no impacto" },
  { id = "interact_hold", label = "[E] por hold (0,35 s)" },
  { id = "subtitle", label = "Tamanho das legendas" },
  { id = "ui_scale", label = "Escala do HUD" },
  { id = "language", label = "Idioma" },
}
local A11Y_SCALES = { 1.0, 1.25, 1.5, 2.0 }
local UI_SCALES = { 1.0, 1.25, 1.5 }
local LANGS = { "pt", "en" }
local a11y_last

-- O menu é um UiRoot: o som sem posição saía na origem da UI, não no herói.
local function ui_sfx(name)
  local has, x, y, z = viber.player_position()
  if has then
    viber.sound(name, { x = x, y = y, z = z })
  else
    viber.sound(name)
  end
end

local function a11y_default()
  return { subtitle_scale = 1.0, shake = true, hitstop = true, interact_hold = false }
end

local function current_lang()
  if viber.locale and viber.locale.get then
    return viber.locale.get() or "pt"
  end
  return "pt"
end

local function a11y_refresh()
  if not viber.a11y then
    return
  end
  local a = viber.a11y() or a11y_default()
  local ui_scale = tonumber(viber.game().ui_scale) or 1.0
  local lang = current_lang()
  local sig = table.concat({
    tostring(a.subtitle_scale), tostring(a.shake),
    tostring(a.hitstop), tostring(a.interact_hold),
    tostring(ui_scale), lang,
  }, "|")
  if sig == a11y_last then
    return
  end
  a11y_last = sig
  local rows = {}
  for _, row in ipairs(A11Y_ROWS) do
    local value
    if row.id == "subtitle" then
      value = string.format("%.2f×", a.subtitle_scale or 1)
    elseif row.id == "ui_scale" then
      value = string.format("%.2f×", ui_scale)
    elseif row.id == "language" then
      value = lang
    else
      value = a[row.id] and "ligado" or "desligado"
    end
    rows[#rows + 1] = { id = row.id, label = row.label, value = value }
  end
  viber.ui.list("a11y", rows)
end

local function a11y_clicks()
  for _, row in ipairs(A11Y_ROWS) do
    if viber.ui.clicked("a11y-" .. row.id) then
      if row.id == "ui_scale" then
        local current = tonumber(viber.game().ui_scale) or 1.0
        local at = 1
        for i, scale in ipairs(UI_SCALES) do
          if math.abs(current - scale) < 0.01 then
            at = i % #UI_SCALES + 1
            break
          end
        end
        local next_scale = UI_SCALES[at]
        viber.game().ui_scale = next_scale
        viber.ui.scale(next_scale)
      elseif row.id == "language" then
        local current = current_lang()
        local at = 1
        for i, lang in ipairs(LANGS) do
          if current == lang then
            at = i % #LANGS + 1
            break
          end
        end
        viber.game().lang = LANGS[at]
        if viber.locale and viber.locale.set then
          viber.locale.set(LANGS[at])
        end
      elseif not viber.a11y or not viber.a11y_set then
        a11y_last = nil
      else
        local a = viber.a11y() or a11y_default()
        if row.id == "subtitle" then
          local at = 1
          for i, scale in ipairs(A11Y_SCALES) do
            if math.abs((a.subtitle_scale or 1) - scale) < 0.01 then
              at = i % #A11Y_SCALES + 1
              break
            end
          end
          viber.a11y_set({ subtitle_scale = A11Y_SCALES[at] })
        else
          viber.a11y_set({ [row.id] = not a[row.id] })
        end
      end
      a11y_last = nil
      ui_sfx("ui")
    end
  end
end

-- A loja é dirigida pelas LINHAS da lista `shop` (game/shop.lua monta-as):
-- um preço "+N" é venda, o resto é compra. Com a lista fixa de antes
-- (só madeira/pedra) os despojos de quest apareciam mas o clique morria.
local shop = viber.load("game/shop.lua")

-- A cena de menu é só o acampamento atrás do título: não tem regras, quests
-- nem HUD. Gravar ali criaria um "Continuar" que cai no acampamento sem jogo.
local function in_menu_scene()
  return viber.scene_name and viber.scene_name() == "menu" or false
end

local function shop_clicks()
  for _, row in ipairs(viber.ui.rows("shop")) do
    if row.id and viber.ui.clicked("shop-" .. row.id) then
      local selling = tostring(row.price or ""):sub(1, 1) == "+"
      -- Direto: o evento ui_action só era drenado pelo merchant da cena;
      -- nas salas sem mercador (e com o menu aberto) o clique ficava
      -- pendente até se voltar ao mercado.
      shop.trade(selling and "sell" or "buy", row.id)
    end
  end
end

local function slot_exists(slot)
  local ok, list = pcall(viber.save_slots)
  if not ok or type(list) ~= "table" then
    -- Engine sem metadados: deixa o load decidir (não bloqueia o botão).
    return true
  end
  for _, entry in ipairs(list) do
    if entry.slot == slot and entry.exists then
      return true
    end
  end
  return false
end

-- A aba Sistema era alimentada pela engine com `UiData`, que só conhece o
-- componente `Health` da engine. O HP e o nível DESTE jogo vivem em
-- `viber.game()` (lib/rules.lua), por isso a lista mostrava sempre "Nível 0"
-- e "Vida 0/0". O script reclama a fonte `system` (nome da engine, permitido
-- — `set_script` impede o colector de a reescrever) e publica os valores
-- reais, incluindo os que só o jogo conhece (ouro, zona, tempo de jogo).
local function fmt_playtime(secs)
  local n = tonumber(secs)
  if not n or n < 0 then
    return "—"
  end
  n = math.floor(n)
  local h = math.floor(n / 3600)
  local m = math.floor((n % 3600) / 60)
  if h > 0 then
    return string.format("%d h %02d min", h, m)
  end
  return string.format("%d min", m)
end

local last_system = ""

local function refresh_system()
  local game = viber.game()
  local level = tonumber(game.level)
  local hp, max_hp = tonumber(game.hp), tonumber(game.max_hp)
  local health = "—"
  if hp and max_hp and max_hp > 0 then
    health = string.format("%d/%d", math.floor(hp + 0.5), math.floor(max_hp + 0.5))
  end
  local scene = (viber.scene_name and viber.scene_name()) or "—"
  if scene == "main" then
    scene = "O Vale"
  end
  local rows = {
    { key = "Motor", value = "Viber" },
    { key = "Render", value = "Bevy · Rapier" },
    { key = "Cena", value = scene },
    { key = "Hora", value = viber.ui.get("clock") },
    { key = "Nível", value = level and tostring(level) or "—" },
    { key = "Vida", value = health },
    { key = "Ouro", value = tostring(rules.vault_get("gold")) },
    { key = "Zona", value = viber.ui.get("zone.name") },
    { key = "Sessão", value = fmt_playtime(game.playtime_secs) },
  }
  local parts = {}
  for i, row in ipairs(rows) do
    parts[i] = row.key .. "\2" .. row.value
  end
  local sig = table.concat(parts, "\1")
  if sig == last_system then
    return
  end
  last_system = sig
  viber.ui.list("system", rows)
end

-- Slots: os botões Guardar/Carregar não diziam nada sobre o slot. Em especial
-- na cena de menu (Definições do título) os Guardar existiam mas o guarda
-- recusava — um botão morto. O estado é refrescado ao abrir e 1×/s.
local slots_force = true
-- Depois de Guardar, a metadata só existe no disco no fim do frame; adiar
-- meio segundo evita re-ler o estado antigo e deixar o Carregar desactivado.
local slots_delay = 0
-- Carregar = retomar um jogo: no frame seguinte ao load, o `title_done` do
-- save carregado tem de ficar verdadeiro (saves antigos do autosave "Ecrã de
-- título" gravaram false e deixavam a história muda).
local resume_pending = false

local function refresh_slots()
  local menu_scene = in_menu_scene()
  if viber.ui.set_disabled then
    for i = 1, 3 do
      viber.ui.set_disabled("act-save-" .. i, menu_scene)
    end
  end
  local ok, list = pcall(viber.save_slots)
  if not ok or type(list) ~= "table" then
    return
  end
  local exists = {}
  for _, entry in ipairs(list) do
    if entry.exists and entry.slot and entry.slot >= 1 and entry.slot <= 3 then
      exists[entry.slot] = true
    end
  end
  if viber.ui.set_disabled then
    for i = 1, 3 do
      viber.ui.set_disabled("act-load-" .. i, not exists[i])
    end
  end
end

local function save_slot(slot)
  viber.save_slot(slot)
  slots_force = true
  slots_delay = 0.5
  -- Sem toast/som aqui: o save da engine já publica "Jogo gravado." (ou a
  -- falha) e toca o clip `save`. O toast local dizia "guardado" mesmo quando
  -- a gravação falhava (dois avisos contraditórios no ecrã).
end

-- Nas Definições do título as linhas de jogo (talentos, loja, missões) não
-- fazem nada — em vez de aceitarem o clique e o ignorarem, ficam `disabled`
-- (o gamepad salta-as e o CSS cinzeia-as). `exists` evita o warning de
-- "unknown element" nas linhas que a lista ainda não construiu.
local function set_disabled_existing(id, disabled)
  if viber.ui.set_disabled and viber.ui.exists and viber.ui.exists(id) then
    viber.ui.set_disabled(id, disabled)
  end
end

local function gate_game_actions(disabled)
  for _, id in ipairs(SKILLS) do
    set_disabled_existing("skill-" .. id, disabled)
  end
  for _, row in ipairs(viber.ui.rows("shop")) do
    if row.id then
      set_disabled_existing("shop-" .. row.id, disabled)
    end
  end
  for _, row in ipairs(viber.ui.rows("quests")) do
    if row.id then
      set_disabled_existing("track-" .. row.id, disabled)
    end
  end
end

local function load_slot(slot)
  if not slot_exists(slot) then
    viber.toast("O slot " .. slot .. " está vazio.")
    ui_sfx("error")
    return
  end
  viber.load_slot(slot)
  resume_pending = true
  -- A UI é autoload: o modal sobrevivia ao load e o jogador continuava no
  -- diário depois de carregar. Fecha tudo e devolve o HUD.
  viber.ui.open("menu", false)
  viber.ui.open("region-map", false)
  viber.ui.open("travel", false)
  viber.ui.open("profiler-win", false)
  viber.ui.set_visible("hud", true)
end

function on_update(dt)
  -- O título e o ecrã de morte são modais "por cima de tudo": o [Q] abria o
  -- diário por baixo do véu (e a loja deixava comprar num jogo que não
  -- começou / num herói morto). Fecha-o assim que apareça.
  if (viber.ui.is_open("title") or viber.ui.is_open("death"))
      and viber.ui.is_open("menu") then
    viber.ui.open("menu", false)
  end
  -- Escala carregada de um save diferente da sessão — mesmo com o menu
  -- fechado, para o HUD não ficar no tamanho antigo.
  local st = viber.state()
  if resume_pending then
    resume_pending = false
    viber.game().title_done = true
  end
  -- Troca para a cena do acampamento no frame SEGUINTE ao autosave: o
  -- pedido de save é consumido no Update e a cena troca no `Last`. Feita no
  -- mesmo frame, a metadata podia sair com `scene="menu"` (se o sistema de
  -- save corresse no frame seguinte) e o Continuar descartava o autosave.
  if st.pending_title_scene then
    st.pending_title_scene = false
    local scene = viber.scene_name and viber.scene_name()
    if viber.ui.is_open("title") and scene ~= nil and scene ~= "menu" then
      viber.change_scene("menu")
    end
  end
  -- Definições do título abrem este mesmo diário na cena de menu. Aí não há
  -- jogo a decorrer: comprar/vender, gastar talentos ou seguir missões mexia
  -- no estado da partida anterior e o load/Novo Jogo deitava tudo fora.
  local on_title = in_menu_scene()
  local want = tonumber(viber.game().ui_scale) or 1.0
  if st.applied_scale ~= want and viber.ui.scale then
    st.applied_scale = want
    viber.ui.scale(want)
  end
  if viber.ui.is_open("menu") then
    journal.refresh()
    a11y_refresh()
    a11y_clicks()
    refresh_system()
    st.slots_t = (st.slots_t or 0) - dt
    slots_delay = math.max(0, slots_delay - dt)
    if (slots_force and slots_delay <= 0) or not st.menu_was_open or st.slots_t <= 0 then
      slots_force = false
      st.slots_t = 1.0
      refresh_slots()
      -- Repetido 1×/s: as linhas das listas nascem depois do 1.º frame e
      -- perderam o `disabled` se ele fosse aplicado só ao abrir.
      gate_game_actions(on_title)
    end
    st.menu_was_open = true
  else
    st.menu_was_open = false
  end
  if viber.ui.clicked("menu-close") then
    viber.ui.open("menu", false)
  end

  if not on_title then
    for _, id in ipairs(SKILLS) do
      if viber.ui.clicked("skill-" .. id) then
        journal.learn(id)
      end
    end

    shop_clicks()

    -- Seguir/parar de seguir uma missão. O cartão do HUD e a seta leem
    -- `tracked_quest`; o diário é o único sítio para o escolher.
    if viber.ui.is_open("menu") then
      for _, row in ipairs(viber.ui.rows("quests")) do
        if row.id and viber.ui.clicked("track-" .. row.id) then
          travel.track(row.id)
        end
      end
    end
  end

  if viber.ui.clicked("act-save-1") and not on_title then save_slot(1) end
  if viber.ui.clicked("act-save-2") and not on_title then save_slot(2) end
  if viber.ui.clicked("act-save-3") and not on_title then save_slot(3) end
  if viber.ui.clicked("act-load-1") then load_slot(1) end
  if viber.ui.clicked("act-load-2") then load_slot(2) end
  if viber.ui.clicked("act-load-3") then load_slot(3) end
  if viber.ui.clicked("act-title") then
    local from_title = on_title
    -- Autosave antes de sair: o Continuar do título carrega este estado.
    -- A partir do próprio menu não se grava (não há jogo para continuar).
    if not from_title then
      viber.save_slot(0)
    end
    -- NÃO pôr `title_done=false` aqui: o save é aplicado no pós-frame e
    -- apanhava o valor já mudado. O autosave seguinte virava o "Continuar"
    -- mais recente num jogo MUDO (story.lua desiste com title_done=false).
    -- A cena de menu não corre as regras/história, por isso o título não
    -- precisa deste carimbo; `title.lua` só o marca a false no arranque.
    viber.ui.open("menu", false)
    viber.ui.open("region-map", false)
    viber.ui.open("travel", false)
    viber.ui.open("profiler-win", false)
    viber.ui.set_visible("hud", false)
    -- O título tem uma cena própria (o acampamento); trocar para ela liberta
    -- o mundo grande enquanto o jogador está no ecrã de título. A troca
    -- acontece no frame seguinte (ver `pending_title_scene` no topo), depois
    -- de o pedido de save ter sido consumido com a cena de jogo ainda activa.
    if not from_title then
      st.pending_title_scene = true
    end
    viber.ui.open("title", true)
  end
  if viber.ui.clicked("act-profiler") then
    -- O botão dizia "Profiler [P]" e só mostrava um toast. O modal do
    -- profiler é `profiler-win` (não "profiler").
    viber.ui.open("menu", false)
    viber.ui.open("profiler-win", true)
  end
end
