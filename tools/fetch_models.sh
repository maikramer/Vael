#!/usr/bin/env bash
# Materializa os modelos do CelerAI dentro do jogo.
#
# Os scripts do Vael chamam `viber.nlu.load("assets/public/nlu")` e a engine só
# aceita caminhos relativos sem '..' — por isso os modelos (que vivem no
# checkout irmão ../CelerAI/models) aparecem aqui como symlinks. A engine lê
# através deles sem saber.
#
# Clone novo: correr uma vez depois de clonar ../CelerAI.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
gitclones="$(dirname "$root")"

for name in nlu d1; do
    dest="$root/assets/public/$name"
    if [ -L "$dest" ] || [ -e "$dest" ]; then
        echo "ok: assets/public/$name já existe"
        continue
    fi
    src="$gitclones/CelerAI/models/$name"
    if [ ! -d "$src" ]; then
        echo "erro: $src não existe — clona o CelerAI (irmão do Vael) primeiro" >&2
        exit 1
    fi
    ln -s "../../../CelerAI/models/$name" "$dest" || cp -r "$src" "$dest"
    echo "link: assets/public/$name -> CelerAI/models/$name"
done
