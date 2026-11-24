# Playtest: `bevy_playtest` + oráculo visual `d1-ultra`

Duas camadas novas por cima do QA de cenários ([`QA.md`](QA.md)):

| Camada | O que faz | Onde |
|---|---|---|
| Bateria in-process | bot chaos/guiado, oráculos do `bevy_playtest` + invariantes do Vael, várias seeds e soak | `tests/playtest_scenarios.rs`, `playtests/*.json` |
| Oráculo visual+estado | captura frames a ~10 Hz do jogo real e pergunta ao `liquid/d1-ultra` se a cena está conforme | `tools/playtest_vision.py`, `qa/vision/tours/*.json` |

## Porque não a bateria por subprocesso

O takeover do `bevy_playtest` arranca o jogo e faz **fast-forward** de
`duration_s × tps` frames; não espera por assets assíncronos. No Vael (GLB por
distância, terreno, véu) o cenário podia acabar antes de o mundo estar pronto.
O harness aqui constrói a App completa, boota até `WorldReadiness.ready`,
deixa o cenário QA assentar (120 frames) e só então chama
`bevy_playtest::run_scenario`. A App é construída pelo mesmo
`vael::app::build_game_app` que o `main` usa.

O takeover continua disponível como smoke manual:

```bash
BEVY_PLAYTEST_SCENARIO=playtests/menu.json cargo run --features playtest -- --resolution 640x360
```

## Bateria

```bash
# gate rápido (cenas QA; uma App por cenário)
cargo test --features playtest --test playtest_scenarios -- --ignored --test-threads=1 --nocapture

# só um cenário / uma seed
PLAYTEST_ONLY=menu PLAYTEST_SEEDS=42 \
  cargo test --features playtest --test playtest_scenarios -- --ignored --test-threads=1 --nocapture

# mundo principal / interiores (mais lento)
PLAYTEST_ONLY=main_landmark ...   # teste `playtest_scenarios_world`

# soak
cargo test --features playtest --test playtest_scenarios -- --ignored playtest_soak --test-threads=1
```

- A feature `playtest` é ativada pelo self dev-dependency do `Cargo.toml`; o
  `cargo build`/release normal fica sem `bevy_playtest` (`cargo tree -e normal`).
- **Um processo filho por cenário**: o `run_scenario` tem um run buffer global
  por processo (o segundo no mesmo processo é rejeitado antes de gerar
  relatório). O teste pai lança o próprio binário com
  `--exact playtest_child`; isso isola também GPU, subscriber de logs e saves.
- Filtros: `PLAYTEST_ONLY=nome1,nome2` e `PLAYTEST_SEEDS=42,7,1337`
  (default da bateria rápida; o main world e o soak usam só `42`).
  `PLAYTEST_SOAK_SECONDS=90` encurta o soak para o gate local (default 600).
- Relatórios: `target/playtest-reports/<cenário>-seed<N>.json`.
- **GPU obrigatória** (o takeover remove a janela, não o `RenderPlugin`). Os
  testes estão `#[ignore]` para não partirem um `cargo test` sem GPU.
- `VAEL_QA_SAVE_DIR` é uma tempdir por corrida; o `$TMPDIR/vael-qa-saves` do
  arranque normal não é partilhado entre cenários.
- O `LogPlugin` do Vael é omitido quando `BEVY_PLAYTEST_SCENARIO` existe para o
  harness instalar o seu subscriber; sem isso os `ERROR` dos invariantes não
  contavam como violações.

### Cenários

| JSON | QA state | Cobertura | Scope |
|---|---|---|---|
| `playtests/menu.json` | start-scene | título/menu, UI | fast |
| `playtests/sandbox_chaos.json` | `sandbox` | movimento, HUD, chão | fast |
| `playtests/arena_chaos.json` | `combat/arena` | combate, criaturas | fast |
| `playtests/arena_scripted.json` | `combat/arena` | teclas guiadas (W/Space/J/C/R/B/1) | fast |
| `playtests/interior_forge.json` | `interiors/forge` | interior + porta | fast |
| `playtests/main_landmark.json` | `world/forge_stage2` | main world/streaming | world |
| `playtests/soak_sandbox.json` | `sandbox` | 10 min de chaos | soak |

