# Estudo de padronização da arquitectura de desenvolvimento (2026-11-14)

> **2026-11-15:** o crate `bevy_ext_scene_runtime` foi **apagado** (órfão,
> zero consumidores; ver [`DELETE-MONOLITH.md`](DELETE-MONOLITH.md)). As
> linhas B2/B4/E3 que o citam ficam como história da medida de 2026-11-14.

> **Estudo de 2026-11-14.** Define o standard de engenharia para o **Vael** e o
> **`bevy_extensions`**, mede o desvio actual e faseia a convergência
> (S0–S4). Âmbito decidido: os dois checkouts do jogo/engine; `viber-tools`,
> `AiGameKit` e `CelerAI` ficam fora (referência pontual). O estudo **não
> duplica os donos existentes**: o roadmap de pacotes é o
> [`../../bevy_extensions/docs/PACKAGES.md`](../../bevy_extensions/docs/PACKAGES.md),
> a política de crates é o
> [`../../bevy_extensions/docs/CRATES.md`](../../bevy_extensions/docs/CRATES.md),
> o estado da migração é a [`MIGRATION.md`](MIGRATION.md), o mapa
> peça→destino é o [`EXIT-MAP.md`](EXIT-MAP.md) e as convenções de índice são
> as do [`README.md`](README.md). As contagens foram medidas a 2026-11-14 e
> mudam por onda — vale o "≥N desde a data".

## 1. Âmbito e método

- Levantamento por inspecção directa de manifests, workflows, scripts, docs e
  fontes dos dois checkouts (não por memória de docs anteriores). Caminhos,
  linhas e números citados são verificáveis no commit do dia.
- Cada regra tem **id**, **nível** (`MUST`/`SHOULD`), **alvo mensurável** e o
  sítio onde se aplica. `MUST` = gate de CI ou revisão; `SHOULD` = prática
  recomendada com justificação registada quando não cumprida.
- O objectivo não é reescrever a arquitectura: é **uniformizar** o que já é
  convenção implícita, eliminar duplicações de contrato e tornar cada
  fronteira verificável por máquina.

## 2. O que já é standard (não inventar de novo)

| # | Prática existente | Fonte |
|---|---|---|
| 1 | Workspace com `[workspace.package]` (edition 2024, MSRV 1.97.1, licença, repo) e `[workspace.dependencies]` centralizado | `bevy_extensions/Cargo.toml:37-67`, `viber-tools/Cargo.toml:13-40` |
| 2 | `bevy` base sem defaults e cada crate liga o mínimo que precisa | `bevy_extensions/Cargo.toml:47` (22 usos de `default-features = false`) |
| 3 | **TypePath histórico congelado** na linha 0.x, com testes-contrato | `bevy_extensions/docs/PACKAGES.md:45-57`; 394 anotações `#[type_path]` (`345` com path `viber::*`) |
| 4 | **Plugin por módulo** (`pub struct XPlugin;` + `impl Plugin`) com wiring aditivo | 128 declarações unitárias; `Vael/src/lib.rs:113-231` |
| 5 | Umbrella + subplugins documentado, com aviso contra dupla registração | `bevy_extensions/crates/bevy_ext_audio/src/lib.rs:100-109` |
| 6 | `AttrSpec` + `lift` + `KEYS`: round-trip exacto componente ↔ atributo DSL | `bevy_extensions/crates/bevy_ext_scene/src/lib.rs:29-72` |
| 7 | `SceneSetting<T>` para definições autoradas em `.bsn` | `bevy_extensions/crates/bevy_ext_world/src/bsn/settings.rs:172-178` |
| 8 | `vael_fn!` + `vael_table` como única porta da bridge `viber.vael` | `Vael/src/vault.rs:259,282-291` (66 ocorrências) |
| 9 | QA em camadas com um só cenário (sim/cena/vivo) | `Vael/docs/QA.md:3-16`; `qa/scenarios/**`; `tools/qa.py` |
| 10 | CI com `cargo fmt --check` + `clippy -D warnings` + test + doc | `bevy_extensions/.github/workflows/ci.yml` (jobs test/msrv/package); `Vael/.github/workflows/ci.yml:53-58` |
| 11 | Conventional Commits em português, com escopo por área | `git log` dos dois repos; convenção documentada só no AiGameKit (fora do âmbito) |
| 12 | Índice de documentação com donos e regras ("um facto, um dono") | `Vael/docs/README.md:3-57` |

