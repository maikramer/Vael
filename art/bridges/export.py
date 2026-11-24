"""Export the visible bridge assemblies from Travessias.blend for the Vael LOD pipeline."""

from pathlib import Path

import bpy

MODELS = {
    "Passadeira da Bruxa": "passadeira_da_bruxa",
    "Ponte do Ermo": "ponte_do_ermo",
    "Ponte Norte": "ponte_norte",
}


def main():
    directory = Path(__file__).resolve().parents[2] / "assets/public/assets/meshes/infra"
    for title, filename in MODELS.items():
        bpy.context.window.scene = bpy.data.scenes[title]
        collection = bpy.data.collections[f"Modelo • {title}"]
        bpy.ops.object.select_all(action="DESELECT")
        for member in collection.objects:
            bpy.context.view_layer.objects.active = member
            for modifier in list(member.modifiers):
                bpy.ops.object.modifier_apply(modifier=modifier.name)
            member.select_set(True)

        # The LOD generator simplifies one mesh. Keep planks, supports and rails
        # together in the export, with separate primitives for their materials.
        bpy.context.view_layer.objects.active = next(iter(collection.objects))
        bpy.ops.object.join()
        bridge = bpy.context.object
        bridge.name = bridge.data.name = title
        bpy.ops.export_scene.gltf(
            filepath=str(directory / f"{filename}_lod0.glb"),
            export_format="GLB",
            use_active_scene=True,
            use_selection=True,
            export_apply=True,
            export_animations=False,
        )


if __name__ == "__main__":
    main()
