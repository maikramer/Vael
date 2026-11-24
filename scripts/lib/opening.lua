-- O jogador escolhe uma margem do mapa andando à volta do dador.
-- O prompt mostra sempre o destino antes da confirmação por interacção.
local M = {}

function M.region(dx, dz)
  if math.abs(dx) > math.abs(dz) then
    if dx > 0 then return "desert_worm", "Assinar para o Ermo · Kadi" end
    return "swamp_bogwarden", "Assinar para o Lodo · Onno"
  end
  if dz < 0 then return "peaks_ogre", "Assinar para os Picos · Sig" end
  return "forest_witch", "Assinar para o Bosque · Vessa"
end

function M.legend(dx, dz)
  if dz < -math.abs(dx) then return "yard", "Experimentar a seta do curral" end
  if dx < 0 then return "wall", "Desenhar a esquina da cerca" end
  return "roof", "Desenhar o telhado branco"
end

function M.offset()
  local has, px, _, pz = vael.player_position()
  if not has then return nil end
  local x, _, z = vael.position()
  return px - x, pz - z
end

function M.prompt(st, label)
  if st.label ~= label then
    st.label = label
    vael.set_interaction(label, "e", 3.5)
  end
end

return M