**Mapa de maturidade** (o que falta é o objecto do standard):

| Área | Estado | Lacuna principal |
|---|---|---|
| Tooling (fmt/lint/toolchain) | 0 de 3 configs nos dois repos | Sem `rustfmt.toml`, `clippy.toml`/`[workspace.lints]`, `rust-toolchain.toml` |
| CI | Forte no `bevy_extensions`; parcial no Vael | Vael faz checkout do `Viber` aposentado e só corre `--lib`; 64% dos testes da engine fora do CI |
| Fronteiras de crates | DAG sem ciclos | Contratos duplicados (`SceneLoad` ×4, `AiLocomotionSystems` ×2); 3 excluídos com path-deps mortas |
| Contratos de runtime | Convenções implícitas | 4 formas de plugin, 30 nomes repetidos, 14 parsers de env com 3 semânticas |
| Dados/autoria | Pipeline tipado em curso | 6 camadas de representação; 12 pares gémeos divergentes; 2 stacks de física/save/UI |
| Testes/QA | Boa pirâmide local | CI não corre os gates que a doc declara |
| Documentação | Índice forte no Vael | Sem índice no `bevy_extensions`; refs mortas ao `Viber`/.xml; banners em falta |
| Release | Workflow pronto no `bevy_extensions` | Nada publicado; versões skewed; `publish.sh` quebrado; Vael sem `LICENSE` |
| Governança | Ausente | Sem CONTRIBUTING/SECURITY/templates/CODEOWNERS/dependabot nos dois |

## 3. O standard

### A. Tooling e estilo

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| A1 | MUST | `rustfmt.toml` na raiz de cada repo, com opções explícitas (edition 2024, newline Unix) | `Vael/`, `bevy_extensions/` |
| A2 | MUST | `rust-toolchain.toml` fixa **1.97.1** (o MSRV real) nos dois repos | idem; CI testa o MSRV à parte |
| A3 | MUST | `[workspace.lints]` curado (clippy `all = "warn"` + allowlist justificada) nos dois workspaces | hoje nenhum repo tem lints de workspace |
| A4 | MUST | CI bloqueante em cada repo: `fmt --check`, `clippy --all-targets -D warnings`, `test`, `doc -D warnings`, e job **MSRV** | o `bevy_extensions` já tem test/msrv/package; o Vael não tem MSRV/doc |
| A5 | SHOULD | `cargo-deny` (licenças/bans) nos dois; Dependabot a abrir PRs de deps | modelo: `vramd` upstream (fora do âmbito) |
| A6 | SHOULD | `[profile.release]` explícito e documentado (LTO/codegen-units), para o target não guardar duas gerações | hoje o Vael não tem profile release |

### B. Fronteiras e camadas

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| B1 | MUST | DAG documentado e **sem ciclos**: `core` não depende da família; `scene`/`elements`/`flora`/`water` só de Bevy (e `core` quando aplicável) | actualizar o grafo do `PACKAGES.md` quando uma aresta nova entrar |
| B2 | MUST | **Contrato partilhado mora num só crate** (`core`): `SceneLoad` (hoje ×4: `core`, `viber_engine`, `scene_runtime`, `terrain_runtime`), `AiLocomotionSystems` (×2), `Health`/`Hostile`/`Corpse` (gémeos `core`/`ai`/monólito) — re-exportar, nunca redefinir | `bevy_ext_core/src/lib.rs` |
| B3 | MUST | **Extrair ⇒ consumir**: o monólito consome o crate extraído e mantém só shim `pub use`; proibido gémeo. Gate de CI compara os pares e falha acima do limite acordado | pares medidos: `spawn.rs` 947, `water.rs` 1 437, `runtime.rs` 1 035, `postfx.rs` 865, `physics.rs` 814, `splat.rs` 598, `ai.rs` 483… (levantamento 2026-11-14) |
| B4 | MUST | **Workspace fechado**: nenhum crate de produção fora do workspace; um excluído só existe com `Cargo.lock` próprio e path-deps resolvíveis. Hoje `scene_runtime`/`physics`/`engine` apontam para `../Viber` (inexistente) e não compilam | `bevy_extensions/Cargo.toml:29-34`; alinhar ao `EXIT-MAP.md` |
| B5 | MUST | **Features**: `bevy` sem defaults; cada crate liga o mínimo; features pesadas opt-in; nomes `snake_case` (`experimental_pbr_pcss`); os nomes com hífen existentes (`luau-jit`, `ai`) ficam até um breaking documentado | `bevy_extensions/Cargo.toml:47`; `viber_engine` é a excepção a rever |
| B6 | MUST | **Pins**: caret por omissão; `=` só com razão escrita no `Cargo.toml` (hoje `bevy_hanabi = "=0.20.0"`, `bevy_lights = "=0.5.0"`, `jackdaw_* = "=0.20.2"`) | raiz do Vael e `bevy_extensions/Cargo.toml` |
| B7 | SHOULD | Um **prelude curado** por crate publicável (hoje 0 em todos) | `lib.rs` de cada crate |
| B8 | MUST | **Orçamento de módulo: 1 500 LOC**; acima disso, split obrigatório ou excepção registada no doc do módulo (hoje 42 ficheiros >1.5k, incl. `recipes/mod.rs` 7 257, `ui/style.rs` 4 376, `postfx.rs` 4 355) | ambos os repos |

