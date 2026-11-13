-- scripts/lib/voice.lua: Voz Viva e Combate Semântico (5 Acordes do Coro).
--
-- Classifica cantos e versos ditos pelo jogador [T] em 5 acordes
-- ("brasa", "geada", "tempestade", "raiz", "pacto") combinando correspondência
-- lexical normalizada, parley.resolve (Nomes Verdadeiros de criaturas), clima/hora
-- e decide.choice (desempate semântico opcional).

local parley = viber.load("lib/parley.lua")
local decide = viber.load("lib/decide.lua")
local environment = viber.load("lib/environment.lua")
local clock = viber.load("lib/clock.lua")
local rules = viber.load("lib/rules.lua")
local actions = viber.load("lib/actions.lua")

local ok_marks, marks = pcall(viber.load, "lib/landmarks.lua")
if not ok_marks or type(marks) ~= "table" then
  marks = nil
end

local M = {}

local CHORD_ORDER = { "brasa", "geada", "tempestade", "raiz" }

local CREATURE_LABELS = {
  wolf = "Lobo",
  bandit = "Bandido",
  ["bandit-archer"] = "Arqueiro",
  goblin = "Goblin",
  shade = "Sombra",
  scorpion = "Escorpião",
  slime = "Slime",
  bogling = "Limo",
  witch = "Bruxa",
  ["sand-worm"] = "Verme da Areia",
  ["bog-warden"] = "Guardião dos Afogados",
  boss = "Guardião",
}

local FALLBACK_CREATURE_ALIASES = {
  lobo = "wolf",
  lobos = "wolf",
  alcateia = "wolf",
  fera = "wolf",
  sombra = "shade",
  sombras = "shade",
  espectro = "shade",
  fantasma = "shade",
  bandido = "bandit",
  bandidos = "bandit",
  saqueador = "bandit",
  arqueiro = "bandit-archer",
  goblin = "goblin",
  goblins = "goblin",
  duende = "goblin",
  escorpiao = "scorpion",
  escorpioes = "scorpion",
  slime = "slime",
  slimes = "slime",
  lodo = "slime",
  gosma = "slime",
  bogling = "bogling",
  limo = "bogling",
  bruxa = "witch",
  feiticiera = "witch",
  feiticeira = "witch",
  ogro = "boss",
  gigante = "boss",
  verme = "sand-worm",
  guardiao = "bog-warden",
}

local PACT_VERBS = {
  cacar = true,
  caca = true,
  cacada = true,
  caco = true,
  pacto = true,
  jurar = true,
  juro = true,
  juramento = true,
  abater = true,
  abato = true,
  matar = true,
  mato = true,
  morte = true,
  presa = true,
  nome = true,
  selo = true,
  selar = true,
  vingar = true,
}

local CHORD_LEXICON = {
  brasa = {
    exact = {
      fogo = true, chama = true, chamas = true, sol = true, luz = true,
      cinza = true, cinzas = true, calor = true, brasa = true, brasas = true,
      tocha = true, tochas = true, ardor = true, verao = true, flama = true,
      flamas = true, fogueira = true, lume = true, quente = true,
    },
    prefixes = { "queim", "ardent", "incendi", "abrasa" },
  },
  geada = {
    exact = {
      gelo = true, frio = true, fria = true, pedra = true, pedras = true,
      rocha = true, rochas = true, montanha = true, montanhas = true,
      pico = true, picos = true, escudo = true, escudos = true, inverno = true,
      neve = true, geada = true, geadas = true, granizo = true, cristal = true,
      cristais = true, glacial = true, muralha = true,
    },
    prefixes = { "gelad", "congel", "proteg", "proteca", "petrif" },
  },
  tempestade = {
    exact = {
      vento = true, ventos = true, ventania = true, raio = true, raios = true,
      trovao = true, trovoes = true, trovoada = true, tempestade = true,
      tempestades = true, relampago = true, relampagos = true, ceu = true,
      ceus = true, velocidade = true, passo = true, passos = true,
      correr = true, corrida = true, corte = true, cortar = true, cortes = true,
      lamina = true, laminas = true, furacao = true, brisa = true,
      vendaval = true, sopro = true, celere = true, nuvem = true, nuvens = true,
      ar = true,
    },
    prefixes = { "relampag", "tempestad", "velocid", "trovoad", "ventan", "faisc" },
  },
  raiz = {
    exact = {
      vida = true, agua = true, aguas = true, cura = true, curar = true,
      floresta = true, florestas = true, semente = true, sementes = true,
      musgo = true, poco = true, pocos = true, terra = true, raiz = true,
      raizes = true, guia = true, guiar = true, caminho = true, caminhos = true,
      luzir = true, bosque = true, folha = true, folhas = true, arvore = true,
      arvores = true, rio = true, rios = true, nascente = true, solo = true,
      eco = true, ecos = true, broto = true, brotar = true, verde = true,
      orvalho = true,
    },
    prefixes = { "curat", "floresc", "sement", "raiz", "nascent", "germin" },
  },
}

