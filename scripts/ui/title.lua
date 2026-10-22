-- ui/title.lua: ecrã de título, continuar/novo jogo e créditos.
--
-- Autoload: corre em todas as cenas e sobrevive às trocas. A cena de menu
-- (`scenes/menu.xml`, `project.start-scene`) é só o acampamento atrás do véu;
-- o mundo grande (`main`) só carrega quando o jogador escolhe Continuar/Novo
-- Jogo. `nil` (mundos QA sem título, ou interiores) salta o título.
--
-- Arranque de DEBUG: `viber run --scene main --entry dev` cai directamente no
-- mundo — o título nunca abre e o HUD nasce ligado (ver docs/DEBUG.md).

local dev = viber.load("lib/dev.lua")

local function scene_id()
  return viber.scene_name and viber.scene_name() or nil
end

local function scene_has_title()
  local scene = scene_id()
  return scene == "main" or scene == "menu"
end

local function slots()
  local ok, list = pcall(viber.save_slots)
  if not ok or type(list) ~= "table" then
    return {}
  end
  return list
end

local function latest_slot()
  local best
  for _, slot in ipairs(slots()) do
    -- Um save gravado na cena de menu não é um jogo (não tem regras nem HUD).
    if slot.exists and slot.scene ~= "menu"
        and (not best or (slot.saved_at or 0) > (best.saved_at or 0)) then
      best = slot
    end
  end
  return best
end

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

local function slot_summary(slot)
  if not slot then
    return "Sem jogos guardados"
  end
  local level = tonumber(slot.level) or 0
  local where = level > 0 and ("Nível " .. level) or "Jogo guardado"
  return where .. " · " .. fmt_playtime(slot.playtime)
end

local function load_slot(slot)
  -- Slot 4 = save legado (`viber.save()`), que não é um slot numerado.
  if slot.slot == 4 then
    viber.load_save()
  else
    viber.load_slot(slot.slot)
  end
end

local function leave_title()
  viber.game().title_done = true
  viber.ui.set_visible("hud", true)
  viber.ui.open("title", false)
  viber.ui.open("credits", false)
end

function on_update(dt)
  local st = viber.state()
  if not st.ready then
    st.ready = true
    -- `--entry dev`: o mesmo estado final do "Novo Jogo", mas sem passar pelo
    -- acampamento nem pelo véu. `st.skip` mantém o título fechado enquanto
    -- ninguém o abrir pelo diário/morte; `leave_title` liga `title_done` e o
    -- HUD. A escala guardada é aplicada aqui porque o ramo normal já não corre.
    if dev.skip_intro() then
      st.skip = true
      local scale = tonumber(viber.game().ui_scale)
      if scale and viber.ui.scale then
        viber.ui.scale(scale)
      end
      leave_title()
      return
    end
    if not scene_has_title() then
      st.skip = true
      return
    end
    -- Antes do primeiro tick do story (o prólogo tem 0.1 s de atraso).
    viber.game().title_done = false
    viber.ui.open("title", true)
    viber.ui.set_visible("hud", false)
    local scale = tonumber(viber.game().ui_scale)
    if scale and viber.ui.scale then
      viber.ui.scale(scale)
    end
  end
  if st.skip then
    -- QA/interior arranca sem título, mas o botão "Ecrã de título" do diário
    -- (ou a morte) pode abri-lo mais tarde: só salta enquanto ninguém o abriu.
    if not viber.ui.is_open("title") then
      return
    end
    st.skip = false
  end

  -- Novo Jogo: a cena `main` carrega no pós-frame; a pose do acampamento
  -- não pode ficar (o herói aparecia dentro da muralha). Assenta no spawn.
  if st.pending_spawn and scene_id() == "main" then
    st.pending_spawn = false
    viber.teleport_player(0, 150, 0)
  end

  -- Definições abre o diário (tab Sistema); ao fechar, o título volta —
  -- mas só se o jogador não tiver CARREGADO um jogo entretanto. Antes, um
  -- "Carregar" nas Definições abria o mundo e o título reaparecia por cima
  -- (o menu fechava + o ecrã de jogo ficava escondido atrás do véu).
  if st.settings then
    if viber.ui.is_open("menu") then
      return
    end
    -- O profiler é aberto pelo botão da aba Sistema; o título espera que
    -- ele feche em vez de o tapar (o profiler vive na mesma camada).
    if viber.ui.is_open("profiler-win") then
      return
    end
    st.settings = false
    local hud = viber.ui.read and viber.ui.read("hud")
    if not (hud and hud.visible) then
      viber.ui.open("title", true)
    end
    return
  end

  -- Continuar: o load troca de cena no pós-frame. O título fica aberto até a
  -- cena mudar (ou 6 s) — se o save estiver corrompido a engine avisa e o
  -- jogador continua no título em vez de ficar no acampamento sem jogo.
  if st.pending_continue then
    st.continue_t = (st.continue_t or 0) + dt
    local scene = scene_id()
    if scene == nil or scene ~= "menu" then
      st.pending_continue = false
      leave_title()
    elseif st.continue_t > 6.0 then
      st.pending_continue = false
    end
    return
  end

  if not viber.ui.is_open("title") then
    return
  end

  local best
  st.slot_t = (st.slot_t or 0) - dt
  if st.slot_t <= 0 or not st.slots_loaded then
    st.cached_best = latest_slot()
    st.slots_loaded = true
    st.slot_t = 2.0
  end
  best = st.cached_best
  local disabled = best == nil
  if st.continue_disabled ~= disabled and viber.ui.set_disabled then
    st.continue_disabled = disabled
    viber.ui.set_disabled("title-continue", disabled)
  end
  -- "Continuar" sem contexto obrigava a adivinhar o save. Mostra nível e
  -- tempo de jogo do save mais recente (ou diz que não há nenhum).
  local info = slot_summary(best)
  if st.save_info ~= info then
    st.save_info = info
    viber.ui.set_text("title-save-info", info)
  end

  if viber.ui.clicked("title-continue") then
    if best then
      load_slot(best)
      st.pending_continue = true
      st.continue_t = 0
    end
  elseif viber.ui.clicked("title-new") then
    local game = viber.game()
    -- Preferências de ecrã/idioma sobrevivem ao "Novo Jogo": limpar a tabela
    -- toda repunha a escala e o idioma sem o jogador pedir.
    local keep_scale, keep_lang = game.ui_scale, game.lang
    local session = tonumber(game.session_id) or 0
    -- Recolhe primeiro: apagar durante `pairs` é comportamento indefinido.
    local keys = {}
    for key in pairs(game) do
      keys[#keys + 1] = key
    end
    for _, key in ipairs(keys) do
      game[key] = nil
    end
    game.ui_scale, game.lang = keep_scale, keep_lang
    -- Os controllers autoload (combate/hotbar/abilidades) distinguem uma
    -- sessão nova disto para não nascerem com recargas/target do jogo antigo.
    game.session_id = session + 1
    leave_title()
    -- Recarrega o mundo para repor props colhidos, baús e criaturas.
    local ok = viber.change_scene("main")
    if not ok then
      viber.teleport_player(0, 150, 0)
    else
      st.pending_spawn = true
    end
  elseif viber.ui.clicked("title-settings") then
    viber.ui.open("title", false)
    viber.ui.open("menu", true)
    viber.ui.select_tab("menu", "system")
    st.settings = true
  elseif viber.ui.clicked("title-credits") then
    viber.ui.open("credits", true)
  elseif viber.ui.clicked("title-quit") then
    viber.quit()
  end

  if viber.ui.clicked("credits-close") then
    viber.ui.open("credits", false)
  end
end