### C. Contratos de runtime

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| C1 | MUST | Forma de plugin única: `pub struct XPlugin;` + `impl Plugin`; config struct **só** quando há configuração; genérico só onde o tipo é parâmetro (`WaterPlugin<S>`); **proibido nome de plugin repetido entre packages** (hoje ~30) | todos os crates da família e o Vael |
| C2 | MUST | Sets/schedules têm **um dono** (`core`): os consumidores importam, não redefinem | `core/src/lib.rs:97-112,900-905` |
| C3 | MUST | **Gates de env**: um helper por repo, três semânticas nomeadas (opt-in, default-on, trivalor) e prefixo por dono (`VAEL_` jogo, `VIBER_` engine). Catálogo num doc único (ver Apêndice A) | hoje 14 parsers locais em 3 semânticas; `compat::env_enabled` cobre só o opt-in |
| C4 | MUST | **Erros**: `anyhow` nas bordas (I/O, parsing, app); biblioteca devolve erro tipado quando há ≥2 modos de falha; **proibido `Result<_, String>` em API pública** (hoje `bevy_ext_save`); `mlua::Result` fica na fronteira de scripting | `bevy_ext_save/src/lib.rs:363,385` |
| C5 | MUST | **Nomes**: `Vael*` para estado/plugins do jogo; tipos de domínio sem prefixo no resto; ficheiros/módulos `snake_case`; sufixo `_system` em todas as funções registadas como sistemas (hoje ~50 de ~410) | `Vael/src/`; crates extraídos |
| C6 | MUST | **Superfície pública**: `pub(crate)` por omissão; `pub` só para o contrato; `#![allow(...)]` a nível de crate exige comentário de justificação (hoje há um em `Vael/src/lib.rs:21`) | ambos |

### D. Dados e autoria

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| D1 | MUST | Pipeline canónico: `.bsn` → `WorldNode` → specs tipadas → IR dos recipes → ECS. Nenhuma camada nova sem passar por este caminho | `bevy_ext_world`/`bevy_ext_scene`/recipes |
| D2 | MUST | **Componente tipado novo**: `AttrSpec` + `KEYS` + `lift`/`to_attrs` com round-trip exacto + `bsn_migrate` + `bsn_equiv` + teste de `type_path` | `bevy_ext_scene/src/lib.rs:29-72`; `bsn_equiv` como gate |
| D3 | MUST | **TypePath congelado**: tipos existentes mantêm o path histórico; tipos novos usam o path do crate novo; nunca dois tipos distintos com o mesmo path | `PACKAGES.md:45-57`; testes-contrato |
| D4 | MUST | **Uma stack por domínio**, com end-date para as legadas: save (hoje `VaelSave` + `moonshine-save` + `bevy_ext_save`), UI (`bevy_pf` no jogo; `bevy_ext_ui` sem consumidor — não ganhar novos), física (`avian` é o keeper do `EXIT-MAP`; Rapier só no legado até à onda final) | `EXIT-MAP.md`; `docs/SAVE.md`, `docs/PHYSICS-AVIAN.md` |
| D5 | MUST | Prefabs (`world/prefabs/**`), identidade `WorldOrigin`, `SceneNodeId` no disco, células de 128 m e `CustomProperties` do Jackdaw seguem as convenções do [`AGENTS.md`](../AGENTS.md) | `AGENTS.md` (Prefabs e organização do mundo) |

