-- game/equipment.lua: o que o herói leva na mão é CONTEÚDO do jogo.
--
-- A engine só tem `viber.attach(slot, url, {bone, x, y, z, rx, ry, rz, scale})`:
-- encontra o osso pelo nome e pendura o GLB. A tabela de armas/grips e a
-- decisão de qual usar vivem aqui, não no Rust.
--
-- Host: `world/gameplay.xml` (`tag="always-active"`).

local SWORD = {
  url = "assets/meshes/props/sword_hero_lod0.glb",
  bone = "hand_r",
  x = 0.12, y = 0.04, z = 0.04,
  rx = -1.33, ry = 12.71, rz = 0.96,
  scale = 1.0,
}

function on_update(dt)
  local st = viber.state()
  local pid = viber.player_id()
  -- O herói pode ser recriado (cena/load): reanexa quando o dono muda.
  if st.owner ~= pid then
    st.owner = pid
    st.attach_t = 0
    st.tries = 0
  end
  if not pid then
    return
  end
  -- Nos primeiros frames o GLB do herói pode ainda não ter hierarchy: o
  -- attach falhava (osso não encontrado) e nunca era repetido. Tenta de
  -- novo durante ~2 s; o slot substitui, não acumula.
  if (st.tries or 0) >= 4 then
    return
  end
  st.attach_t = (st.attach_t or 0) - dt
  if st.attach_t <= 0 then
    st.attach_t = 0.5
    st.tries = (st.tries or 0) + 1
    viber.attach("weapon", SWORD.url, SWORD)
  end
end
