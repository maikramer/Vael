# Estudo de enxugo do código do Vael (2026-11-13)

> Estudo de 2026-11-13 sobre enxugar o crate `vael`: substituir código
> artesanal por crates que já existem, reduzir repetição e cortar tempo de
> compilação. A **Fase 1** (enxugo mecânico sem risco de comportamento)
> foi aplicada no mesmo dia; as fases seguintes ficam aqui priorizadas com
> o mecanismo e o risco de cada corte. Todas as contagens e medições levam
> a data e o commit a que se referem (baseline: `d929a29` + WIP de
> `save_native`/`respawn` em curso; o build correu com outra sessão a
> compilar em paralelo — os tempos absolutos são conservadores).

## Sumário executivo

- **Nenhuma dependência nova é precisa.** Os substitutos úteis (`clap`,
  `bevy_color`, `viber::locale`, `bevy_ext_world::parse_color`, o padrão
  atómico do `bevy_ext_save`) já estão no lock, quase todos por via da
  engine. As adopções avaliadas e rejeitadas estão na secção
  [Negativas](#avaliações-negativas) com o porquê.
- **A Fase 1 (aplicada)** consolidou os padrões repetidos — glue da bridge
  `viber.vael`, família "tonumber", gates de env, parser de cor — em
  `src/compat.rs` + macro `vael_fn!`. Custo em linhas ≈ **0 líquido** (ver
  medição): o ganho é uma fonte de verdade por padrão e semânticas com
  nome, não LOC. As reduções de linhas reais vivem na Fase 2.
- **Compilação**: o binário carrega default-features do bevy (duas stacks
  de áudio, gilrs, sysinfo), a feature `ai` do `viber` (ORT + openssl) e
  595 crates no grafo normal. O corte aplicado foi o **GC do target** (as
  unidades do Vael agora varridas; 6,4 GB libertados na primeira corrida).
  Os cortes de features ficam priorizados como pilotos na Fase 3.
- **Suite pós-F1**: `cargo test --lib` verde (326 testes, 0 falhas,
  2026-11-13), incluindo os testes de bridge que exercitam cada função
  `viber.vael` ao vivo pela VM. O boot smoke (`--scene main --entry dev`)
  confirmou os 11 módulos de bridge a instalar no arranque; o render morreu
  depois por OOM de GPU — contenção com o Jackdaw da sessão concorrente
  (GPU de 6 GB a 91%), padrão já registado no `MIGRATION.md`, alheio ao F1
  (que não toca em render).

## Mapa do crate (2026-11-13, `d929a29`)

`src/` tem 40 ficheiros e ~28,7k linhas, das quais ~21% são `#[cfg(test)]`
(harvest 822, economy 732, gear 699…). Maiores: `harvest.rs` 2660,
`ui/screens.rs` 2601, `quest.rs` 1586, `economy.rs` 1569, `gear.rs` 1509,
`quest_events.rs` 1498. Funções maiores: `player_movement` (~356 lin),
`sync_screens` (~288), `install` do gear (~272),
`VaelSave::from_world_kv` (~243), `harvest_input` (~229).

O workspace tem `default-members = ["."]`: `cargo run`/`test` na raiz não
compilam `editor/` (legado) nem `content/` (vivo, dependência do jogo).
Uma feature cargo apenas (`hanabi`); os gates `VAEL_*` são todos env vars
de runtime — o código compila sempre inteiro.

## Padrões de repetição

| Padrão | Onde (exemplos) | Acção |
|---|---|---|
| **A. Glue da bridge** — por função: `Arc::clone` + `vael.set` + `create_function`; por módulo: get-or-create da tabela `vael` | 53 closures em 11 módulos (`vault`, `xp`, `gear`, `quest`, `combat`, `economy`, `travel`, `quest_events`, `clock`, `environment`, `harvest`) | **F1**: `vault::vael_table` + macro `vael_fn!` |
| **B. Família "tonumber"** — coerção Lua/JSON com tolerâncias ligeiramente diferentes por cópia | `lua_number` ×2 + variante com strings; `lua_count` ×2; `truthy`; `write_number` ×2 (uma engolia NaN, outra não); leitores "número do legacy KV" (`save.rs::as_*`, `hud.rs::number_like`, `rules_shadow.rs::legacy_num`) | **F1** para o lado Lua (`compat`); F2 para o lado JSON (`save.rs` e afins, após o WIP assentar) |
| **C. Gates de env** — `env_enabled` copiado, listas "1/true/on/yes" implícitas | `harvest.rs`, `quest_events.rs` (cópias exactas); variantes em `rules_authority`, `main.rs`, parses numéricos em `environment`/`lighting`/`flora` | **F1**: `compat::env_enabled`; F2: os parses numéricos com filtro |
| **D. UI triplicada** — `label()` idêntico ×3; `toast`/`sfx`/`patch_game` no `screens` e no `map`; 8 construtores `RowVm` com o mesmo esqueleto; ciclo "values → position → (i+1)%len" ×4 no `a11y_click` | `ui/mod.rs`, `ui/screens.rs`, `ui/map.rs` | **F2** (ecrãs estão em evolução activa) |
| **E. `from_world_kv`** — mega-match de migração de saves (~10× o mesmo braço) | `save.rs` | **Não mecanizar** agora: é caminho de migração com semântica fina e testes; uma tabela de mapeamento só depois do WIP de `merge_world_kv` fechar |
| **F. `register_type` repetido** — a cadeia registada no plugin e nos 2 testes | `save_native.rs` | F2 trivia |
| **G. Overlays "fechar tudo"** — a mesma sequência de `open(false)`/`set_visible` | `ui/screens.rs` ×5, `save_native.rs` | F2: helper `close_overlays` |
| **H. Constantes duplicadas** — `RAIN_HEAVY` no `clock` e no `rules_shadow` | — | F2: re-exportar do `clock` (que já é `pub`) |

### Medição honesta da F1

Os 11 módulos ficaram **−146 linhas** e o `compat.rs` novo tem 153
(incluindo ~75 de testes e ~20 de docs): **líquido ≈ 0**. A estimativa
inicial (−400/−600) não contava com o rustfmt a rebentar os argumentos do
macro em linhas separadas. O que a F1 compra de facto:

- uma implementação por padrão (a tabela `viber.vael`, o `tonumber`, o
  `write_number`, o gate de env, o parser de cor, o `kv()` de teste);
- semânticas antes implícitas passam a ter nome (`lua_number` estrito vs
  `lua_tonumber` do Lua; `write_number` engole NaN — unificação que pôs o
  XP no contrato "NaN nunca entra no jogo", antes só explícito no combate);
- uma função nova da bridge regista-se numa linha.

Se o objectivo for reduzir LOC, os alavancas estão na F2 (clap no
`main.rs` ~−90, `RowVm` builder ~−80, `close_overlays` ~−30, `label`×3
~−20), não em mais dedup micro.

## Substituições por crates

Política aplicada: [`../Viber/docs/CRATES.md`](../../Viber/docs/CRATES.md)
— substituir roda própria sem mudar comportamento visível, zero deps git,
`bevy = "0.20.1"` sem pin exacto (gate do Jackdaw). **Tudo abaixo é
substituição interna: nenhum pacote novo no lock.**

| O quê | Substituto | Estado |
|---|---|---|
| Parser hex de cor do popup (`harvest`) | `bevy_ext_world::xml::values::parse_color` (mesma matemática `/255`; wrapper mantém o contrato estreito "só `#hex`, inválida cai no amarelo" — nomes/`0x` do formato de cena continuam fora) | **F1 aplicado** (−29 lin) |
| CLI à mão (`main.rs`, ~95 lin) | `clap` derive (a mesma stack do Viber, já no lock) | **F2** — preservar path posicional, `--jackdaw-extract-schema` silenciosa, `--force`, `VAEL_RESOLUTION` |
| Locale carregado à mão + `label()` ×3 | `viber::locale::Locale` (o `LocalePlugin` já corre no arranque — os `locale/{pt,en}.json` são lidos duas vezes) | **F2** — o Vael precisa dos dois idiomas (o idioma vem do save) |
| `write_meta` sem fsync (`save_native`) | padrão tmp+fsync+rename do `bevy_ext_save::save_to_disk` | **F2** (robustez: kill −9 entre write e rename pode perder a meta) |
| Família tonumber / gates env | módulo próprio `src/compat.rs` | **F1 aplicado** |

### Avaliações negativas

- **`bevy_tweening`**: não há um único lerp/easing caseiro para substituir
  (o smoothing da câmara vem da engine). Adoptar seria dep nova contra o
  espírito do enxugo.
- **`thiserror`**: não existem enums de erro caseiros; `anyhow` está bem
  aplicado. Nada a fazer.
- **serde `#[serde(untagged)]` no `save.rs`**: os leitores defensivos
  aceitam legado (strings JSON, listas `a,b,c`) com mensagens de erro
  úteis; untagged degradaria os erros. Manter.
- **`distance3`/`distance_xz`** (14 lin): `bevy_math` cobre, mas operam
  sobre `[f32;3]` e a conversão de tipos espalharia mais mudanças do que
  as linhas poupadas. Manter.
- **Cooldown à mão** (`harvest.rs::focus.cooldown`, 3 lin): `Timer` do
  Bevy não paga as linhas. Manter (e `state_sync`/`hud_ui` já usam `Timer`
  onde deve).
- **`parse_components`** (o `gmatch` do prop): paridade documentada com o
  Lua, edge cases em comentário. Manter.
- **`shadow_tick` duplicado** (shadow vs authority): duplicação deliberada
  de paridade com o `rules.lua`. Não é alvo.
- **`player_position` ×2** (`ui/map`, `ui/waypoint`): semânticas distintas
  (o do waypoint tem fallback para o `TravelState`); unificar mudaria
  comportamento no mapa. Avaliado, não unificado.

## Compilação

Grafo normal do jogo: **595 crates únicos** (2026-11-13). Pesos principais:

- `bevy` com **default-features ON**: `bevy_audio`→rodio→cpal 0.17 (ao
  lado do `bevy_kira_audio`→cpal 0.18 que o jogo usa de facto), `bevy_gilrs`
  (gamepad), `sysinfo_plugin`, x11+wayland. O CRATES.md já aponta o corte
  como frágil (re-listar ~20 features default).
- `viber` com `features = ["ai"]`: `celer-*` → `ort` + `ureq` +
  `native-tls` + `openssl` + `ndarray` + `tokenizers` (com icu/esaxx…).
- `jackdaw_runtime` (`pie`): 14 crates `jackdaw_*` no binário do jogo (as
  ~30 restantes do lock vêm só do `editor/`, fora do build normal).
- `mlua` vendored compila a VM Luau em C++; `glam` ×6 versões (nalgebra/
  landmass/kira/bevy — não colapsáveis por `cargo update`); ~65
  proc-macros.
- Sem `dynamic_linking`, `trace/tracy`, `meshlet` em lado nenhum; `hanabi`
  optional e desligado.

Perfis: o Vael define `[profile.dev]` igual à engine mas **não tem
`[profile.release]`** — o dia-a-dia (`CARGO_TARGET_DIR=../Viber/target
cargo run --release`) usa o release default do Cargo enquanto o Viber usa
`lto="thin", codegen-units=16`; os fingerprints das deps divergem e o
target partilhado guarda **duas cópias das deps release**.

### Baseline (2026-11-13, `d929a29`, sessão concorrente activa)

| Medição | Valor |
|---|---|
| `cargo check` morno (tudo em cache) | ~1,2 s |
| `cargo check` com invalidação (lib + celer-nlu frescos) | ~222 s |
| incremental após `touch src/lib.rs` | ~2,9 s |
| `cargo test --lib` (compilar + correr, 1ª vez do dia) | ~291 s |

GC do target: as unidades do Vael não estavam na lista do
`scripts/target-gc.sh` do Viber — 14 gerações de `libvael` e binários de
teste de ~450 MB acumulavam sem morte. Com o patch (unidades
`vael|vael_editor|vael_content`), a primeira execução real (2026-11-13)
libertou **6,4 GB** (o target partilhado passou de ~16 GB para 9,8 GB). O
GC só corre pelos builds do wrapper da engine; builds do Vael devem
corrê-lo à mão de quando em quando
(`bash ../Viber/scripts/target-gc.sh ../Viber/target ../Viber`).

Nota de higiene do irmão: `celer-nlu` compila com 31 warnings
(`unsafe_op_in_unsafe_fn` na edição 2024, código AVX2) — não é deste
âmbito, mas é barato de silenciar no CelerAI.

### Cortes priorizados (por razão ganho/risco)

1. **GC das unidades vael** — aplicado (F1). Mecanismo: lista `units` do
   `target-gc.sh`. Risco ~0 (apagar a geração errada custa uma
   recompilação).
2. **Política de `[profile.release]`** — ou replicar `lto="thin",
   codegen-units=16` no Vael, ou tirar o LTO do Viber; qualquer dos lados
   mata a dupla cópia de deps no target. Custo: link mais lento (com LTO)
   ou binário mais lento (sem). Decisão de produto, não mecânica.
3. **`default-features = false` do bevy/viber** — mata uma stack de áudio
   C (alsa-sys/rodio), gilrs, sysinfo, vorbis. Frágil (CRATES.md:67): o
   piloto tem de re-listar as defaults e fazer QA runtime de áudio/gamepad
   antes de ligar.
4. **`ai` fora do loop dev** — a feature já é opt-in no `viber`; o Vael é
   que liga. Um feature `ai-dev` invertida poupa ORT/openssl nos builds de
   desenvolvimento; o binário de jogo continua com `ai`. Muda o binário —
   QA das quests de decisão antes.
5. **`bevy/dynamic_linking` num perfil dev dedicado** — troca 5–20 s de
   link por arranque; exige disciplina para nunca fazer release com ela.
6. **Dedup de versões** (`rand`, `getrandom`, `base64`, `itertools`) por
   `cargo update` dirigido — ganho pequeno, fazer de caminho.
7. **Split do crate** (tirar `ui/screens`/`harvest` para sub-crate) — só
   se o inner-loop dev do jogo doer; para `--release` o custo dominante
   são deps + link, não o crate do jogo.

## Plano faseado

- **F1 (aplicado 2026-11-13)**: `src/compat.rs`
  (`lua_number`/`lua_tonumber`/`lua_count`/`truthy`/`write_number`/
  `env_enabled` + testes), `vault::vael_table` + macro `vael_fn!` nos 11
  módulos de bridge (nomes/assinaturas Lua imutáveis), `parse_popup_color`
  sobre o `parse_color` da cena, `kv()` de teste partilhado, unidades do
  Vael no `target-gc.sh`.
- **F2 (aplicada 2026-11-13)**: clap no `main.rs` (`Args::parse`; o `--force`
  passa a exigir `--export-settings` e o `--help`/erros passam a ser do
  clap — a forma das flags do `viber run` não mudou); `label()` com fonte
  única no `content.rs` (usado por `ui/mod`, `ui/map` e `ui/screens`);
  leitores JSON do legacy (`json_i64`/`json_f32`/`json_bool`/
  `json_number_like`) no `compat` (save/hud re-pointed); builder `RowVm::
  menu(...)` com `.with_progress/.with_icon/.info` + `close_overlays` +
  `PREF_KEYS` (a sig e o corpo das preferências já não podem divergir) nos
  ecrãs; fsync tmp+rename no `write_meta`; `register_save_types`
  partilhado plugin/testes; `RAIN_HEAVY` re-exportado do `clock` (cast
  f64→f32 nos dois usos). Duas avaliações negativas novas: o
  `viber::locale::Locale::load` engole ficheiros inválidos em silêncio
  (o loader do Vael reporta no `content.errors`, gate do teste de
  conteúdo — não se adota) e o `kv_flag`/`spawn_from_kv` do
  `rules_authority` têm semânticas próprias (strings valem `false`;
  chaves planas `spawn_*` com filtro finito — não são os do `compat`).
- **F3 (pilotos com QA runtime)**: features 3–5 da lista de compilação;
  remoção do `editor/` do workspace (é decisão de produto — o build normal
  já não o compila; só limparia lock/`--workspace`).

## Riscos — o que não se toca

- **Contrato da bridge**: nomes/assinaturas `viber.vael` são usados pelos
  scripts Lua vivos; enxugar invólucro sim, mudar assinaturas não.
- **Gates `VAEL_*` e pares Rust/Lua** (harvest/travel/quest_events
  opt-in; `VAEL_RULES_RUST=0`) — deliberados até ao QA no pool.
- **`shadow_tick`** — ferramenta de paridade com o `rules.lua`.
- **Migração de saves** (`from_world_kv`/`merge_world_kv`/`to_world_kv`) —
  semântica "nada se perde em legacy" com testes.
- **`content/`** — esquemas congelados (gate do editor).
- **Workarounds do `bevy_pf`** (`sync_menu_pages`, fontes seguras) — são
  lacunas do crate, não código morto; revalidar XAML headless após os
  tocar (warnings=0).
- **Sessões concorrentes**: validar sempre o HEAD num estado limpo e
  commitar paths explícitos.
