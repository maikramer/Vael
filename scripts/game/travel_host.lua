-- Controller sempre activo. A lógica está em game/travel.lua (módulo);
-- este ficheiro só existe porque um script de entidade não pode `return`.

local travel = viber.load("game/travel.lua")

function on_update(dt)
  travel.update(dt)
end
