# GPU remota (vramd na rede)

O `vramd` pode correr num servidor e ser usado pela rede: o laptop envia
**código + dados**, o servidor gera nas GPUs e devolve as saídas. O
orquestrador e as fases CPU ficam no laptop.

Servidor: `root@192.168.0.201` (2× RTX 3060 12 GB). A máquina tem **só**:

- driver NVIDIA + CUDA runtime;
- `vramd` (pip) e o `uv` (para construir envs);
- o cache de modelos Hugging Face normal (`~/.cache/huggingface`);
- os caches do vramd (bundles, envs, sandboxes, blobs).

Não há Docker, não há checkout do AiGameKit, não há venvs do pipeline
instalados à mão. O código viaja num bundle content-addressed no início de
cada batch.

## Setup do servidor (uma vez)

```bash
# no servidor (vramd >= 0.7.4; usa o Python do SISTEMA no venv)
python3 -m venv /opt/vramd
/opt/vramd/bin/pip install -U vramd
# uv constrói as envs das tools (o serviço corre como o utilizador do serviço)
curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh
/opt/vramd/bin/vramd config
```

> Não cries o venv com o Python gerido pelo `uv` enquanto root: fica em
> `/root/.local/share/uv/python`, que o utilizador do serviço não consegue
> executar (systemd `203/EXEC`). O `python3 -m venv` do sistema evita isso; o
> `uv` continua a servir para as envs das tools.

O wizard pergunta listener, cache, `HF_HOME`, política de modelos, GPUs,
permissões de sessões e serviço systemd. Ele:

- escreve `/etc/vramd/config.yml` (ou `~/.config/vramd/config.yml` sem root);
- cria o token 0600 em `/etc/vramd/token`;
- detecta `/srv/vramd-cache/models/hf` e **move** o cache para
  `~/.cache/huggingface` (rename no mesmo filesystem ou cópia com verificação;
  sem re-download);
- cria a unidade systemd e, como root, faz `enable --now vramd`.

Alternativa não interactiva (o que está em produção):

```bash
/opt/vramd/bin/vramd config --non-interactive \
  --listen 0.0.0.0:8790 \
  --cache-dir /var/cache/vramd \
  --token-file /etc/vramd/token \
  --hf-home /home/maikeu/.cache/huggingface \
  --models-policy auto \
  --gpus auto \
  --allow-client-code --allow-client-env \
  --service-user maikeu \
  --move-models --install-service -y
```

Abre a 8790 só para a LAN (o Docker antigo contornava o UFW; o vramd não):

```bash
ufw allow from 192.168.0.0/24 to any port 8790 proto tcp comment 'vramd LAN'
```

O `config` dá ao utilizador do serviço acesso ao token, ao cache e ao
`HF_HOME` (chown como root). Confirma:

```bash
vramd status
vramd devices           # GPUs locais + peers remotos
vramd models list       # cache HF (o antigo /srv/... deve aparecer)
vramd env list
```

> O `vramd` fala por TCP com token (sem TLS). Usa WireGuard/VPN ou uma LAN de
> confiança; o token é credencial de administração do daemon.

## Cliente

```bash
# ~/.config/vramd/remote.env (0600)
export VRAMD_REMOTE=192.168.0.201:8790
export VRAMD_REMOTE_TOKEN=…        # cat /etc/vramd/token no servidor
export VRAMD_AUTO_START=0
```

```bash
source ~/.config/vramd/remote.env
vramd status                       # fila/backends/sessões do servidor
cd assets
gameassets resume --profile game.yaml --manifest manifests/<pack> --gpu remote
```

ou, do Vael:

```bash
tools/regen_remote.sh characters
tools/regen_remote.sh audio-sfx-ui --force
VRAMD_GPU_MODE=auto tools/regen_remote.sh characters   # respeita o gpu: do manifesto
```

## Código: o cliente é a fonte de verdade

No primeiro job remoto de cada backend/wave, o `aigamekit_shared.vramd_bundle`:

