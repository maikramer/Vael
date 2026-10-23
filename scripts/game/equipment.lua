-- game/equipment.lua: o que o herói leva na mão é CONTEÚDO do jogo.
--
-- A engine só tem `viber.attach(slot, url, {bone, x, y, z, rx, ry, rz, scale})`:
-- encontra o osso pelo nome e pendura o GLB. A tabela de armas/grips e a
-- decisão de qual usar vivem aqui, não no Rust.
--
-- `V` percorre a lista. O slot `weapon` SUBSTITUI o GLB (não acumula), por
-- isso a troca é imediata. O lock-on do combate deixou de ler o V — usa a
-- ação nativa `lock` (ver game/combat.lua).
--
-- Host: `world/gameplay.xml` em `main` (props do editor) e
-- `world/autoload/player.xml` (`game-lua-equipment-interior`) nas cenas
-- próprias — sem isto o V e o grip desapareciam dentro das casas.

-- Grip/offsets EXPORTADOS: o nó do host guarda os valores em `prop-*`; o
-- Inspector do editor (secção Script), o `viber scene set` e o jogo leem o
-- mesmo sítio. Distâncias em metros, ângulos em GRAUS (ordem XYZ, a mesma do
-- `viber.attach`). Sem `prop-*`, vale o default da anotação.
--
-- `item-inicial` escolhe o item do arranque; `preview` escolhe a arma ou
-- ferramenta que o editor pré-visualiza. Em Edit os scripts não correm: o
-- plugin do editor prende o GLB ao osso lendo estas props (o nó marcado com
-- `groups="grip-preview"` vive em `world/gameplay.xml`).
-- `--@group` só agrupa os campos no Inspector; o XML e o `viber.prop` não mudam.
--@export preview: enum(espada, tocha, machado, picareta) = espada -- arma/ferramenta do preview no editor (o jogo ignora)
--@export item-inicial: enum(espada, tocha) = espada -- equipado no arranque
--@export osso: string = hand_r -- osso do rig onde o grip é preso
--@export braco-tocha: number = 25.7831 -- braço à frente com a tocha, em graus
--
--@group Espada
--@export espada-modelo: string = assets/meshes/props/sword_hero_lod0.glb -- GLB da espada
--@export espada-x: number = 0.12 -- posição no osso, em metros
--@export espada-y: number = 0.04
--@export espada-z: number = 0.04
--@export espada-rx: number = -76.2034 -- rotação no osso, em graus
--@export espada-ry: number = 8.2294
--@export espada-rz: number = 55.0039
--@export espada-escala: number(0.01..20) = 1
--
--@group Tocha
--@export tocha-modelo: string = assets/meshes/interiors/lantern_hanging_lod0.glb -- GLB da tocha
--@export tocha-x: number = 0.12 -- posição no osso, em metros
--@export tocha-y: number = 0.32
--@export tocha-z: number = 0
--@export tocha-rx: number = 180 -- rotação no osso, em graus
--@export tocha-ry: number = 0
--@export tocha-rz: number = 0
--@export tocha-escala: number(0.01..20) = 0.4
--
--@group Machado
--@export machado-modelo: string = assets/meshes/props/felling_axe_lod0.glb -- GLB do machado
--@export machado-x: number = 0.08 -- posição no osso, em metros
--@export machado-y: number = 0.05
--@export machado-z: number = 0.01
--@export machado-rx: number = -25.394 -- rotação no osso, em graus
--@export machado-ry: number = 167.115
--@export machado-rz: number = -34.95
--@export machado-escala: number(0.01..20) = 1
--
--@group Picareta
--@export picareta-modelo: string = assets/meshes/props/pickaxe_lod0.glb -- GLB da picareta
--@export picareta-x: number = 0.06 -- posição no osso, em metros
--@export picareta-y: number = 0.05
--@export picareta-z: number = 0.07
--@export picareta-rx: number = 55.185 -- rotação no osso, em graus
--@export picareta-ry: number = -67.976
--@export picareta-rz: number = 115.164
--@export picareta-escala: number(0.01..20) = 1
--
-- A tocha usa o GLB da lanterna pendurada do pool (não há tocha de mão
-- própria). O vidro/brasa é emissivo, portanto lê-se acesa com o bloom; à
-- noite a lanterna do viajante ilumina o caminho.
-- Grip: o `hand_r` aponta o +Y local para BAIXO do braço; a lanterna tem a
-- BASE na origem do GLB e o aro no TOPO (≈0,32 m na escala 0,4). `rx = 180°`
-- põe o aro PARA CIMA; `y = 0.32` desce a base uma altura de modelo, portanto
-- o ARO fica na mão e o corpo pende ABAIXO dela. Quem inclina à frente é o
-- BRAÇO (`braco-tocha`), não o lampião — o corpo fica vertical.
-- A morte bloqueia o V (o herói caído não troca de arma enquanto o ecrã de
-- morte não abre). É a mesma fonte de HP que o hotbar/combate usam.
local rules = viber.load("lib/rules.lua")
local actions = viber.load("lib/actions.lua")

