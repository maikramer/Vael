---
name: vael-editor
description: Usar para abrir/editar o Vael no editor oficial do Jackdaw (jd open), empacotar extensões .jdext e validar cenas world.bsn.
---

# Editor oficial do Jackdaw (Vael)

O fluxo é o **Jackdaw oficial instalado** (`jackdaw`/`jd` 0.20.2). O crate
`editor/` deste repo é legado e já não é o caminho de autoria.

```bash
jd open .                    # abre /home/maikeu/GitClones/Vael no editor oficial
jd build                     # reconstrói o schema/PIE do jogo
jd doctor --project .        # valida o projeto
```

A raiz tem `jackdaw.toml` com `package = "vael"`, `plugin = "VaelPlugin"`,
`assets_dir = "assets/public"` e `[extensions]`. O `jd build` extrai o schema
da lib (`--jackdaw-extract-schema`) e o Play (PIE) corre o jogo.

## Extensões

As extensões do Vael são projetos oficiais em `../bevy_extensions/extensions/`
(`jackdaw_extension`, `[workspace]` e `jackdaw.toml` próprios). O ciclo é:

```bash
cd ../bevy_extensions/extensions/bevy_voxel_terrain_jackdaw
jd build
jd extension pack
jd extension install bevy_voxel_terrain_jackdaw-*-x86_64-unknown-linux-gnu.jdext \
  --trust-publisher
jackdaw check-extensions --expect bevy_voxel_terrain_jackdaw
```

A extensão do terreno voxel adiciona o painel `Vael: Voxel Terrain`, os
operadores `vael.voxel.add_{cave,arch,bridge}`/`rebuild` e o preview
Transvoxel. Como declara `app_plugin`, só ativa **no arranque seguinte** do
editor.

- Cada nó continua `bevy_world_xml::bsn::WorldNode { tag, attrs, text, origin,
  pose }`: `Transform`/`Name` são a pose/nome; `GltfSource` e luzes nativas
  são preview.
- **CLI de cena XML-only**: `viber scene`/`tool settle`/`scatter`/`bake` nos
  `.xml` legados; `viber analyze`/`scene lint` leem `.bsn`.
- O pool do Vael é meshopt/KTX2. O editor oficial não regista o asset source
  do `bevy_ext_gltf` (teria de ser antes do `AssetPlugin`); por isso os GLB do
  pool podem falhar no viewport. O preview voxel não depende deles.
- Grip da arma e preview de terreno/composições/UI continuam fora.

Detalhe: `editor/README.md` (legado), `docs/MIGRATION.md` e o README de cada
extensão em `../bevy_extensions/extensions/`.
