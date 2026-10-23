-- cinematic-trigger.lua: toca uma `<Cinematic>` quando o herói ENTRA no raio.
-- Tudo é DADO da instância (Inspector › Script ou `viber scene set`):
--   prop-cinematic="campo-de-batalha" prop-radius="18"
-- "Só uma vez" vale por save: a marca vive em `viber.game()` (persiste).
--@export cinematic: string = ""            -- id da `<Cinematic>` (viber cinematic list)
--@export radius: number(1..120) = 14        -- raio de entrada (m)
--@export once: bool = true                  -- só a primeira vez (por save)

function on_update(dt)
  local st = viber.state()
  local id = viber.prop("cinematic")
  if id == "" then
    st.inside = false
    return
  end
  local has, px, _, pz = viber.player_position()
  if not has then
    st.inside = false
    return
  end
  local x, _, z = viber.position()
  local radius = viber.prop("radius")
  local inside = (px - x) ^ 2 + (pz - z) ^ 2 <= radius * radius
  -- Um menu aberto (mapa, diário) ou outra cinemática não pode disparar a
  -- cena por baixo dele. Não consome a entrada: quando fechar, o gatilho
  -- ainda vê o herói dentro.
  if viber.cinematic_playing() or viber.menu_open() then return end
  -- Só na ENTRADA. Sem este bordo, um gatilho com `once="false"` voltava a
  -- disparar no frame seguinte ao fim da cena, em ciclo, enquanto o herói
  -- ficasse dentro do raio.
  local entering = inside and not st.inside
  st.inside = inside
  if not entering then return end
  local g = viber.game()
  local key = "cine_" .. id
  -- A marca só existe quando `once` é verdadeiro: um gatilho `once="false"`
  -- gravava-a na mesma e desligava para sempre outro gatilho (once=true) do
  -- mesmo filme.
  local once = viber.prop("once")
  if once and g[key] then return end
  if once then
    g[key] = true
  end
  viber.cinematic(id)
end