local ITEMS = {
  { name = "Espada", kind = "sword", prefix = "espada" },
  { name = "Tocha", kind = "torch", prefix = "tocha" },
}

-- O grip é lido das props a cada frame: mudar um campo no Inspector reanexa
-- no frame seguinte (o editor faz o mesmo em Edit, sem correr este script).
local function grip_for(prefix)
  return {
    url = viber.prop(prefix .. "-modelo"),
    bone = viber.prop("osso"),
    x = viber.prop(prefix .. "-x"),
    y = viber.prop(prefix .. "-y"),
    z = viber.prop(prefix .. "-z"),
    rx = math.rad(viber.prop(prefix .. "-rx")),
    ry = math.rad(viber.prop(prefix .. "-ry")),
    rz = math.rad(viber.prop(prefix .. "-rz")),
    scale = viber.prop(prefix .. "-escala"),
  }
end

-- Assinatura do grip: detecta edições às props ao vivo sem reanexar por frame.
local function grip_signature(grip)
  return table.concat({
    grip.url, grip.bone,
    grip.x, grip.y, grip.z,
    grip.rx, grip.ry, grip.rz,
    grip.scale,
  }, "|")
end

-- Ferramentas de colheita: `game/harvest.lua` publica `harvest.tool`
-- ("chop"/"mine") enquanto há alvo. Os grips são props como os dos itens
-- (defaults antigos do `held-items.json` da referência), por isso também se
-- afinam no Inspector. O slot `weapon` é ÚNICO: a ferramenta substitui o item
-- equipado e o item volta sozinho quando o alvo desaparece. O V fica bloqueado
-- com ferramenta na mão — os dois nunca disputam o slot.
local HARVEST_PREFIX = { chop = "machado", mine = "picareta" }

-- Intensidade da luz da tocha (lm). 600k = LANTERN_MAX_INTENSITY do engine;
-- acende de dia e reforça a lanterna automática da noite.
local TORCH_LIGHT = 600000

-- Diálogo/mapa abertos: o V é do grafo/UI, não do equipamento.
local function ui_locked()
  if viber.menu_open() then
    return true
  end
  -- O título também é uma UI: o V não troca de item por baixo dele.
  if viber.ui.is_open and viber.ui.is_open("title") then
    return true
  end
  local dialogue_until = tonumber(viber.ui.get("dialogue.until")) or 0
  local left = dialogue_until - viber.time()
  -- Um timestamp muito à frente é resto de outra sessão (`viber.time()`
  -- recomeça no load): não pode trancar o V para sempre.
  return left > 0 and left <= 2.0
end

