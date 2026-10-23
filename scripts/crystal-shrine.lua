local rules = viber.load("lib/rules.lua")
-- crystal-shrine.lua: dois modos, escolhidos por prop:
--   * `riddle_id` vazio — comportamento antigo: orar dá XP uma vez;
--   * `riddle_id` definido — enigma mudo (data/riddles.json): a pista é som,
--     objetos ou estátuas; a resposta é uma palavra validada pelo modelo.
--@export riddle_id: string = ""

local once = viber.load("lib/once.lua")
local riddle = viber.load("lib/riddle.lua")

local PROMPT, DONE_PROMPT = "Orar ao santuário", "O santuário está em silêncio"

local function set_prompt(done)
  viber.set_interaction(done and DONE_PROMPT or PROMPT, "e", 3.0)
end

-- Herói caído: o modal de morte só abre ao fim dos 2,2 s de `dying`, e nessa
-- janela o [E] ainda chega ao santuário. Nenhuma das duas variantes (XP do
-- oração ou prémio do enigma) pode pagar a um cadáver.
local function hero_down()
  local _, hp = rules.player_hp()
  return (hp or 0) <= 0
end

-- O prémio do enigma é pago por `lib/riddle.lua` DEPOIS de marcar o enigma
-- como resolvido; um item que não cabe na pilha perdia-se para sempre (o
-- `reward` só avisa "não coube"). Verifica-se a capacidade antes de abrir a
-- barra de resposta, para o jogador fazer espaço e voltar.
local function reward_fits(r)
  local rewards = r.reward or {}
  for _, item in ipairs(rewards.items or {}) do
    local name, count = tostring(item):match("^([^:]+):?(%d*)$")
    local want = tonumber(count) or 1
    local def = name and rules.item_def and rules.item_def(name)
    local cap
    if def and def.kind ~= "resource" then
      local stack = tonumber(def.stack)
      cap = (stack and stack > 0) and math.floor(stack) or rules.stack_max()
    end
    if cap and want > 0 and rules.vault_get(name) + want > cap then
      return false
    end
  end
  return true
end

local function update_riddle(st)
  local r = riddle.get(st.rid)
  if not r then
    return
  end
  local done = riddle.solved(st.rid)
  local label = done and "O santuário está em silêncio" or (r.interact or "Examinar o santuário")
  if st.label ~= label then
    st.label = label
    viber.set_interaction(label, "e", 3.0)
  end

  -- Pista sonora: toca enquanto o jogador estiver perto, sem spammar.
  local clue = r.clue or {}
  if not done and clue.kind == "audio" and clue.sound then
    local okp, px, py, pz = viber.player_position()
    local oks, sx, sy, sz = viber.entity_position()
    if okp and oks then
      local dx, dz = px - sx, pz - sz
      local dist = math.sqrt(dx * dx + dz * dz)
      local now = viber.time()
      if dist <= (clue.radius or 12) and now >= (st.sound_t or 0) then
        st.sound_t = now + (clue.period or 7)
        viber.sound(clue.sound)
      end
    end
  end

  if viber.menu_open() or hero_down() or not viber.interacted("e") then
    return
  end
  if done then
    viber.say("O santuário está em silêncio.")
    return
  end
  local met, missing = riddle.require_met(st.rid)
  if not met then
    viber.say("Faltam " .. missing .. " marcos por medir.")
    return
  end
  if not reward_fits(r) then
    viber.say("A tua bolsa está cheia para o que o santuário guarda.")
    return
  end
  riddle.ask(st.rid, function()
    st.label = nil -- re-regista o prompt no próximo frame
  end)
end

function on_update(dt)
  local st = viber.state() -- POR ENTIDADE (no top-level era partilhado)
  if not st.ready then
    st.ready = true
    st.rid = viber.prop("riddle_id") or ""
    if st.rid == "" then
      st.done = once.taken()
      set_prompt(st.done)
    end
  end
  if st.rid ~= "" then
    update_riddle(st)
    return
  end
  local taken = once.taken()
  if taken ~= st.done then
    st.done = taken
    set_prompt(taken)
  end
  if viber.menu_open() or hero_down() then return end
  if viber.interacted("e") and not st.done then
    st.done = true
    once.mark()
    set_prompt(true)
    rules.add_xp(20)
    viber.toast("Uma luz fria percorre o cristal (+20 XP).")
  end
end