### E. Testes e QA

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| E1 | MUST | Pirâmide: unit inline (`#[cfg(test)]`), integração em `tests/`, cenários QA (`qa/scenarios`) e QA vivo. Um teste novo escolhe o nível mais barato que cobre o risco | Vael (390 `#[test]` inline) e crates |
| E2 | MUST | **A CI corre o que a doc declara**: `viber analyze`, `viber scene lint`, `viber bsn check`, `cargo test --release --test qa_scenarios`, `--test water_overhaul` | hoje a CI do Vael corre só `-p vael --lib` (`ci.yml:63`) |
| E3 | MUST | Crates excluídos do workspace têm **CI próprio** com o seu lock (hoje `viber_engine` 1 456 + `engine` 91 + `physics` 43 + `scene_runtime` 19 testes nunca correm no CI) | workflow por crate excluído |
| E4 | MUST | Determinismo: golden tests para RNG/noise e round-trips (já existe); qualquer relaxação exige justificação | `core/rng.rs`, `content/tests/roundtrip.rs` |
| E5 | MUST | O cenário QA é o **único artefacto** de teste de gameplay (sim/cena/vivo usam o mesmo JSON) | `docs/QA.md:3-16` |
| E6 | SHOULD | Extensões Jackdaw (`extensions/*`) têm gate `cargo check` no CI do `bevy_extensions` | hoje sem CI |

### F. Documentação

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| F1 | MUST | Cada repo tem **índice de docs** com vivo/histórico e convenções | Vael tem (`docs/README.md`); o `bevy_extensions` não |
| F2 | MUST | Cada repo tem `AGENTS.md` curto e actual (o do `bevy_extensions` ainda se titula "Viber" e manda correr `install.sh`, que não existe) | `bevy_extensions/AGENTS.md:1,386-398` |
| F3 | MUST | Estudos e medições históricos levam banner (`> **Estudo/Medição de \<data>…**`); `ARCH-REVIEW-STUDY`/`CODE-SLIM-STUDY` ainda não têm | `TERRAIN-VOXEL-STUDY.md` é o modelo |
| F4 | MUST | "Um facto, um dono": nada de copiar tabelas entre docs; o dono é citado | `docs/README.md:44-46` |
| F5 | MUST | **Zero referências mortas**: `../Viber` (aposentado a 2026-11-13), `.xml` como fonte, `install.sh` inexistente. Gate grep no CI | hoje: `Vael/README.md`, `CONTEXT.md`, `.cursor/skills/*`, `bevy_extensions/AGENTS.md`, `viber-tools/docs/COMPAT.md` |
| F6 | SHOULD | Checker de links relativos em Markdown (CI) | não existe |
| F7 | MUST | **Língua**: PT nos docs/commits/comentários/testes do **jogo** (Vael, monólito interno); **EN** nos crates publicáveis (`bevy_ext_*` da release e futuros publicáveis) e nos READMEs públicos. Desvio actual nos crates: ~180 linhas `//!` PT vs ~839 EN (levantamento) — converge com o sweep de release | ambos |
| F8 | MUST | Datas e contagens: medição leva data+commit; preferir "≥N desde \<data>" | `docs/README.md:50-52` |
| F9 | SHOULD | Estudos trazem evidência (comandos/medições) em apêndice, para re-verificação | este doc, Apêndice A |