O QA state novo `qa/scenarios/interiors/forge.json` põe o herói ao pé do
portal `exit` (nome do `ScenePortal` em `scenes/interiors/forge.bsn`).

### Invariantes do Vael

`src/playtest_support.rs` corre só durante um cenário, depois do `ready`, e
loga `ERROR` com target `vael_playtest` (o log sink do harness transforma-o em
violação): herói único, posição finita/dentro do mapa, `PlayerVitals`
(`0 <= hp <= max_hp`), `XpState`, `Inventory` (contagens não negativas) e
`QuestLog` (estados conhecidos, `tracked` existente). O throttle é 5 s.

No teste, `nodes_in_bounds` fica **fixado** (o default ±1000 é mais pequeno que
o mundo autorado) e `log_warn` é ignorado quando `strict_warnings` é falso;
`finite_transforms`, `frozen_world`, `panic`, `log_error`, `log_capture_lost` e
os invariantes do Vael continuam a falhar. O pin está em
`tests/playtest_scenarios.rs`; um futuro extent configurável no crate permite
voltar a apertar.

O par `log_error`/`landmass_rerecast` esteve fixado até 2026-11-23 (bug
conhecido do navmesh: `The polygon at index 2139 is concave…` e, mais tarde,
`The edge made from vertices X and Y is used by more than two polygons`). Este
último fechou com o patch #8 do `rerecast` vendor (`VIBER_PATCH.md`: uma aresta
ficava com três donos quando duas regiões simplificavam de forma diferente a
parede que partilham), reproduzido com o tile real e agora coberto por testes.
Entretanto o `bevy_ext_nav` converte e valida o tile antes de o entregar ao
landmass — o plugin `landmass_rerecast` já não corre no jogo, por isso esse alvo
não volta a logar. Hoje, um tile que o landmass recuse é
aviso (`bevy_ext_nav`, o tile é re-assado com a grelha deslocada) e só loga
`ERROR` quando a recusa se repete (`NAV_REJECTIONS_BEFORE_ERROR`); nenhum
`log_error` está fixado.

## Oráculo visual

```bash
# sem custo: captura + request.json, sem chamar a API
python3 tools/playtest_vision.py --scenario combat/arena --duration 60 --dry-run

# valida chave/modelo/endpoint (pedido mínimo)
python3 tools/playtest_vision.py --probe

# corrida real com orçamento
python3 tools/playtest_vision.py --scenario combat/arena --duration 90 --budget-usd 0.20

# tour com passos e baseline
python3 tools/playtest_vision.py --tour qa/vision/tours/forge_interior.json --duration 90
```

- Lança o jogo real a 480x270 (default; `--resolution` muda) com
  `--scenario <QA>` e `--bridge <porta>` (ou usa `--attach <porta>` para
  observar um jogo já aberto). O jogo é lançado com `VIBER_FPS_CAP=0`: o
  pacer adaptativo reagia ao custo do próprio screenshot e travava o render
  num divisor baixo. `VAEL_QA_SAVE_DIR` aponta para os artefactos da corrida:
  nem o menu sem cenário toca nos saves reais.
- **Captura**: `brp_extras/screenshot` (uma chamada, ~40 ms a 480x270; medido
  ~9,1–9,6 Hz nas cenas QA leves a 10 Hz alvo). O PNG é lido e apagado do
  `/tmp` logo a seguir. No **main world** com streaming/GLB o render é o teto:
  a taxa medida caiu para ~1,7 Hz (o `summary.json` diz sempre a taxa real e o
  warmup). Se os extras estiverem desligados (`VAEL_BRP_EXTRAS=0`), cai para
  `viber.screenshot`+status (mais lento, ~3–7 Hz).
