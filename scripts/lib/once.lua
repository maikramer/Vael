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

function M.taken()
  local mark = viber.game()[M.key()]
  -- Um save editado à mão pode trazer a marca como texto/bool ("1" > 0
  -- rebenta a comparação e o script do baú/POI falhava em TODOS os frames).
  return type(mark) == "number" and mark > 0
end

function M.mark()
  viber.game()[M.key()] = 1
end

return M
