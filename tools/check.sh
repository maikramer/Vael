#!/usr/bin/env bash
# Gate local do Vael: corre tudo o que valida o jogo sem abrir a janela.
#   tools/check.sh            # a partir de qualquer pasta
# Sai com 1 se algum passo OBRIGATÓRIO falhar. O binário da engine não é
# recompilado (VIBER_NO_BUILD=1): isto valida o jogo, não a engine.
set -u
cd "$(dirname "$0")/.."
export VIBER_NO_BUILD=1
fail=0
step() { printf '\n== %s ==\n' "$1"; }

step "conteúdo (JSON + referências)"
python3 tools/lint_content.py || fail=1

step "sintaxe Lua"
if python3 -c 'import luaparser' 2>/dev/null; then
  # townsfolk.lua usa sintaxe Luau que o luaparser não lê: fica de fora.
  python3 - <<'PY' || fail=1
import glob, sys
from luaparser import ast
SKIP = {'scripts/townsfolk.lua'}
bad = 0
for path in sorted(glob.glob('scripts/**/*.lua', recursive=True)):
    if path in SKIP:
        continue
    try:
        ast.parse(open(path, encoding='utf-8').read())
    except Exception as e:  # noqa: BLE001
        bad += 1
        print('ERRO:', path, str(e)[:200])
print(f"lua: {bad} ficheiro(s) com erro de sintaxe")
sys.exit(1 if bad else 0)
PY
else
  echo "saltado: pip install luaparser (ou cargo test --test script_syntax em ../Viber)"
fi

step "viber analyze"
if command -v viber >/dev/null; then
  viber analyze world.xml 2>&1 | grep -v '^warning: <Road>' | tail -12
  [ "${PIPESTATUS[0]}" -eq 0 ] || fail=1
  step "viber scene lint"
  viber scene lint 2>&1 | tail -8
  [ "${PIPESTATUS[0]}" -eq 0 ] || fail=1
else
  echo "saltado: viber não está no PATH"
fi

step "traduções em falta (informativo)"
python3 tools/i18n_extract.py | tail -6 || true

printf '\n%s\n' "$([ $fail -eq 0 ] && echo 'OK' || echo 'FALHOU')"
exit $fail