- **Decisão**: `--decision-hz` (default 1.5, máximo 2) envia os últimos
  `--frames-per-request` frames (default 8; máximo 8 do Liquid) + texto de
  estado (`qa_snapshot`, posição, cena). Sem fila: se um pedido demora,
  salta a vez seguinte. O estado é amostrado a 5 s.
- **Perguntas**: `visual_ok` (noul), `estado_ok` (noul), `anomalia` (choice:
  none/black_screen/missing_texture/geometry_explosion/t_pose/floating_object/
  ui_broken/camera_clipping/other) e `severidade` (score: cosmética/
  jogabilidade/bloqueante).
- **Suspeita** se `visual_ok < 0.5` **ou** `estado_ok < 0.5` **ou** anomalia
  ≠ none com confiança ≥ `--min-confidence` (0.6). Resposta em falta ou
  `refusal` conta como suspeita, nunca como conforme.
- **Artefactos** em `qa/playtest-artifacts/<UTC>-<tour>/` (ignorado no git):
  frames, `request.json` (sem base64/chave), `verdict.json`, `state.json`,
  `logs.json`, `game.log`, `report.md`, `verdicts.jsonl` e `summary.json`.
- Limites do `liquid/d1-ultra`: até 8 imagens, body ≤4,5 MB, 10 000 patches
  32×32, contexto 32K, $0,042/M input. O tool corta frames se o body passar de
  `--max-body-mb` (default 4,0). Custo medido: ~$0,00017/pedido com 8 frames a
  480×270 ⇒ ~$0,9/h a 1,5 Hz.
- `--pause-on-anomaly` pausa o jogo para inspeção; `--stop-on-anomaly` pára a
  corrida; `--fail-on-anomaly` devolve exit 1. `--keep` deixa o jogo aberto.

### Tours

`qa/vision/tours/*.json`: `{name, qa_scenario, context, baseline, steps}`.
Passos usam a sintaxe do `qa.py` (`key`, `wait`, `goto`, `teleport`,
`interact`, `say`, `screenshot`) e um passo novo `scene`
(`viber.scene.change` + espera pelo `ready`). `baseline: true` envia 1 frame
de referência + 7 atuais (cenas estáticas). Tours iniciais: `menu`,
`sandbox_hud`, `arena_combat`, `forge_interior`, `main_landmark`,
`portal_roundtrip`.

**Cuidado com `scene`**: carregar `main` a partir de um interior pelo
`viber.scene.change` pode bater no cap de 240 s do loader (a mesma *gotcha* de
[`TOOLS.md`](TOOLS.md): `main`→interior→`main` via portal funciona, recarregar
`main` directamente pode não). Para o roundtrip real, usa o portal do cenário
(`auto-radius` 1.6) ou arranca a cena `main` directamente (`main_landmark`).

## Chave e custo

`secrets.env` na raiz (ignorado no git, `chmod 600`; modelo em
`secrets.env.example`):

```dotenv
OPENROUTER_API_KEY=sk-or-v1-...
OPENROUTER_DECISION_MODEL=liquid/d1-ultra
```

O tool lê primeiro `OPENROUTER_API_KEY` do ambiente; `VAEL_SECRETS_FILE` aponta
outro ficheiro. Nunca imprime a chave. O endpoint é
`https://openrouter.ai/api/alpha/decisions` (`OPENROUTER_DECISIONS_URL` para
override). A chave usada no arranque desta integração foi partilhada no chat:
rodá-la na OpenRouter e substituir o valor em `secrets.env`.

## Limites conhecidos

- O fast-forward distorce sistemas de `Time<Real>` (state sync, alguns
  timers); o cenário usa tempo virtual e o oráculo visual cobre o tempo real.
- `verify_replay` não entra: o Vael tem streaming/wall-clock e divergiria.
- Oráculos genéricos podem dar falsos positivos no loading/streaming (bounds
  ±1000, `ERROR` de assets/áudio). Baseline primeiro; pin de findings
  conhecidos no teste (`report.violation_rules()`/`violation_pairs()`) e
  ajuste de `liveness.timeout_s`.
- CI headless com lavapipe (`mesa-vulkan-drivers`) fica como follow-up.