local SHRINE_SITES = {
  { name = "bosque-shrine", riddle_id = "echo_wolf", label = "Santuário do Bosque" },
  { name = "vael-shrine", riddle_id = "three_gifts", label = "Santuário de Vael" },
  { name = "crystal-shrine", riddle_id = "odd_statue", label = "Santuário de Cristal" },
  { name = "menhir-route-shrine", riddle_id = "menhir_route", label = "Santuário da Rota do Menir" },
  { name = "desert-venom-shrine", riddle_id = "desert_venom", label = "Santuário do Espelho de Areia" },
  { name = "peaks-echo-shrine", riddle_id = "peaks_echo", label = "Santuário da Vigília do Gelo" },
}

local JUDGE_SITES = {
  { name = "judge-oath", judge_id = "hunter_oath", label = "Altar do Juramento" },
  { name = "judge-desert", judge_id = "desert_ascetic", label = "Altar do Voto da Sede" },
  { name = "judge-swamp", judge_id = "swamp_purifier", label = "Altar do Voto do Lodo" },
  { name = "judge-peaks", judge_id = "peaks_vigil", label = "Altar do Voto da Vigília" },
}

local ACCENTS = {
  ["á"] = "a", ["à"] = "a", ["â"] = "a", ["ã"] = "a", ["ä"] = "a",
  ["Á"] = "a", ["À"] = "a", ["Â"] = "a", ["Ã"] = "a", ["Ä"] = "a",
  ["é"] = "e", ["è"] = "e", ["ê"] = "e", ["ë"] = "e",
  ["É"] = "e", ["È"] = "e", ["Ê"] = "e", ["Ë"] = "e",
  ["í"] = "i", ["ì"] = "i", ["î"] = "i", ["ï"] = "i",
  ["Í"] = "i", ["Ì"] = "i", ["Î"] = "i", ["Ï"] = "i",
  ["ó"] = "o", ["ò"] = "o", ["ô"] = "o", ["õ"] = "o", ["ö"] = "o",
  ["Ó"] = "o", ["Ò"] = "o", ["Ô"] = "o", ["Õ"] = "o", ["Ö"] = "o",
  ["ú"] = "u", ["ù"] = "u", ["û"] = "u", ["ü"] = "u",
  ["Ú"] = "u", ["Ù"] = "u", ["Û"] = "u", ["Ü"] = "u",
  ["ç"] = "c", ["Ç"] = "c",
}

local function normalize(text)
  local s = tostring(text or "")
  for utf, asc in pairs(ACCENTS) do
    s = s:gsub(utf, asc)
  end
  s = s:lower()
  s = s:gsub("[%p%c]", " ")
  s = s:gsub("%s+", " ")
  s = s:gsub("^%s+", ""):gsub("%s+$", "")
  return s
