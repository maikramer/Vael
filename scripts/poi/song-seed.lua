local rules = viber.load("lib/rules.lua")
-- poi/song-seed.lua — Semente de Canção (o "Korok" de Vael).
--
-- Colecionável escondido com MICRO-PUZZLE, recompensa própria (item
-- "korok-seed", nunca ouro) e contador de coleção: a cada 5 sementes,
-- +1 ponto de talento (rules.add_skill_points). Persistência: o estado de
-- coleta vive em `viber.game()` (world_kv) → sobrevive a save→load; a
-- entidade despawnata-se ao ser coletada e volta a auto-despintar-se na
-- próxima sessão se a semente já foi contada.
--
-- Indicador de atenção (tese BotW §5.1.2, gravity): coluna de fumo subindo
-- do esconderijo — visível de longe, significado descoberto de perto.
--
-- MODOS (prop-mode):
--   "rock"  — erguer a pedra solitária: [E] e a pedra abre-se em faíscas
--             (a semente estava por baixo).
--   "odd"   — a pedra que canta: num grupo de pedras, UMA brilha; [E] nela.
--   "ring"  — anel de notas: 3 cristais (prop-index 1..3, MESMO prop-group);
--             acender os três libera a semente no último.
--   "hide"  — descoberta pura: a semente está escondida (atrás de relevo,
--             sob ponte, na boca da gruta); o fumo denuncia o sítio, não o
--             objeto.
--
--@export mode: enum(rock,odd,ring,hide) = "rock"
--@export group: string = ""              -- id do anel (mode="ring"; igual nos 3)
--@export index: int(1..3) = 1            -- nota deste cristal (mode="ring")
--@export prompt: string = "Erguer a pedra"
--@export range: number(0.5..10) = 2.2    -- alcance do [E] (m)
--@export seed_key: string = ""           -- chave de persistência (default: nome da entidade)

-- Recompensas REAIS: 22. O mundo tem 28 entidades, mas cada anel de 3
-- cristais paga UMA semente no fecho (3 anéis = 3 sementes, não 9).
local TOTAL_REWARDS = 22

-- Herói caído: os 2,2 s de `dying` antecedem o modal de morte, por isso o
-- [E] ainda chega ao script nessa janela. `heal_player` recusa; a semente
-- não pode ser colhida por um cadáver.
local function hero_down()
  local alive, hp = rules.player_hp()
  return alive and (hp or 0) <= 0
end

-- Tecto da pilha é DADO (`data/items.json`): `rules.vault_add` usa o `stack`
-- do próprio item quando existe, não o `stack_max` global. O guard tem de
-- ler o mesmo número — senão um `stack` mais baixo deixa o teste passar e o
-- `item_add` devolver 0 com a recompensa já marcada.
local function seed_cap()
  local def = rules.item_def and rules.item_def("korok-seed")
  if def and def.kind ~= "resource" then
    local stack = tonumber(def.stack)
    if stack and stack > 0 then
      return math.floor(stack)
    end
  end
  return rules.stack_max() or 99
end

