--@export site: string = ""

local rules = vael.load("lib/rules.lua")
local concordia = vael.load("lib/concordia.lua")
local talk = vael.load("ui/say.lua")

local labels = {
  wood = "madeira", stone = "pedra", ["dark-wood"] = "madeira escura",
  ["bog-moss"] = "musgo do pântano", cactus_fiber = "fibra de cacto",
}

local function deposit_key(site, kind)
  return "concordia.work." .. site.id .. "." .. kind
end

local function deliver(site, game)
  local partial = concordia.open("barn")
  local plan = {}
  local complete = true
  for _, cost in ipairs(site.work) do
    local key = deposit_key(site, cost[1])
    local stored = game[key] or 0
    local need = cost[2] - stored
    local amount = math.min(need, rules.vault_get(cost[1]))
    complete = complete and amount == need
    if amount > 0 then plan[#plan + 1] = {kind=cost[1], amount=amount, key=key, stored=stored} end
  end
  if #plan == 0 or (not partial and not complete) then return false, false end
  for i, line in ipairs(plan) do
    if not rules.vault_take(line.kind, line.amount) then
      for j = 1, i - 1 do rules.vault_add(plan[j].kind, plan[j].amount) end
      return false, false
    end
  end
  for _, line in ipairs(plan) do
    game[line.key] = complete and 0 or line.stored + line.amount
  end
  return true, complete
end

local function inhabit(site, game, key)
  game[key] = 3
  if site.id == "well" then
    game["biome.concordia"] = true
    game["mark.poco-de-concordia"] = true
  elseif site.id == "house" then
    vael.toast("A tua fogueira está acesa. O próximo canteiro é o portão da cerca.")
  end
  vael.sound("quest_complete")
  vael.cinematic("canteiro-" .. site.id)
  vael.save_slot(0)
end

function on_update(dt)
  local site = concordia.sites[vael.prop("site")]
  if not site then return end
  local game, st = vael.game(), vael.state()
  local key = "concordia." .. site.id
  local stage = concordia.stage(site.id)
  local label = site.name
  local ready = true
  if stage == 0 then
    local work, can_store = {}, false
    for _, cost in ipairs(site.work) do
      local have = rules.vault_get(cost[1])
      local missing = cost[2] - (game[deposit_key(site, cost[1])] or 0)
      ready = ready and have >= missing
      can_store = can_store or (have > 0 and missing > 0)
      if missing > 0 then
        work[#work + 1] = ("%d %s"):format(missing, labels[cost[1]])
      end
    end
    local verb = ready and "Entregar obra" or (can_store and concordia.open("barn") and "Guardar obra")
    label = label .. " · " .. (verb and verb .. " · " or "") .. table.concat(work, " · ")
  elseif stage == 1 and not site.piece then
    label = label .. " · A levantar o abrigo"
  elseif stage == 1 then
    ready = rules.item_count(site.piece) >= 1
    if ready then
      label = label .. " · Encaixar: " .. site.piece_name
    else
      label = label .. " · " .. site.piece_name .. " · " .. site.boss .. ", " .. site.place
    end
  elseif stage == 2 then
    label = label .. " · A fechar o tecto"
  else
    label = label .. " · Habitado"
  end
  if st.label ~= label then
    st.label = label
    vael.set_interaction(label, "e", 4)
  end
  if vael.menu_open() then return end
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  if stage == 2 or (stage == 1 and not site.piece) then
    st.finish = (st.finish or 0) + dt
    if st.finish >= 2 then
      st.finish = 0
      inhabit(site, game, key)
    end
    return
  end
  st.finish = 0
  if talk.active() or not vael.interacted("e") then return end
  if stage == 0 then
    local delivered, complete = deliver(site, game)
    if complete then
      game[key] = 1
      vael.sound("chest_open")
      if site.piece then
        talk.say(site.piece_name .. " · " .. site.boss .. ", " .. site.place .. ".", 6)
      end
      vael.save_slot(0)
    elseif delivered then
      talk.say("A obra ficou guardada no celeiro.", 4)
      vael.save_slot(0)
    else
      talk.say(label, 6)
    end
  elseif stage == 1 and ready and rules.vault_take(site.piece, 1) then
    game[key] = 2
    vael.sound("chest_open")
    vael.save_slot(0)
  elseif stage == 3 then
    talk.say(site.opens, 5)
  else
    talk.say(label, 6)
  end
end
