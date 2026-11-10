-- lib/i18n.lua: tradução das frases que os SCRIPTS mostram ao jogador.
--
-- A engine traduz as chaves `@nome` da UI declarativa (`locale/<lang>.json`).
-- As frases montadas em Lua (toasts, prompts, balões, números flutuantes)
-- eram PT fixo em ~180 sítios. Em vez de tocar em todos, este módulo embrulha
-- UMA vez as funções de saída (`viber.toast`, `say`, `set_interaction`,
-- `prompt`, `damage_number`): a frase PT é a chave — estilo gettext.
--
-- `locale/strings_<lang>.json`:
--   { "exact":    { "frase PT": "tradução" },
--     "patterns": [ ["^padrão Lua com (capturas)$", "template com %1"] ] }
--
-- Em PT (ou sem ficheiro para o idioma) a frase passa intacta. O que não
-- tiver tradução também passa em PT — `tools/i18n_extract.py` lista o que falta.
-- A VM é partilhada por todos os scripts, por isso instalar uma vez chega
-- (`lib/rules.lua` instala no arranque).

local M = {}

local tables = {}   -- lang → { exact, patterns } | false
local memo = {}     -- lang → { frase → tradução }
local memo_n = 0

local function lang()
  local chosen = viber.game().lang
  if type(chosen) == "string" and chosen ~= "" then
    return chosen
  end
  if viber.locale and viber.locale.get then
    return viber.locale.get() or "pt"
  end
  return "pt"
end

local function table_for(code)
  local t = tables[code]
  if t ~= nil then
    return t
  end
  t = false
  if code ~= "pt" then
    local ok, data = pcall(viber.load_json, "locale/strings_" .. code .. ".json")
    if ok and type(data) == "table" then
      t = {
        exact = type(data.exact) == "table" and data.exact or {},
        patterns = type(data.patterns) == "table" and data.patterns or {},
      }
    end
  end
  tables[code] = t
  return t
end

local function tr_line(t, line)
  local hit = t.exact[line]
  if hit then
    return hit
  end
  for _, pair in ipairs(t.patterns) do
    local pattern, template = pair[1], pair[2]
    if type(pattern) == "string" and type(template) == "string" then
      local caps = { line:match(pattern) }
      if #caps > 0 then
        -- `%1`..`%9` no template são as capturas; a própria captura passa
        -- por `exact` (nomes de itens, estados) antes de entrar.
        return (template:gsub("%%(%d)", function(d)
          local cap = caps[tonumber(d)]
          if type(cap) ~= "string" then
            return ""
          end
          return t.exact[cap] or cap
        end))
      end
    end
  end
  return line
end

function M.tr(text)
  if type(text) ~= "string" or text == "" then
    return text
  end
  local code = lang()
  local t = table_for(code)
  if not t then
    return text
  end
  local cache = memo[code]
  if not cache then
    cache = {}
    memo[code] = cache
  end
  local done = cache[text]
  if done then
    return done
  end
  local out
  if text:find("\n", 1, true) then
    local lines = {}
    for line in (text .. "\n"):gmatch("(.-)\n") do
      lines[#lines + 1] = tr_line(t, line)
    end
    out = table.concat(lines, "\n")
  else
    out = tr_line(t, text)
  end
  -- Os números flutuantes geram frases sempre novas: o cache tem teto.
  if memo_n > 2000 then
    memo, memo_n = { [code] = {} }, 0
    cache = memo[code]
  end
  cache[text] = out
  memo_n = memo_n + 1
  return out
end

-- Embrulha as funções de saída. Os embrulhos chamam `viber.__i18n_tr`, que
-- um hot-reload deste módulo substitui — nunca se embrulha duas vezes.
function M.install()
  viber.__i18n_tr = M.tr
  if viber.__i18n_installed then
    return
  end
  viber.__i18n_installed = true
  local function wrap_first(name)
    local orig = viber[name]
    if type(orig) ~= "function" then
      return
    end
    viber[name] = function(text, ...)
      return orig(viber.__i18n_tr(text), ...)
    end
  end
  wrap_first("toast")
  wrap_first("say")
  wrap_first("set_interaction")
  wrap_first("damage_number")
  local prompt = viber.prompt
  if type(prompt) == "function" then
    viber.prompt = function(key, label, ...)
      return prompt(key, viber.__i18n_tr(label), ...)
    end
  end
end

return M
