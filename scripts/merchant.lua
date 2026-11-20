-- merchant.lua: Bram, o mercador. [E] gira o player, fala com gesto e som;
-- fora de alcance a banter recomeça.
--
-- A LOJA vive em game/shop.lua (catálogo, preços e negociação em Lua).
-- Este script tem de correr SEMPRE (`tag="always-active"`): o LOD de 45 m
-- engolia os cliques buy/sell e despejava-os todos ao regressar à banca.
local shop = vael.load("game/shop.lua")
local rules = vael.load("lib/rules.lua")
-- Reação a esbarrão (o herói empurra a banca; o Bram tropeça/cede).
local bump = vael.load("lib/bump.lua")

function on_collision_enter(other)
  bump.enter(vael.state(), other)
end

function on_collision_stay(other, dt)
  bump.stay(vael.state(), other, dt)
end

function on_collision_exit(other)
  bump.exit(vael.state(), other)
end

--@export prompt: string = "Falar com Bram"     -- texto do prompt de interação
--@export range: number(0.5..10) = 3.5          -- alcance do [E] (m)
--@export lines: string[] = "“Ferro bom não se negocia barato, forasteiro.”" | "“Precisa de uma lâmina? Chegou na hora certa.”" | "“Dizem que há cristais nas ruínas ao leste...”"
function on_update(dt)
  local st = vael.state() -- POR ENTIDADE (no top-level era partilhado)
  bump.sample(st, vael.time())
  if bump.tick(st, vael.time()) then
    -- A loja continua a drenar a fila mesmo durante o tropeço.
    shop.update(dt)
    return
  end
  if not st.ready then
    st.ready = true
    vael.set_interaction(vael.prop("prompt"), "e", vael.prop("range"))
  end
  local dist = vael.distance_to_player()
  -- O alcance do [E] é o AUTORADO escalado pela config global; comparar com
  -- o valor cru (3.5 m) mantinha a banter viva até ao dobro do alcance real.
  local near = dist ~= nil and dist <= vael.interact_range(vael.prop("range"))
  if st.near and not near then
    st.i = nil -- saiu de alcance: banter recomeça
  end
  st.near = near
  -- A loja corre SEMPRE, inclusive com o menu aberto: era a única coisa que
  -- drenava buy/sell, e a banca abre o menu no frame seguinte.
  shop.update(dt)
  if vael.menu_open() then return end
  -- A fila da loja continua a ser drenada (é este host que a drena), mas o
  -- herói caído não fala nem abre a banca: `menu_open` não cobre a morte.
  local _, hp = rules.player_hp()
  if hp <= 0 then return end
  if vael.interacted("e") then
    local lines = vael.prop("lines")
    st.i = ((st.i or 0) % math.max(#lines, 1)) + 1
    vael.face_player()
    vael.gesture("talk")
    if st.i == 1 then
      vael.sound("shop_open") -- sino de boas-vindas na 1.ª fala
    end
    if lines[st.i] then
      vael.toast(lines[st.i])
    end
    -- Falar com o Bram abre a banca: o diário no separador Loja (antes a
    -- loja só se achava sabendo que existia um separador no [Q]).
    vael.ui.open("menu", true)
    vael.ui.select_tab("menu", "shop")
  end
end