### G. Release e versionamento

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| G1 | MUST | Semver lockstep na linha 0.x; subir Bevy é breaking da família | `PACKAGES.md:59-67` |
| G2 | MUST | **Versões alinhadas**: workspace `0.1.0` vs 6 crates `0.1.1` (`core`, `world`, `gltf`, `save`, `terrain`, `voxel_terrain`) — subir o `[workspace.package]` **e as entradas de `[workspace.dependencies]`** a `0.1.1` (ou repor todos) antes de publicar | `bevy_extensions/Cargo.toml:37,47-67`, `crates/*/Cargo.toml:1-9` |
| G3 | MUST | **`tools/publish.sh` correcto e com dry-run verde**: falta `bevy_ext_scene` na ordem (o `world` depende dele) e o `is_published` por `cargo search` é frágil | `bevy_extensions/tools/publish.sh:72-88` |
| G4 | MUST | Tag `vX.Y.Z` por release; o workflow valida tag==versão | `bevy_extensions/.github/workflows/release.yml:27-40` |
| G5 | MUST | `CHANGELOG.md` Keep-a-Changelog por repo, actualizado no release | `bevy_extensions` tem; o Vael não (é binário, mas precisa de release notes) |
| G6 | MUST | **`LICENSE` na raiz de cada repo**: o Vael **não tem nenhum** apesar de `content/`/`editor/` declararem `MIT OR Apache-2.0` e de a doc o afirmar; o `bevy_extensions` tem `LICENSE-APACHE`/`LICENSE-MIT` | lacuna legal concreta |
| G7 | MUST | Metadados completos por crate publicável: `license`, `repository`, `authors`, `description`, `keywords`, `categories`, `readme`, `docs.rs` | 13 READMEs para 22 crates; só o monólito sem `keywords` |
| G8 | SHOULD | `cargo-semver-checks` no CI dos publicáveis | princípio já em `PACKAGES.md:71`, sem job |
| G9 | MUST | Checklist de release por repo (build, gates, versão, tag, CHANGELOG, publish, release notes) | não existe em doc versionado |

### H. Governança

| ID | Nível | Regra | Alvo / onde |
|---|---|---|---|
| H1 | MUST | `CONTRIBUTING.md` nos dois repos (setup com os irmãos, gates, estilo, commits) | só o AiGameKit tem (fora do âmbito) |
| H2 | MUST | `SECURITY.md` (como reportar; política de segredos) | nenhum dos dois |
| H3 | MUST | Templates de PR e issue em `.github/` | nenhum |
| H4 | SHOULD | `CODEOWNERS` (com um maintainer, mapeia áreas para revisão futura) | nenhum |
| H5 | SHOULD | Dependabot a abrir PRs de deps (Cargo) | só o AiGameKit |
| H6 | MUST | Política de branches: `main` + branches curtas de feature/exit com fast-forward; sessões concorrentes não se revertem (regra da memória do projecto, agora escrita) | `concurrent-sessions-same-worktree` (memória) |
| H7 | MUST | Conventional Commits em PT nos dois repos (`tipo(escopo): assunto`, corpo multi-parágrafo quando há decisão) | já praticado; formalizar aqui |
| H8 | MUST | **Processo de ship em doc versionado** (`docs/SHIP.md`), não só em memória — a memória `ship-all-siblings-pt` ainda lista o `Viber` | Vael |

## 4. Gap analysis

| Área | Hoje (evidência) | Alvo | Gap | Esforço/risco |
|---|---|---|---|---|
| Tooling | 0/3 configs; fmt/clippy bloqueantes só em 3 repos; MSRV testado só na engine | A1–A4 | Criar configs + fechar gates | Baixo; pode revelar warnings (corrigir com `#[allow]` justificado) |
| CI | Vael: checkout do `Viber` morto (`ci.yml:21-24`), só `--lib`; 1 609/2 513 testes da engine fora | E2–E3, A4 | Reescrever o job do Vael; jobs por crate excluído | Médio; exige runner com 5 checkouts e tempo de CI |
| Contratos | `SceneLoad` ×4, `AiLocomotionSystems` ×2, 30 nomes de plugin, 14 parsers de env | B2, C1, C3 | Mover contratos ao `core`; helper de env; lint de nomes | Médio; toca monólito (coordenação com exit2/exit3) |
| Gémeos | 12 pares divergentes (até 1 437 linhas) | B3 | Regra + gate; convergência é o F3 do ARCH-REVIEW | Alto; já faseado nas waves |
| Workspace | 3 excluídos com path-deps mortas; sem lock | B4 | Arquivar ou resolver (alinhado ao `EXIT-MAP`) | Médio; decisão de produto |
| Docs | Refs mortas ao `Viber`/.xml; sem índice na engine; sem banners nos estudos; `EXIT-MAP` fora do índice | F1–F8 | Limpeza + gate de links/refs | Baixo |
| Release | Nada publicado; versões skewed; `publish.sh` sem `scene`; Vael sem `LICENSE`; sem tags/CHANGELOG no Vael | G1–G9 | Alinhar versões, corrigir script, legal, tags | Médio; publicação precisa do token (utilizador) |
| Governança | 0/5 itens nos dois repos | H1–H8 | Criar ficheiros e doc de ship | Baixo |