1. junta `Shared/src` e `<Tool>/src` (sem venvs, testes nem pesos);
2. gera um freeze exacto por tool a partir do venv local (`uv pip freeze`);
3. monta `session/envs/<tool>.lock` e calcula o hash do bundle;
4. envia o bundle como blob (dedupe: se o servidor já o tem, não sobe nada);
5. abre `session-open` com o manifesto: ficheiros, envs, backends e modelos.

O servidor extrai o bundle para `<cache>/sessions/bundles/<sha>` **verificando
cada ficheiro contra o manifesto** (paths relativos, sem symlinks, tamanho e
sha256) e arranca o worker com:

```text
PYTHONPATH=<bundle>/Shared/src:<bundle>/<Tool>/src
cwd=<bundle>
<env>/bin/python -m <tool> serve --ums-worker
```

Cada job leva `session` e `backend`; o resultado inclui `session` e
`bundle_sha`, por isso o log prova qual código correu. O código é **pinado por
batch**: alterações a meio entram no batch seguinte (reabrir a sessão), o que
torna os resultados reproduzíveis.

- **Mudou código Python** → o próximo batch calcula outro hash e envia o
  bundle; nada a fazer no servidor.
- **Mudaram dependências** → o lock muda, o vramd constrói uma env nova
  (`<cache>/envs/<hash>`) com `uv` e reutiliza-a nas próximas vezes.
- **Servidor reiniciou** → perde as sessões em memória; o cliente detecta
  `SESSION_UNKNOWN`, reabre e repete o job uma vez (o bundle está no CAS).

## Modelos

O vramd usa o cache **normal** do Hugging Face (`HF_HOME`). Os manifests
declaram os modelos por backend; antes do primeiro load o vramd verifica o
cache e, conforme `models.policy`:

| política | comportamento |
|---|---|
| `auto` (default) | usa o cache; se faltar, tenta o Hub; sem rede falha com hint |
| `hub` | exige rede para descarregar o que faltar |
| `off` | só cache local (servidor offline) |

```bash
vramd models list                 # repos/revisions/tamanho, pins das sessões
vramd models ensure tencent/Hunyuan3D-Omni
vramd models gc                  # LRU até models.max_gib, protegendo pins
```

O primeiro setup move o cache antigo `/srv/vramd-cache/models/hf`; não há
re-download dos
~60 GB. Um modelo gated (`stable-audio-3`) precisa de `HF_TOKEN`: o wizard
escreve `/etc/vramd/hf-token` (0600) ou exporta `HF_TOKEN`.

## Ambientes (deps)

As deps pesadas (torch/diffusers/bpy) não viajam no bundle: o cliente envia o
freeze e o vramd constrói/reutiliza `<cache>/envs/<hash>` com `uv`. A primeira
construção de uma env é a parte lenta (GBs); mudanças de código não a repetem.
As envs ficam visíveis em `vramd env list`; para pré-aquecer:

```bash
vramd env build /caminho/para/session/envs/text3d.lock --python 3.13
```

O build precisa de rede (PyPI/PyTorch) ou de um cache de wheels. O host precisa
de `nvcc`/CUDA toolkit para extensões que compilam (ex.: `nvdiffrast`).

## Throughput

Medido localmente (`vramd/scripts/bench_transfer.py --size-mib 32`, dados
incompressíveis): o caminho antigo (JSON+base64, uma ligação por chunk de
2 MiB) faz ~64 MiB/s; o novo (frames + uma ligação + chunka de 8 MiB) faz
~283 MiB/s de upload e ~350 MiB/s de download, ~4,4× no upload. Numa LAN
gigabit o limite passa a ser a rede (~118 MiB/s); a vantagem principal é não
gastar CPU com base64 e não reenviar blobs que o servidor já tem.

## O que corre onde

| Fase | Onde |
|---|---|
| `text2d` / `texture2d` / `skymap2d` | servidor (GPU) |
| `text3d generate` (inclui o text2d interno) | servidor (GPU) |
| `text3d refine` (Hi3D) | **laptop (GPU)** — exceção cu124; não entra nas sessões |
| `paint3d texture` | servidor (GPU) |
| `text2sound generate` | servidor (GPU) |
| `text3d topology-fix` / `patch-pbr` | laptop (CPU) |
| `text3d lod` / `finish` (`tools/make_lods.py`) | laptop (CPU) |
| `animator3d game-pack` (bpy) | laptop (CPU) |
| `rigging3d pipeline` (SkinTokens) | **laptop (GPU 6 GB)** — ainda não fala com o vramd |
| colisão/validação/LOD/collision do `resume` | laptop (CPU) |

