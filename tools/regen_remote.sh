#!/usr/bin/env bash
# Regenera um pack no servidor de GPU (vramd remoto): GPU lá, CPU aqui.
#
#   tools/regen_remote.sh <pack> [args do resume…]
#   tools/regen_remote.sh characters
#   tools/regen_remote.sh audio-sfx-ui --force
#
# Env: VRAMD_REMOTE_ENV (default ~/.config/vramd/remote.env) — escrito por
# `AiGameKit/scripts/docker/server-deploy.sh token`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${VRAMD_REMOTE_ENV:-$HOME/.config/vramd/remote.env}"

if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    . "$ENV_FILE"
fi

if [ -z "${VRAMD_REMOTE:-}" ]; then
    echo "ERRO: VRAMD_REMOTE não definido." >&2
    echo "  Corre (em AiGameKit): scripts/docker/server-deploy.sh token" >&2
    echo "  ou define VRAMD_REMOTE=host:port e VRAMD_REMOTE_TOKEN." >&2
    exit 1
fi

PACK="${1:?uso: tools/regen_remote.sh <pack> [args do resume…]}"
shift || true

echo "vramd remoto: $VRAMD_REMOTE"
cd "$ROOT/assets"
exec gameassets resume --profile game.yaml --manifest "manifests/$PACK" --no-dashboard "$@"