## 5. Plano faseado

Cada fase é auto-contida e deixa o repo verificável; os gates citados são os
da secção 3. Nenhuma fase re-litiga decisões vivas (secção 6).

### S0 — Drift e legal (1 sessão)
- `LICENSE` na raiz do Vael (`MIT OR Apache-2.0`, como `content/`/`editor/`).
- CI do Vael: sair o checkout `maikramer/Viber` e a entrada `Viber` do
  `rust-cache`; actualizar o comentário dos irmãos.
- `tools/check.sh`: `--full` deixa de usar `../Viber/target`; o gate local
  passa a incluir `cargo fmt --check` e `cargo clippy -D warnings`.
- Banners nos dois estudos (`ARCH-REVIEW-STUDY`, `CODE-SLIM-STUDY`);
  `EXIT-MAP.md` entra no índice.
- `bevy_extensions/AGENTS.md`: título/contrato de engine (não "Viber");
  remover `install.sh` inexistente e apontar `viber-tools`.
- Limpar refs mortas: `Vael/README.md`, `CONTEXT.md`, `.cursor/skills/*`,
  `viber-tools/docs/COMPAT.md` (só a parte de docs; o código do
  `viber-tools` está fora do standard).
- **Gate:** `grep -rn '../Viber'` sem resultados nos dois repos (fora de
  menções históricas marcadas); `tools/check.sh` verde; CI verde.

### S1 — Baseline de ferramentas
- `rustfmt.toml`, `rust-toolchain.toml` (1.97.1) e `[workspace.lints]` nos
  dois repos; correr `cargo fmt` uma vez (diff mecânico, commit isolado).
- CI do Vael ganha jobs de MSRV e `cargo doc -D warnings`; CI de ambos passa
  a correr `--all-targets`.
- Vael: `qa_scenarios` + `water_overhaul` no CI (ou `tools/check.sh --full`).
- **Gate:** CI verde nos dois; `cargo fmt --check`/`clippy -D warnings` no
  job local e no CI.

### S2 — Contratos e gates
- `SceneLoad`/`AiLocomotionSystems` (e os gémeos de vitals/corpse quando a
  onda respectiva fechar) passam a existir só no `core`, com re-export.
- Helper único de gates de env por repo + catálogo `docs/ENV.md` (Apêndice A
  como semente); prefixo por dono.
- Gate de CI: nomes de plugin duplicados, `Result<_, String>` em API pública
  e pares gémeos acima do limite.
- **Gate:** `cargo test --workspace` verde; o gate novo falha em regressões
  conhecidas e passa depois da correcção.

### S3 — Workspace e release
- Alinhar versões (workspace 0.1.1 ou repor 0.1.0) e corrigir
  `tools/publish.sh` (`bevy_ext_scene` na ordem; dry-run verde).
- Excluídos: `scene_runtime`/`physics`/`engine` arquivados ou resolvidos
  (path-deps ao `Viber` mortas) — decisão alinhada ao `EXIT-MAP`.
- Tags `vX.Y.Z` + `CHANGELOG` no Vael (binário: notas de release) e na
  engine; `docs.rs` metadata; `cargo-semver-checks` (SHOULD).
- **Gate:** `tools/publish.sh --dry-run` verde; `cargo test --workspace`;
  CI do `release.yml` a passar numa tag de teste.

### S4 — Governança, docs e QA
- `CONTRIBUTING.md`, `SECURITY.md`, templates PR/issue, `CODEOWNERS`,
  Dependabot nos dois repos; `docs/SHIP.md` (substitui a memória como fonte).
- Índice de docs no `bevy_extensions`; gate de links/refs mortas; aplicar a
  regra de língua (F7) no sweep de release dos publicáveis.
- **Gate:** `make`/`check` documentado; CI corre o gate de links; ship doc
  validado por um release de teste.

## 6. Não-objectivos (decisões vivas)

- **TypePath congelado**, lockstep 0.x, RNG/noise congelados, terreno próprio,
  `avian` como keeper da física — vêm do `PACKAGES.md`/`EXIT-MAP.md` e não se
  re-litigam aqui.
