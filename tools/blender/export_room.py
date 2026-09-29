"""Export a room's modern visuals for the engine (docs/REMAKE.md §4, step 5).

    blender --background <store>/rooms/r100/r100.blend --python export_room.py

Writes `<room folder>/visual.glb` from the modern collections only, never
`reference` or the locked gameplay collections. glTF 2.0 with:
- object custom properties as node extras (`lightmap`, `source`, …);
- lights as KHR_lights_punctual;
- every UV map, so the `lightmap` atlas arrives as TEXCOORD_1.

Then writes the room manifest `room.ron` (schema 1) that the engine reads
from the content store.
"""

import os

import bpy

EXPORTED = [
    "architecture", "props_static", "props_dynamic", "doors", "destructibles",
    "decals", "lights", "reflection_probes", "gi_probes", "fog_volumes",
]
SCHEMA = 1


def main():
    room_dir = os.path.dirname(bpy.data.filepath)
    room = bpy.context.scene.get("room") or os.path.basename(room_dir)
    cols = bpy.data.collections
    for c in cols:
        c.hide_select = False
    bpy.ops.object.select_all(action="DESELECT")
    objs = [o for name in EXPORTED if name in cols for o in cols[name].all_objects]
    for o in objs:
        o.hide_set(False)
        o.select_set(True)
    if not objs:
        raise SystemExit("export_room: the modern collections are empty")
    # Materials sample the first UV map; the second is the lightmap atlas.
    for o in objs:
        if o.type == "MESH" and o.data.uv_layers:
            o.data.uv_layers.active_index = 0
            o.data.uv_layers[0].active_render = True

    path = os.path.join(room_dir, "visual.glb")
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_extras=True,
        export_lights=True,
        export_yup=True,
        export_apply=False,
        export_texcoords=True,
        export_normals=True,
        export_cameras=False,
        # Vertex colours on a stand-in are the original's baked lighting;
        # the engine lights with the bake instead.
        export_vertex_color="NONE",
        export_all_vertex_colors=False,
    )
    lightmapped = sum(1 for o in objs if "lightmap" in o)
    lights = sum(1 for o in objs if o.type == "LIGHT")
    with open(os.path.join(room_dir, "room.ron"), "w", encoding="utf-8") as f:
        f.write(
            "// Written by tools/blender/export_room.py; the engine reads it (ADR-013).\n"
            f"(\n    schema: {SCHEMA},\n    room: \"{room}\",\n    visual: \"visual.glb\",\n"
            # Blender's exporter turns a watt into 683 lm and a Cycles diffuse
            # bake stores E/pi in watts, so the bake matches the lights at 683.
            "    lightmap_exposure: 683.0,\n)\n"
        )
    print(f"export_room: {path}: {len(objs)} objects, {lightmapped} lightmapped, {lights} lights")


main()
