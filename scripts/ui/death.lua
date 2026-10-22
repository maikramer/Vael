-- ui/death.lua: ecrã de morte. A regra de morte vive em lib/rules.lua;
-- aqui só se escolhe o que fazer.

local rules = viber.load("lib/rules.lua")

-- Carregar = retomar; o `title_done` do save carregado tem de ficar
-- verdadeiro (saves antigos podiam trazê-lo a false).
local resume_pending = false

local function latest_slot(list)
  local best
  for _, slot in ipairs(list) do
    -- Um save gravado na cena de menu não é um jogo (não tem regras nem HUD).
    if slot.exists and slot.scene ~= "menu"
        and (not best or (slot.saved_at or 0) > (best.saved_at or 0)) then
      best = slot
    end
  end
  return best
end

-- `save_slots` também lista o save legado (slot 4) quando existe. Só quando
-- a própria chamada falha é que não há metadados nenhuns.
local function slots()
  local ok, list = pcall(viber.save_slots)
  if not ok or type(list) ~= "table" then
    return nil
  end
  return list
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
  return "Último save: " .. where .. " · " .. fmt_playtime(slot.playtime)
end

local function load_game()
  local list = slots()
  if list == nil then
    -- Engine sem metadados de slots: deixa o load legado decidir.
    viber.load_save()
  else
    local best = latest_slot(list)
    if not best then
      -- Sem save nenhum o botão era um clique morto que fechava o ecrã de
      -- morte e devolvia o jogador a um cadáver.
      viber.toast("Sem jogos guardados.")
      viber.sound("error")
      return
    end
    if best.slot == 4 then
      viber.load_save()
    else
      viber.load_slot(best.slot)
    end
  end
  viber.ui.open("death", false)
  viber.ui.set_visible("hud", true)
  resume_pending = true
end

function on_update(dt)
  local st = viber.state()
  if resume_pending then
    resume_pending = false
    viber.game().title_done = true
  end
  if viber.ui.is_open("death") then
    st.slot_t = (st.slot_t or 0) - dt
    if not st.slots_loaded or st.slot_t <= 0 then
      st.slots_loaded = true
      st.slot_t = 1.0
      local list = slots()
      st.best = list and latest_slot(list) or nil
      -- Carregar só com um save real: o estado disabled vem do CSS
      -- `.title-btn:disabled` e o gamepad salta-o.
      if st.load_disabled ~= (st.best == nil) and viber.ui.set_disabled then
        st.load_disabled = st.best == nil
        viber.ui.set_disabled("death-load", st.load_disabled)
      end
      local info = slot_summary(st.best)
      if st.save_info ~= info then
        st.save_info = info
        viber.ui.set_text("death-save-info", info)
      end
    end
  end

  if viber.ui.clicked("death-retry") then
    rules.respawn()
  elseif viber.ui.clicked("death-load") then
    load_game()
  elseif viber.ui.clicked("death-title") then
    viber.ui.open("death", false)
    viber.ui.set_visible("hud", false)
    -- Cancelar a morte pendente: o título é o topo, não pode ficar um
    -- respawn agendado (5 s) à espera do próximo tick de `main`.
    local game = viber.game()
    game.dying = 0
    game.death_pending = false
    game.death_wait = 0
    -- Não escrever `title_done=false`: um autosave pendente (abate de chefe,
    -- entrega de quest) apanhava o valor e o Continuar seguinte carregava um
    -- jogo sem história. O título só o marca a false no primeiro arranque.
    -- O título vive sobre o acampamento, não sobre o mundo que acabou de matar.
    if viber.scene_name and viber.scene_name() ~= "menu" then
      viber.change_scene("menu")
    end
    viber.ui.open("title", true)
  end
end
