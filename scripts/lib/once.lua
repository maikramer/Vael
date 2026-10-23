-- lib/once.lua: marca de "já feito" que SOBREVIVE ao save.
--
-- `viber.state()` é por entidade viva. Um load cria entidades novas: o baú,
-- o musgo e a relíquia voltavam a pagar. A marca vive em `viber.game()`
-- (primitivo, vai no save), chaveada pelo nome da entidade (ou pela pose,
-- quando não tem nome).
--
--   local once = viber.load("lib/once.lua")
--   if once.taken() then … end
--   once.mark()

local M = {}

function M.key()
  -- Cache por ENTIDADE: `taken()` e `mark()` calculavam a chave outra vez;
  -- um prop que se mexa entre os dois escrevia numa chave diferente.
  local st = viber.state()
  if st.once_key then
    return st.once_key
  end
  local name = viber.entity_name()
  local key
  if name and name ~= "" then
    key = "once:" .. name
  else
    -- Inclui Y e 1 casa decimal: XZ arredondado ao metro fazia dois
    -- coletáveis no mesmo quadrado (ou em pisos diferentes) partilhar marca.
    local x, y, z = viber.position()
    key = string.format("once:%.1f:%.1f:%.1f", x or 0, y or 0, z or 0)
  end
  st.once_key = key
  return key
end

-- Chave dos saves anteriores à chave 3D: os POIs SEM nome gravavam
-- `once:<x0>:<z0>` (XZ arredondado ao metro, sem Y). Sem esta leitura, um
-- save antigo dava o coletável outra vez quando a chave mudou.
local function legacy_key()
  local st = viber.state()
  if st.once_legacy ~= nil then
    return st.once_legacy or nil
  end
  local name = viber.entity_name()
  if name and name ~= "" then
    st.once_legacy = false
    return nil
  end
  local x, _, z = viber.position()
  st.once_legacy = string.format("once:%.0f:%.0f", x or 0, z or 0)
  return st.once_legacy
end

function M.taken()
  local game = viber.game()
  local mark = game[M.key()]
  -- Um save editado à mão pode trazer a marca como texto/bool ("1" > 0
  -- rebenta a comparação e o script do baú/POI falhava em TODOS os frames).
  if type(mark) == "number" and mark > 0 then
    return true
  end
  -- Migração: encontra a marca pela chave antiga e copia-a para a nova.
  local old = legacy_key()
  if old then
    local legacy = game[old]
    if type(legacy) == "number" and legacy > 0 then
      game[M.key()] = legacy
      return true
    end
  end
  return false
end

function M.mark()
  viber.game()[M.key()] = 1
end

return M
