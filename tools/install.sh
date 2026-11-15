#!/usr/bin/env bash
# Instala o CLI do Vael (`vael-tools`) em ~/.local/bin.
#   tools/install.sh
# Remove os restos do CLI `viber` (o clone ../viber-tools foi aposentado);
# os saves em ~/.local/share/viber NÃO são tocados.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p vael-tools
mkdir -p "$HOME/.local/bin"
ln -sfn "$PWD/target/release/vael-tools" "$HOME/.local/bin/vael-tools"
echo "vael-tools -> $HOME/.local/bin/vael-tools"
# O `jd build`/`jd run` compila em `.jackdaw/target/game`; apontá-lo ao
# `target/` partilha o `target/debug` com o `cargo build` (o mesmo perfil dev),
# em vez de manter uma segunda cópia de tudo. Só se ainda não existir lá nada;
# `-sfn` repõe um link que ficou a apontar para um `target/` já apagado.
if [ ! -e .jackdaw/target/game ]; then
  mkdir -p .jackdaw/target
  ln -sfn ../../target .jackdaw/target/game
  echo ".jackdaw/target/game -> target/ (build do jd partilhado)"
fi
for stale in "$HOME/.local/bin/viber" "$HOME/.local/bin/viber-tools" "$HOME/.local/bin/viber-engine"; do
  if [ -e "$stale" ] || [ -L "$stale" ]; then
    rm -f "$stale"
    echo "removido: $stale"
  fi
done