function on_update(dt)
  local st = viber.state()
  -- Dois hosts do MESMO script: o de `world/gameplay.xml` (main, com as
  -- props do editor) e o do autoload (`game-lua-equipment-interior`), para o
  -- V e o grip continuarem vivos dentro das casas. Cada um só acorda na sua
  -- cena; em `main` correr os dois anexava/trocava duas vezes.
  if not st.role then
    st.role = viber.entity_name() == "game-lua-equipment-interior" and "interior" or "main"
  end
  if st.role == "interior" then
    local scene = viber.scene_name and viber.scene_name() or nil
    -- Mundo avulso (QA) e `main` são do host de `gameplay.xml`.
    if scene == nil or scene == "main" then
      -- Fora da cena própria, o estado visual em cache é do outro host: a
      -- próxima entrada é uma cena NOVA para este controller.
      st.scene = nil
      return
    end
    if st.scene ~= scene then
      -- Entrar numa casa/menu depois de `main` (ou da casa anterior): o outro
      -- host pode ter posto a FERRAMENTA de colheita na mão e apagado a
      -- tocha. O `grip_key`/`lantern_kind`/`arm` em cache não valem — reaplica
      -- o item guardado no jogo, senão o machado (ou uma tocha sem luz)
      -- ficava na mão dentro da casa/título.
      st.scene = scene
      st.grip_key, st.kind, st.lantern_kind, st.arm = nil, nil, nil, nil
      st.tries, st.attach_t = 0, 0
    end
  end
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
  local game = viber.game()
  -- Item escolhido vive no JOGO (primitivo no save): trocar para tocha em
  -- `main` e entrar numa casa não pode voltar a espada no host do interior.
  local index = game.equip_index
  if type(index) ~= "number" then
    -- Sem índice no jogo (arranque/save antigo), vale SEMPRE o prop. O
    -- `st.index` de uma sessão anterior não pode decidir a arma de um jogo
    -- novo carregado directamente num interior.
    index = (viber.prop("item-inicial") == "tocha") and 2 or 1
  end
  -- Um save editado com 1.5 (ou NaN) fazia `ITEMS[index]` nil no V.
  index = math.floor(index)
  if index ~= index or index < 1 or index > #ITEMS then
    index = 1
  end
  st.index = index
  game.equip_index = index
  -- `game/harvest.lua` publica o tipo do alvo; com ferramenta na mão o V
  -- fica bloqueado (o slot é um só). Só o host de `main` a lê: o
  -- `harvest.tool` do bind fica no último valor ao entrar numa casa e o
  -- herói aparecia com o machado na mão dentro do salão.
  local tool = ""
  if st.role ~= "interior" then
    tool = viber.ui.get("harvest.tool")
  end
  if type(tool) ~= "string" then
    tool = ""
  end
  local tool_prefix = HARVEST_PREFIX[tool]
  local harvest_grip = tool_prefix and grip_for(tool_prefix) or nil
  local alive, hp = rules.player_hp()
  local dead = alive and hp <= 0
  if not ui_locked() and not harvest_grip and not dead and viber.input.pressed("v") then
    index = index % #ITEMS + 1
    st.index = index
    viber.game().equip_index = index
    actions.record("equipar", "item", ITEMS[index].kind)
    -- Reanexa no mesmo frame: o slot substitui o GLB anterior.
    st.tries = 0
    st.attach_t = 0
    viber.toast("Equipado: " .. ITEMS[index].name)
  end
  -- O grip pedido mudou (item, ferramenta ou offset editado ao vivo): reanexa
  -- já. A assinatura detecta também a chegada/partida do alvo de colheita e
  -- edições às props sem reanexar por frame.
  local grip = harvest_grip or grip_for(ITEMS[index].prefix)
  local grip_key = (harvest_grip and ("tool:" .. tool) or ("item:" .. index))
    .. "|" .. grip_signature(grip)
  if st.grip_key ~= grip_key then
    st.grip_key = grip_key
    st.tries = 0
    st.attach_t = 0
  end
  -- O combate lê isto: com a tocha na mão não há arma para atacar. A
  -- `viber.lantern` acende a luz da tocha (o `if` protege engines antigas
  -- durante o hot-reload, antes do rebuild). `lantern_kind` é separado do
  -- `kind` para o hot-reload REAPLICAR a luz mesmo que o item não mude.
  -- Com uma FERRAMENTA de colheita o slot `weapon` é a ferramenta, não o
  -- item: a tocha ficava acesa (e o braço inclinado) por baixo do machado.
  -- "tool" não é "torch", por isso o combate continua disponível.
  local kind = harvest_grip and "tool" or ITEMS[index].kind
  if st.kind ~= kind then
    st.kind = kind
    viber.ui.set("equip.kind", kind)
  end
  if viber.lantern and st.lantern_kind ~= kind then
    st.lantern_kind = kind
    viber.lantern(kind == "torch" and TORCH_LIGHT or 0)
  end
  -- Braço à frente com a tocha: rotação ADITIVA no `upperarm_r`, somada à
  -- animação de idle/walk/run. `st.arm` guarda o valor enviado para o
  -- hot-reload reaplicar quando `braco-tocha` mudar.
  local arm = (kind == "torch") and math.rad(viber.prop("braco-tocha")) or 0
  if viber.bone_offset and st.arm ~= arm then
    st.arm = arm
    viber.bone_offset("upperarm_r", arm, 0, 0)
  end
  -- Nos primeiros frames o GLB do herói pode ainda não ter hierarchy: o
  -- attach falhava (osso não encontrado) e nunca era repetido. Tenta de
  -- novo durante ~5 s (10 × 0,5 s); num disco lento 2 s não chegavam.
  if (st.tries or 0) >= 10 then
    return
  end
  st.attach_t = (st.attach_t or 0) - dt
  if st.attach_t <= 0 then
    st.attach_t = 0.5
    st.tries = (st.tries or 0) + 1
    viber.attach("weapon", grip.url, grip)
  end
end