function on_update(dt)
  local st = viber.state()

  local function reward_key()
    if st.mode == "ring" then
      return "korok-reward-" .. viber.prop("group")
    end
    return st.key
  end

  local function already_rewarded(game)
    return (game[reward_key()] or 0) > 0
  end

  if not st.ready then
    st.ready = true
    st.mode = viber.prop("mode")
    local name = viber.entity_name() or "anon"
    st.key = "korok-" .. (viber.prop("seed_key") ~= "" and viber.prop("seed_key") or name)
    local game = viber.game()
    st.taken = (game[st.key] or 0) > 0
    if st.mode == "ring" then
      st.ring_key = "korokring-" .. viber.prop("group")
      st.ring_done = (game[st.ring_key] or 0) >= 3 or already_rewarded(game)
    end
    if st.taken or (st.mode == "ring" and st.ring_done) then
      -- Já coletada (sessão anterior): sai de cena sem prompt.
      viber.despawn_self()
      return
    end
    viber.set_interaction(viber.prop("prompt"), "e", viber.prop("range"))
  end

  local function collect(phrase)
    local game = viber.game()
    -- O anel marca a recompensa no GRUPO, não no cristal que fechou.
    -- Sem isto os outros dois cristais (ainda no mundo neste frame)
    -- também chamavam collect e davam semente extra + talento.
    if already_rewarded(game) then
      viber.despawn_self()
      return
    end
    -- Bolsa cheia: NÃO marca a recompensa nem despawna. Antes a semente era
    -- gasta (item_add devolvia 0) e o anel fechava sem prémio possível.
    -- (Depois do teste de recompensa: uma semente já paga não fica no mundo
    -- só porque a bolsa encheu.)
    local cap = seed_cap()
    if rules.vault_get("korok-seed") >= cap then
      viber.toast("A bolsa de sementes está cheia (" .. cap .. ").")
      return
    end
    -- Crédito PRIMEIRO: a marca de recompensa é permanente e o `item_add`
    -- podia recusar (tecto). Marcar antes de a semente entrar perdia-a para
    -- sempre — o anel/cristal já não voltava a pagar.
    local added = rules.item_add("korok-seed", 1) or 0
    if added < 1 then
      viber.toast("A bolsa de sementes está cheia (" .. cap .. ").")
      return
    end
    game[reward_key()] = 1
    game[st.key] = 1
    local okp, x, y, z = viber.entity_position()
    viber.despawn_self()
    if okp then
      viber.burst("sparkle", x, y + 0.6, z, 16)
    end
    viber.sound("loot")
    local count = rules.vault_get("korok-seed")
    local every = rules.talent_every()
    if every > 0 and count % every == 0 then
      rules.add_skill_points(1)
      viber.toast(phrase .. " — semente " .. count .. "/" .. TOTAL_REWARDS .. ": +1 PONTO DE TALENTO!")
    else
      viber.toast(phrase .. " (" .. count .. " na bolsa)")
    end
  end

  if st.mode == "ring" then
    local game = viber.game()
    -- Relê o contador todos os frames: `st.ring_done` era só no arranque,
    -- por isso os cristais que sobravam continuavam a pagar semente.
    if st.ring_done or (game[st.ring_key] or 0) >= 3 or already_rewarded(game) then
      viber.despawn_self()
      return
    end
    -- Cristal já aceso num save carregado na MESMA cena: a entidade não é
    -- recriada pelo load, por isso a marca individual tem de ser relida.
    if (game[st.key] or 0) > 0 then
      st.taken = true
      viber.despawn_self()
      return
    end
    if viber.menu_open() or hero_down() then return end
    if viber.interacted("e") then
      if (game[st.key] or 0) > 0 then
        viber.despawn_self()
        return
      end
      local cap = seed_cap()
      if rules.vault_get("korok-seed") >= cap then
        viber.toast("A bolsa de sementes está cheia (" .. cap .. ").")
        return
      end
      game[st.key] = 1
      local n = (game[st.ring_key] or 0) + 1
      game[st.ring_key] = n
      local x, y, z = viber.position()
      viber.despawn_self()
      viber.burst("sparkle", x, y + 0.4, z, 12)
      if n >= 3 then
        collect("O anel ressoa inteiro: semente de canção!")
      else
        viber.sound("ui")
        viber.toast("Uma nota se acende no ar (" .. n .. "/3)...")
      end
    end
    return
  end

  -- rock / odd / hide: um [E] e a semente é tua.
  -- A marca tem de ser relida todos os frames: um Load na MESMA cena troca
  -- `viber.game()` sem recriar a entidade, e com `st.taken` só do arranque a
  -- semente colectada no save voltava a pagar (semente + talento).
  local game = viber.game()
  if (game[st.key] or 0) > 0 then
    st.taken = true
    viber.despawn_self()
    return
  end
  if viber.menu_open() or hero_down() then return end
  if viber.interacted("e") and not st.taken then
    local phrase = ({
      rock = "A pedra ergue-se: uma semente de canção!",
      odd = "A pedra que canta guarda uma semente!",
      hide = "Escondida onde ninguém olha: uma semente de canção!",
    })[st.mode] or "Uma semente de canção!"
    collect(phrase)
  end
end
