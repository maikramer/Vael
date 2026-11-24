local M = { sites = {}, by_boss = {}, materials = {} }

local data = vael.load_json("data/concordia.json") or {}
for _, site in ipairs(data.sites or {}) do
  M.sites[site.id] = site
  if site.kill then M.by_boss[site.kill] = site end
  for _, cost in ipairs(site.work) do M.materials[cost[1]] = true end
end

function M.stage(id)
  return vael.game()["concordia." .. id] or 0
end

function M.open(id)
  return M.stage(id) == 3
end

function M.grant_piece(rules, kind)
  local site = M.by_boss[kind]
  if not site or M.open(site.id) then return end
  local game = vael.game()
  local key = "concordia.reward." .. site.id
  if game[key] then return end
  if rules.item_add(site.piece, 1) == 1 or rules.item_count(site.piece) == 1 then
    game[key] = true
    vael.toast("Recuperaste: " .. site.piece_name .. ". Leva a peça ao canteiro " .. site.name .. ".")
  end
end

return M
