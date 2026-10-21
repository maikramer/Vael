-- O melee nativo (chain, crítico, costas, cadáver, XP) já acerta quem
-- tem script em enemies/ ou bosses/. Este ficheiro só mantém a barra de
-- alvo: um segundo entity_damage no mesmo [J] cortava o TTK ao meio e o
-- play_clip("attack") cancelava o corte da chain.

local VERTICAL = 2.2
local CYCLE_RANGE = 14

local function show_target(kind, hp, max_hp)
  viber.ui.set("target.name", kind or "")
  if max_hp and max_hp > 0 then
    viber.ui.set("target", math.max(0, hp or 0) / max_hp)
    viber.ui.set("target.alive", (hp or 0) > 0)
  else
    viber.ui.set("target.alive", false)
  end
end

function on_update(_dt)
  local st = viber.state()
  if viber.menu_open() then
    return
  end
  local alive, hp = viber.player_hp()
  if alive and hp <= 0 then
    st.target = nil
    viber.ui.set("target.alive", false)
    viber.ui.set("target.name", "")
    return
  end
  local has, _, py, _ = viber.player_position()
  if not has then
    return
  end

  local list = viber.hostiles(CYCLE_RANGE) or {}
  local ids = {}
  for _, h in ipairs(list) do
    if math.abs((h.y or py) - py) <= VERTICAL then
      ids[#ids + 1] = h
    end
  end

  if viber.input.pressed("v") then
    if #ids == 0 then
      st.target = nil
      if viber.lock_target then
        viber.lock_target(nil)
      end
    else
      local at = 1
      for i, h in ipairs(ids) do
        if h.id == st.target then
          at = i % #ids + 1
          break
        end
      end
      st.target = ids[at].id
      if viber.lock_target then
        viber.lock_target(st.target)
      end
    end
  end

  local shown
  for _, h in ipairs(ids) do
    if h.id == st.target then
      shown = h
      break
    end
  end
  if not shown then
    shown = ids[1]
    st.target = shown and shown.id or nil
  end
  if shown then
    show_target(shown.kind, shown.hp, shown.max)
  elseif st.target ~= nil then
    st.target = nil
    viber.ui.set("target", 0)
    viber.ui.set("target.alive", false)
    viber.ui.set("target.name", "")
  end
end