end

local function tokenize(norm)
  local tokens = {}
  for tok in norm:gmatch("%S+") do
    tokens[#tokens + 1] = tok
  end
  return tokens
end

local function kind_in_hostiles(kind, near_hostiles)
  if type(near_hostiles) ~= "table" then
    return false
  end
  for _, h in ipairs(near_hostiles) do
    local hk = type(h) == "table" and h.kind or h
    if type(hk) == "string" and (hk == kind or (rules.kind_matches and rules.kind_matches(hk, kind))) then
      return true
    end
  end
  return false
end

local function token_matches_chord(tok, spec)
  if spec.exact[tok] then
    return true
  end
  for _, pref in ipairs(spec.prefixes) do
    if #tok >= #pref and tok:sub(1, #pref) == pref then
      return true
    end
  end
  return false
end

local function cardinal_dir(dx, dz)
  if math.abs(dx) >= math.abs(dz) then
    return dx >= 0 and "L" or "O"
  end
  return dz <= 0 and "N" or "S"
end

local function find_nearest_uncompleted_target(px, pz, g)
  if not (viber.find and viber.entity_position) then
    return nil
  end
  local best_dist, best_dx, best_dz, best_label

  local function consider_entity(eid, label)
    if not eid then
      return
    end
    local ok, tx, _, tz = viber.entity_position(eid)
    if not ok then
      return
    end
    local dx = tx - px
    local dz = tz - pz
    local dist = math.sqrt(dx * dx + dz * dz)
    if dist >= 4.0 and (not best_dist or dist < best_dist) then
      best_dist = dist
      best_dx = dx
      best_dz = dz
      best_label = label
    end
  end

  for _, sh in ipairs(SHRINE_SITES) do
    if g["riddle." .. sh.riddle_id] ~= true then
      consider_entity(viber.find(sh.name), sh.label)
    end
  end

  for _, jd in ipairs(JUDGE_SITES) do
    if g["judge." .. jd.judge_id .. ".done"] ~= true then
      consider_entity(viber.find(jd.name), jd.label)
    end
  end

  if viber.find_all and viber.entity_name then
    for _, eid in ipairs(viber.find_all("korok-")) do
      local ename = viber.entity_name(eid)
      if ename and ename:sub(1, 6) == "korok-" then
        local val = tonumber(g["korok-" .. ename]) or 0
        if val <= 0 then
          consider_entity(eid, "Semente do Coro")
        end
      end
    end
  end

  if marks and type(marks.LIST) == "table" then
    for _, mk in ipairs(marks.LIST) do
      if mk.name and g["mark." .. mk.name] ~= true then
        consider_entity(viber.find(mk.name), mk.label or mk.name)
      end
    end
  end

  if not best_dist then
    return nil
  end
  return {
    dir = cardinal_dir(best_dx, best_dz),
    dist = math.floor(best_dist + 0.5),
    label = best_label,
  }
end

function M.label_for_creature(kind)
  return CREATURE_LABELS[kind] or tostring(kind or "")
end

function M.classify(text, parsed, near_hostiles, opts)
  local norm = normalize(text)
  if norm == "" then
    return { chord = "dissonancia", score = 0, target_kind = nil, res = nil }
  end

  local tokens = tokenize(norm)
  local res = parley.resolve(text, parsed)

  local creature_kinds = {}
  if res and type(res.criatura) == "table" then
    for k, v in pairs(res.criatura) do
      if v then
        creature_kinds[#creature_kinds + 1] = k
      end
    end
  end
  if #creature_kinds == 0 then
    local seen = {}
    for _, tok in ipairs(tokens) do
      local mapped = FALLBACK_CREATURE_ALIASES[tok]
      if mapped and not seen[mapped] then
        seen[mapped] = true
        creature_kinds[#creature_kinds + 1] = mapped
      end
    end
  end
  table.sort(creature_kinds)

  local has_pact_verb = false
  for _, tok in ipairs(tokens) do
    if PACT_VERBS[tok] then
      has_pact_verb = true
      break
    end
  end

  local scores = { brasa = 0, geada = 0, tempestade = 0, raiz = 0 }
  for _, tok in ipairs(tokens) do
    for _, cname in ipairs(CHORD_ORDER) do
      if token_matches_chord(tok, CHORD_LEXICON[cname]) then
        scores[cname] = scores[cname] + 2
      end
    end
  end

  local best_chord = nil
  local best_score = 0
  local tied = {}
  for _, cname in ipairs(CHORD_ORDER) do
    local sc = scores[cname]
    if sc > best_score then
      best_score = sc
      best_chord = cname
      tied = { cname }
    elseif sc > 0 and sc == best_score then
      tied[#tied + 1] = cname
    end
  end

  if #creature_kinds > 0 then
    local matched_near = nil
    if near_hostiles ~= nil then
      for _, ck in ipairs(creature_kinds) do
        if kind_in_hostiles(ck, near_hostiles) then
          matched_near = ck
          break
        end
      end
    end
    local force = opts and opts.force_pact
    if near_hostiles == nil or matched_near ~= nil or force or has_pact_verb or best_score == 0 then
      return {
        chord = "pacto",
        target_kind = matched_near or creature_kinds[1],
        score = best_score,
        res = res,
      }
    end
  end

  if (best_score == 0 or #tied > 1) and decide.ready and decide.ready() and decide.decide then
    local ok_dec, ans = pcall(decide.decide, { utterance = text }, {
      q = decide.choice("A qual acorde elemental pertence este verso?", {
        brasa = "fogo, chama, calor, sol, tocha ou luz ardente",
        geada = "gelo, frio, pedra, neve, inverno ou escudo protetor",
        tempestade = "vento, raio, trovao, velocidade, passo ou corte",
        raiz = "agua, cura, floresta, semente, musgo, terra ou guia",
      }),
    })
    if ok_dec and type(ans) == "table" and ans.q and CHORD_LEXICON[ans.q.choice] then
      return {
        chord = ans.q.choice,
        target_kind = nil,
        score = best_score,
        via = "decide",
        res = res,
      }
    end
  end

  if best_chord then
    return {
      chord = best_chord,
      target_kind = nil,
      score = best_score,
      res = res,
    }
  end

  return {
    chord = "raiz",
    target_kind = nil,
    score = 0,
    fallback = true,
    res = res,
  }
end

function M.cast(text, parsed)
  local trimmed = tostring(text or ""):gsub("^%s+", ""):gsub("%s+$", "")
  if trimmed == "" then
    return { ok = false, reason = "empty" }
  end

  local alive, hp = rules.player_hp()
  if alive and hp <= 0 then
    return { ok = false, reason = "dead" }
  end

  local g = viber.game()
  local now = (viber.time and viber.time()) or 0
  local cd_until = tonumber(g.voice_cd_until) or 0
  if now < cd_until then
    local rem = math.max(1, math.ceil(cd_until - now))
    viber.toast("A tua voz ainda ressoa (" .. tostring(rem) .. "s).")
    return { ok = false, reason = "cooldown", remaining = cd_until - now }
  end

  local near_hostiles = rules.hostiles(18.0)
  local cls = M.classify(trimmed, parsed, near_hostiles)

  local has_pos, px, py, pz = false, 0, 0, 0
  if viber.player_position then
    has_pos, px, py, pz = viber.player_position()
  end
  if not has_pos then
    px, py, pz = 0, 0, 0
  end
  local pos = { x = px, y = py, z = pz }

  if cls.chord == "dissonancia" then
    if viber.sound then viber.sound("error", pos) end
    viber.toast("O Coro não reconheceu esse eco.")
    return { ok = false, reason = "empty" }
  end

  local stone_until = tonumber(g.voice_stone_boost_until) or 0
  local boosted = now < stone_until
  local cd_secs = boosted and 3.0 or 6.0
  g.voice_cd_until = now + cd_secs

  local action_id = cls.chord == "pacto" and (cls.target_kind or "pacto") or cls.chord
  if actions and actions.record then
    actions.record("entoar", "acorde", action_id)
  end

  local is_night = clock.is_night and clock.is_night() or false
  local is_rain = environment.raining and environment.raining() or false
  local snow = environment.snow and environment.snow() or nil
  local snow_cover = type(snow) == "table" and (tonumber(snow.cover) or tonumber(snow.depth) or 0) or (tonumber(snow) or 0)
  local has_snow = snow_cover > 0.05
  local wind = environment.wind and environment.wind() or nil
  local wind_speed = type(wind) == "table" and (tonumber(wind.speed) or 0) or (tonumber(wind) or 0)
  local wetness = environment.wetness and (tonumber(environment.wetness()) or 0) or 0

  if cls.chord == "brasa" then
    local dmg = is_night and 22 or 14
    local hits = rules.hostiles_near(px, pz, 6.5)
    for _, h in ipairs(hits) do
      rules.damage_enemy(h.id, dmg)
      rules.enemy_status(h.id, "burn", 5.0)
      if is_rain and viber.entity_knockback then
        local dx = (h.x or px) - px
        local dz = (h.z or pz) - pz
        local len = math.sqrt(dx * dx + dz * dz)
        local ux = len > 1e-4 and (dx / len) or 1
        local uz = len > 1e-4 and (dz / len) or 0
        viber.entity_knockback(h.id, ux, uz, 6.5)
        if viber.burst then viber.burst("smoke", h.x or px, (h.y or py) + 0.8, h.z or pz, 8) end
      end
    end
    g.voice_buff_kind = "brasa"
    g.voice_buff_until = now + 25.0
    if viber.sound then viber.sound("fire_crackle", pos) end
    if viber.burst then viber.burst("fire", px, py + 0.9, pz, 18) end
    if viber.ring then viber.ring(px, pz, 6.5, "#ff7733") end
    if is_rain then
      viber.toast("Acorde da Brasa: vapor escaldante repele em torno de ti!")
    elseif is_night then
      viber.toast("Acorde da Brasa: chama noturna acende a tua lâmina!")
    else
      viber.toast("Acorde da Brasa: chamas envolvem a tua arma (25s).")
    end
  elseif cls.chord == "geada" then
    local amp = has_snow or py > 14.0
    local dmg = amp and 14 or 8
    local kb = amp and 9.5 or 6.5
    local hits = rules.hostiles_near(px, pz, 6.5)
    for _, h in ipairs(hits) do
      rules.damage_enemy(h.id, dmg)
      rules.enemy_status(h.id, "frost", 5.5)
      if viber.entity_knockback then
        local dx = (h.x or px) - px
        local dz = (h.z or pz) - pz
        local len = math.sqrt(dx * dx + dz * dz)
        local ux = len > 1e-4 and (dx / len) or 1
        local uz = len > 1e-4 and (dz / len) or 0
        viber.entity_knockback(h.id, ux, uz, kb)
      end
    end
    rules.grant_iframes(1.8)
    g.voice_buff_kind = "geada"
    g.voice_buff_until = now + 25.0
    if viber.sound then viber.sound("block", pos) end
    if viber.burst then viber.burst("snow", px, py + 1.0, pz, 20) end
    if viber.ring then viber.ring(px, pz, 6.5, "#66ccff") end
    viber.toast("Acorde da Geada: escudo glacial firma a tua lâmina (25s).")
  elseif cls.chord == "tempestade" then
    local dmg = 10 + math.floor(wind_speed * 0.5) + (is_rain and 6 or 0)
    local hits = rules.hostiles_near(px, pz, 7.0)
    for _, h in ipairs(hits) do
      rules.damage_enemy(h.id, dmg)
      rules.enemy_status(h.id, "shock", 4.0)
    end
    g.voice_dash_reset = true
    g.voice_buff_kind = "tempestade"
    g.voice_buff_until = now + 20.0
    if viber.sound then viber.sound("thunder", pos) end
    if viber.burst then viber.burst("sparks", px, py + 1.0, pz, 20) end
    if viber.ring then viber.ring(px, pz, 7.0, "#ffe066") end
    viber.toast("Acorde da Tempestade: o relâmpago renova o teu passo!")
  elseif cls.chord == "raiz" then
    if rules.status_clear then
      rules.status_clear("venom")
    else
      g.venom = 0
      g.venom_tick = 0
    end
    local heal = math.floor(18 + wetness * 12 + (is_rain and 6 or 0) + 0.5)
    rules.heal_player(heal)
    g.voice_buff_kind = nil
    g.voice_buff_until = 0
    if viber.sound then viber.sound("heal", pos) end
    if viber.burst then viber.burst("leaves", px, py + 1.0, pz, 18) end
    if viber.ring then viber.ring(px, pz, 4.5, "#66dd88") end
    local guide = find_nearest_uncompleted_target(px, pz, g)
    if guide then
      viber.toast("Acorde da Raiz: +" .. tostring(heal) .. " vida; eco vivo a " .. guide.dir .. " (~" .. tostring(guide.dist) .. "m).")
    else
      viber.toast("Acorde da Raiz: o veneno cede e +" .. tostring(heal) .. " de vida floresce.")
    end
  elseif cls.chord == "pacto" then
    local tk = cls.target_kind or "wolf"
    local label = M.label_for_creature(tk)
    local hits = rules.hostiles_near(px, pz, 9.0)
    for _, h in ipairs(hits) do
      local hk = h.kind or ""
      if hk == tk or (rules.kind_matches and rules.kind_matches(hk, tk)) then
        rules.damage_enemy(h.id, 14)
        rules.enemy_status(h.id, "shock", 3.0)
        if viber.burst then
          viber.burst("magic", h.x or px, (h.y or py) + 0.9, h.z or pz, 10)
        end
      end
    end
    g.voice_pact_kind = tk
    g.voice_pact_until = now + 45.0
    if viber.sound then viber.sound("roar", pos) end
    if viber.burst then viber.burst("magic", px, py + 1.0, pz, 18) end
    if viber.ring then viber.ring(px, pz, 9.0, "#cc66ff") end
    viber.toast("Pacto selado contra " .. label .. " (45s)!")
  end

  return {
    ok = true,
    chord = cls.chord,
    target_kind = cls.target_kind,
    cooldown = cd_secs,
  }
end

function M.self_test()
  local c1 = M.classify("chama do sol", nil, {})
  assert(c1.chord == "brasa", "esperado brasa, obtido " .. tostring(c1.chord))

  local c2 = M.classify("escudo de gelo", nil, {})
  assert(c2.chord == "geada", "esperado geada, obtido " .. tostring(c2.chord))

  local c3 = M.classify("vento e trovão", nil, {})
  assert(c3.chord == "tempestade", "esperado tempestade, obtido " .. tostring(c3.chord))

  local c4 = M.classify("água da raiz", nil, {})
  assert(c4.chord == "raiz", "esperado raiz, obtido " .. tostring(c4.chord))

  local c5 = M.classify("caçar o lobo", nil, { "wolf" })
  assert(c5.chord == "pacto" and c5.target_kind == "wolf",
    "esperado pacto/wolf, obtido " .. tostring(c5.chord) .. "/" .. tostring(c5.target_kind))

  local c6 = M.classify("caçar o lobo")
  assert(c6.chord == "pacto" and c6.target_kind == "wolf",
    "esperado pacto/wolf com near_hostiles=nil, obtido " .. tostring(c6.chord) .. "/" .. tostring(c6.target_kind))

  return true
end

return M