- **Sem `thiserror` blanket**: o CODE-SLIM avaliou e rejeitou; o standard só
  exige erro tipado onde há ≥2 modos de falha.
- **As waves do monólito** (exit2/exit3) pertencem ao `EXIT-MAP.md`; este
  standard define a regra (B3) e o gate, não a ordem das waves.
- `AiGameKit`, `CelerAI` e `viber-tools` **fora do âmbito**; são referência
  (o AiGameKit é o modelo de CONTRIBUTING/make check; o CelerAI o modelo de
  runbook de release; o `vramd` o modelo de SECURITY/release por tag).
- Sessões concorrentes: não reverter WIP alheio; validar num HEAD limpo e
  commitar paths explícitos (regra H6).

## Apêndice A — Catálogo de gates de ambiente (levantamento 2026-11-14)

Semântica A — **opt-in** (`1/true/on/yes`; default OFF):

| Variável | Onde |
|---|---|
| `VAEL_HANABI` | `src/hanabi_fx.rs:12` |
| `VAEL_HARVEST_RUST` | `src/harvest.rs:67,511` |
| `VAEL_TRAVEL_RUST` | `src/travel.rs:34,395` |
| `VAEL_QUEST_EVENTS_RUST` | `src/quest_events.rs:45,63` |
| `VAEL_RULES_SHADOW` | `src/rules_shadow.rs:496,602` |
| `VAEL_EDITOR_LINK` | `src/editor_link.rs:53` (o `JACKDAW_PIE` também liga) |
| `VIBER_ECS_MIRROR` | `main.rs:294` |

Semântica B — **default ON** (só desliga com `0/false/off[/no]`):

| Variável | Onde |
|---|---|
| `VAEL_ELEMENTS` | `src/environment.rs:144` |
| `VAEL_PRACTICAL_LIGHTS` | `src/lights.rs:197` |
| `VAEL_FLORA`, `VAEL_FLORA_GROVE` | `src/flora.rs:253,364` |
| `VAEL_PRIMITIVES` | `src/primitives.rs:49` |
| `VAEL_HUD_RUST` | `src/hud.rs:22,33` |
| `VAEL_RULES_RUST` | `src/rules_authority.rs:46,72` |
| `VAEL_UI_PF` | `ui/mod.rs:31,249` |
| `VIBER_GRASS` | `src/grass/mod.rs:96` (só `"0"` desliga) |
| `VAEL_TOOLS` | `main.rs:281`; `JACKDAW_PIE` também liga |
| `VAEL_FIRE_MIGRATE` | `src/lights.rs:146` |

Semântica C — **valores/trivalor**:

| Variável | Default | Onde |
|---|---|---|
| `VAEL_ELEMENTS_QUALITY` | `medium` (low/high/ultra) | `src/environment.rs:122` |
| `VAEL_START_HOUR` / `VAEL_START_WEATHER` | cena | `src/environment.rs:69,84` |
| `VAEL_POOL_SIZE` / `VAEL_POOL_SHADOWS` | 24 / 2 | `src/lights.rs:74,140` |
| `VAEL_FIRE_FILL` / `VAEL_FLAME_BILLBOARDS` | 1 / 1 | `src/lights.rs:142-146` |
| `VAEL_FLORA_SHADOW_RADIUS` / `VAEL_FLORA_IMPOSTOR` | cena | `src/flora.rs:260,270` |
| `VAEL_PRIMITIVES_DETAIL` | `high` | `src/primitives.rs:59` |
| `VAEL_RESOLUTION` | 1280×720 da engine | `main.rs:145,306` |
| `VAEL_QA_SAVE_DIR` / `VAEL_SCENARIO` | `temp/vael-qa-saves` / — | `main.rs:107,210` |
| `VAEL_NATIVE_ALLOW` | vazio (lista por vírgulas) | `src/editor_link.rs:83` |
| `VIBER_GRASS_DENSITY` / `VIBER_GRASS_DEBUG` | 1 / OFF | `src/grass/mod.rs:97,837` |
| `VAEL_PREVIEW*` (extensão Jackdaw) | ver README da extensão | `extensions/bevy_voxel_terrain_jackdaw/src/world_preview.rs:117-121` |

**Alvo (C3):** um único helper por repo, um catálogo (`docs/ENV.md`) e um
prefixo por dono; os 14 parsers locais convergem para o helper.
