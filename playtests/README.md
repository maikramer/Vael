# `playtests/` — cenários do `bevy_playtest`

Lidos por `tests/playtest_scenarios.rs` (in-process, GPU). O schema é o do
crate (`Scenario`/`BotConfig`; campos desconhecidos são erro). O cenário QA
do Vael (`VAEL_SCENARIO`) vem da tabela do teste, não deste JSON.

| Ficheiro | QA state | Scope |
|---|---|---|
| `menu.json` | start-scene | fast |
| `sandbox_chaos.json` | `sandbox` | fast |
| `arena_chaos.json` | `combat/arena` | fast |
| `arena_scripted.json` | `combat/arena` | fast |
| `interior_forge.json` | `interiors/forge` | fast |
| `main_landmark.json` | `world/forge_stage2` | world |
| `soak_sandbox.json` | `sandbox` | soak (600 s) |

```bash
PLAYTEST_ONLY=menu PLAYTEST_SEEDS=42 \
  cargo test --features playtest --test playtest_scenarios -- --ignored --test-threads=1 --nocapture
```

Detalhe (invariantes, seeds, limitações): [`../docs/PLAYTEST.md`](../docs/PLAYTEST.md).
