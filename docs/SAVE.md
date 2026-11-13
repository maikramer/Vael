# Save do Vael (`moonshine-save`)

O save do jogo é do crate `vael`. O `SavePlugin` do Viber já não corre no Vael:
`AppOptions::external_save = true` deixa no engine apenas os recursos
partilhados (`SaveRequest`, `SaveDir`, `WorldSaveKey`) e o importador de
recortes de terreno (`PendingSavePatches` + `apply_pending_save_patches`).

## Formato

- **Um ficheiro RON por slot**, escrito pelo `moonshine-save`:
  `<world>.auto.vsave.ron`, `<world>.slot1.vsave.ron`, …,
  `<world>.legacy.vsave.ron`.
- **Um sidecar** `<slot>.vsave.meta.json` com `save_id`, `saved_at`,
  `playtime`, `title`, `level`, `gold` e `scene`; é o que o menu lê sem
  deserializar o RON.
- O payload é um único recurso refletido, `VaelSaveFile` (`src/save_native.rs`):
  posição, volumes, cena/`came_from`, idioma, recortes de terreno e rebinds
  ficam tipados; `VaelSave` (com o `legacy` não refletível), o `world_kv` do
  Lua e os rebinds viajam como strings JSON dentro desse recurso.

## Fluxo

1. A UI, o menu e o Lua (`viber.save`/`viber.save_slot`/`viber.load_slot`)
   escrevem `SaveRequest` como antes.
2. `process_save_requests` corre depois de `luau_update`, captura o estado
   (`capture`) ou dispara `LoadWorld`.
3. Os observers do `moonshine-save` escrevem/lêem o RON; `Saved`/`Loaded`
   marcam o `NativeSaveState`.
4. `finish_native_save` grava o sidecar; `finish_native_load` confirma o
   `save_id` e aplica o payload (Lua `json_to_game`, `VaelSave`, volumes,
   idioma, rebinds, `PendingSavePatches`, cena/posição).

## Compatibilidade

- Saves antigos (`*.save.json` do envelope da engine) continuam a ser
  listados e importados por `legacy_payload` na primeira leitura; o primeiro
  save nativo passa a ser a fonte.
- O `game.vael` tipado do envelope antigo é lido quando existe; caso
  contrário o `VaelSave` é migrado do `world_kv`.

## QA

- `cargo test -p vael --lib` cobre o roundtrip real do moonshine
  (`save_native::tests::moonshine_grava_e_carrega_o_payload`), o sidecar e o
  merge do `VaelSave`.
- Ao vivo: `viber.save_slot(1)` → ficheiros `.vsave.ron`/`.meta.json`;
  teleportar e `viber.load_slot(1)` → posição reposta; matar e “Tentar de
  novo” → HP/frascos/posição; morte num interior → volta a `main`.
