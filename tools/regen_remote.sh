#!/usr/bin/env bash
# Regenera um pack no servidor de GPU (vramd remoto): GPU lá, CPU aqui.
#
#   tools/regen_remote.sh <pack> [args do resume…]
#   tools/regen_remote.sh characters
#   tools/regen_remote.sh audio-sfx-ui --force
#   VRAMD_GPU_MODE=auto tools/regen_remote.sh characters   # respeita o gpu: do manifesto
#
# O código do pipeline é SEMPRE o desta máquina: o `gameassets resume` gera um
# bundle content-addressed (Shared/src + <Tool>/src + locks) e o vramd remoto
# abre uma sessão no primeiro job de cada backend/wave. Não há `sync-repo` nem
# código instalado no servidor; o bundle só é reenviado quando o hash muda. O
# servidor fornece GPU, cache de modelos e envs cacheadas por lock.
#
# Env: VRAMD_REMOTE_ENV (default ~/.config/vramd/remote.env) — escrito por
# `vramd config` no servidor + `VRAMD_REMOTE`/`VRAMD_REMOTE_TOKEN` no cliente.
#      VRAMD_GPU_MODE=remote|local|auto (default remote) — default dos assets
#      com `gpu: auto`; um `gpu:` explícito no manifesto ganha.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${VRAMD_REMOTE_ENV:-$HOME/.config/vramd/remote.env}"

if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    . "$ENV_FILE"
fi

if [ -z "${VRAMD_REMOTE:-}" ]; then
    echo "ERRO: VRAMD_REMOTE não definido." >&2
    echo "  No servidor: vramd config  (escreve o token e imprime o comando)" >&2
    echo "  Aqui: define VRAMD_REMOTE=host:8790 e VRAMD_REMOTE_TOKEN." >&2
    echo "  Exemplo: source ~/.config/vramd/remote.env" >&2
    exit 1
fi

# Compatibilidade: o antigo --no-sync deixou de fazer sentido (o código viaja
# no bundle da sessão). Aceita e ignora para não quebrar scripts.
if [ "${1:-}" = "--no-sync" ]; then
    shift
fi

PACK="${1:?uso: tools/regen_remote.sh [--no-sync] <pack> [args do resume…]}"
shift || true

GPU="${VRAMD_GPU_MODE:-remote}"
echo "vramd remoto: $VRAMD_REMOTE (gpu default: $GPU)"

cd "$ROOT/assets"
exec gameassets resume --profile game.yaml --manifest "manifests/$PACK" \
    --no-dashboard --gpu "$GPU" "$@"