Todo o pipeline remoto é **CUDA 13** (`torch 2.13.0+cu130`,
`nvidia-*-cu13`); os locks de sessão descartam pacotes cu12. O **Hi3D** é a
única exceção: torch 2.5.0+**cu124** (Python 3.10, spconv-cu120/flash-attn),
sem build cu13; fica local e o `resume` força `--gpu local` no refine.

## Escolher local vs remoto por asset

Cada asset do manifesto aceita:

```yaml
assets:
  - id: witch_hut
    idea: "casa de bruxa inclinada"
    pipeline: [3d, paint, collision]
    gpu: remote        # remote | local | auto (default)
```

- `remote`: força o servidor; sem `VRAMD_REMOTE` o job falha alto.
- `local`: ignora `VRAMD_REMOTE` e usa a GPU desta máquina.
- `auto`: usa `VRAMD_GPU_MODE`/`VRAMD_AUTO_GPU`; sem esses, remote se
  `VRAMD_REMOTE` estiver definido, senão local.

Nas tools e no batcher há `--gpu auto|local|remote` (também
`VRAMD_GPU_MODE=...`); um `gpu:` explícito do asset ganha ao default. Waves
mistas submetem cada asset ao vramd certo (local por Unix socket, remoto por
TCP) e esperam no mesmo modo.

`vramd devices` lista as GPUs locais (lanes) e os peers remotos (RTT, banda,
fila, modelos quentes). `vramd route text3d --prompt x --output /tmp/x.glb`
mostra o custo de cada rota sem submeter.

## Operação

```bash
source ~/.config/vramd/remote.env
vramd status
vramd queue
vramd sessions list
vramd sessions close <id>
vramd models list
vramd devices
vramd zero                  # larga a VRAM (recusa com jobs na fila)
```

Sandboxes: `<cache>/remote/<job>/`; TTL de 24 h; `VRAMD_KEEP_SANDBOX=1`
mantém para inspecção. Blobs: `<cache>/blobs/`; TTL/GC em
`VRAMD_BLOB_*`. Bundles: `<cache>/sessions/bundles/<sha>/`; envs:
`<cache>/envs/<hash>/`.

## Problemas comuns

- **`VRAMD_REMOTE_DOWN`** — o daemon não respondeu. `vramd status`; no
  servidor, `systemctl status vramd` e `journalctl -u vramd -n 100`.
  `VRAMD_REMOTE_STRICT=0` reactiva o fallback local (não recomendado).
- **`SESSION_DISABLED`** — falta `--allow-client-code` no servidor
  (`vramd config` → allow_client_code) ou `VRAMD_SESSION=off` no cliente.
- **`SESSION_ENV_DISABLED`** — a env ainda não existe e o servidor está sem
  `--allow-client-env`; corre `vramd env build` ou liga a flag.
- **Build de env falha** — vê `<cache>/envs/<hash>.build.log`; normalmente é
  rede/wheel ou `uv` em falta.
- **Modelo em falta / offline** — `vramd models ensure` com rede, ou move o
  cache; `models.policy=off` para offline estrito.
- **Cliente num venv sem `vramd`** — instala `aigamekit-shared` (depende de
  `vramd>=0.7`) ou define `VRAMD_REMOTE_PROTOCOL=json` para o caminho antigo.
- **Voltar ao local** — não faças `source` do `remote.env` (ou `unset
  VRAMD_REMOTE VRAMD_REMOTE_TOKEN`), ou usa `--gpu local` /
  `gpu: local` no asset.

## Docker

O deploy antigo por Docker foi **removido**: o servidor não tem Docker nem
checkout do AiGameKit. Em `AiGameKit/scripts/docker/` resta só o teste de QA de
máquina limpa (container), que não é o runtime do servidor.
