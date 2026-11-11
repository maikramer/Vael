# GPU remota (vramd na rede)

O `vramd` 0.6 pode correr num servidor e ser usado pela rede: o laptop envia os
dados (prompts, imagens, meshes), o servidor gera nas GPUs e devolve as saídas.
O **orquestrador e as fases CPU ficam no laptop**.

Servidor: `root@192.168.0.100` (2× RTX 3060 12 GB), cache em
`/srv/vramd-cache` (modelos, sandboxes, logs). O daemon corre num container
(`AiGameKit/scripts/docker/`), porta `8790`, token em `/srv/vramd-cache/token`.

## O que corre onde

| Fase | Onde |
|---|---|
| `text2d` / `texture2d` / `skymap2d` | servidor (GPU) |
| `text3d generate` (inclui o text2d interno) | servidor (GPU) |
| `text3d refine` (Hi3D) | servidor (GPU) |
| `paint3d texture` | servidor (GPU) |
| `text2sound generate` | servidor (GPU) |
| `text3d topology-fix` / `patch-pbr` | laptop (CPU) |
| `text3d lod` / `finish` (`tools/make_lods.py`) | laptop (CPU) |
| `animator3d game-pack` (bpy) | laptop (CPU) |
| `rigging3d pipeline` (SkinTokens) | **laptop (GPU 6 GB)** — não fala com o vramd |
| colisão/validação/LOD/collision do `resume` | laptop (CPU) |

Exceção conhecida: o `rigging3d` continua a usar a GPU local. É o único passo GPU
fora do servidor nesta versão.

## Setup (uma vez)

No checkout `AiGameKit`:

```bash
scripts/docker/server-deploy.sh preflight
scripts/docker/server-deploy.sh install-gpu-runtime   # nvidia-container-toolkit
scripts/docker/server-deploy.sh sync-models           # ~98G (uma vez)
scripts/docker/server-deploy.sh sync-repo
scripts/docker/server-deploy.sh build                 # imagem longa
scripts/docker/server-deploy.sh up
scripts/docker/server-deploy.sh token                 # escreve ~/.config/vramd/remote.env
```

O `token` grava `~/.config/vramd/remote.env` (0600):

```sh
export VRAMD_REMOTE=192.168.0.100:8790
export VRAMD_REMOTE_TOKEN=…
export VRAMD_AUTO_START=0
```

> Se a internet do servidor for fraca, constrói a imagem nesta máquina e
> envia-a por LAN comprimida: `server-deploy.sh build-local`, depois
> `push-image` e `up-image` (a imagem é a mesma; o `up` normal tentaria
> reconstruir no servidor).

## Usar

```bash
source ~/.config/vramd/remote.env
vramd status                 # fila/backends do servidor
cd assets
gameassets resume --profile game.yaml --manifest manifests/<pack>
```

ou, do Vael:

```bash
tools/regen_remote.sh characters
tools/regen_remote.sh audio-sfx-ui --force   # args extra vão para o resume
```

O staging é automático e transparente: para cada job GPU, o cliente faz
`stage` → upload dos inputs → `generate` com paths do sandbox → `wait` →
download dos outputs (+ sidecars: `*.omni.json`, `<stem>_1.png`, …) → `clean`.
Os paths locais (do manifest) são reescritos de volta, por isso o `resume`
continua a saltar o que já existe e as fases CPU seguintes veem os ficheiros.

`assets/regen_audio.py` e `tools/make_lods.py` não mudam: o primeiro recebe o
OGG de volta no path local (o gate de cauda/início mede-o como sempre); o
segundo é CPU e já corria aqui.

## Operação

```bash
source ~/.config/vramd/remote.env
vramd status                 # GPUs, calibração, fila
vramd queue                  # jobs
vramd logs text2d -f         # stderr do worker
vramd zero                   # larga a VRAM (recusa com jobs na fila)
vramd respawn text2d         # recicla o worker (código novo no servidor)
```

Sandboxes: `/srv/vramd-cache/remote/<job>/`; TTL de 24 h; `VRAMD_KEEP_SANDBOX=1`
mantém o sandbox para inspeção/retry. O download é atómico (`*.part` →
rename): um job falhado não deixa ficheiros a meio.

## Problemas comuns

- **`VRAMD_REMOTE_DOWN`** — o daemon não respondeu. O fallback in-process está
  **desligado** em modo remoto (senão corria na GPU errada). Verifica
  `vramd status`; no servidor, `docker compose -f scripts/docker/docker-compose.server.yml ps`
  e `logs`. `VRAMD_REMOTE_STRICT=0` reativa o fallback local (não recomendado).
- **Modelo em falta / offline** — o container corre com `HF_HUB_OFFLINE=1`;
  volta a correr `sync-models` (o cache é `/srv/vramd-cache/models`).
- **Primeiro job de um backend** — o worker carrega os pesos (30–60 s) e depois
  fica quente; `vramd preload <backend>` pré-aquece.
- **Voltar ao local** — não faças `source` do `remote.env` (ou `unset
  VRAMD_REMOTE VRAMD_REMOTE_TOKEN`). Sem `VRAMD_REMOTE`, tudo corre no laptop.
- **Calibração** — o servidor usa `backends-6g.yaml` (a única calibração
  medida; conservadora em 12 GB). `vramd calibrate <backend> --out` numa 3060
  produz `backends-12g.yaml` para acrescentar ao repo.
