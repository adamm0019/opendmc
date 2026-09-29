"""Bake a room's static lighting in Cycles (docs/REMAKE.md §4, step 4).

    blender --background <store>/rooms/r100/r100.blend --python bake_room.py -- \
        [--size 2048] [--samples 256] [--standin] [--no-save]

- `--standin` (Milestone B0 only): if `architecture` is empty, fill it with
  copies of the locked reference geometry, so the pipeline can be proven
  before any modern architecture exists. Copies are tagged `standin`.
- Gives every mesh in the lightmapped collections a shared `lightmap` UV
  atlas, bakes Cycles diffuse light (direct + indirect, no albedo: the engine
  multiplies by the material's colour) from the lights in the scene, and
  writes `lightmaps/lightmap0.ktx2` (RGBA16F) with a preview PNG beside it.
- Tags each lightmapped object with `lightmap` (the file, relative to the
  room folder) so the export carries it to the engine as a glTF extra.
"""

import argparse
import os
import sys

import bpy
import numpy as np

sys.path.append(os.path.dirname(os.path.abspath(__file__)))
import ktx2  # noqa: E402

LIGHTMAPPED = ["architecture", "props_static"]
UV = "lightmap"


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser()
    p.add_argument("--size", type=int, default=2048)
    p.add_argument("--samples", type=int, default=256)
    p.add_argument("--standin", action="store_true")
    p.add_argument("--no-save", action="store_true")
    return p.parse_args(argv)


def use_gpu(scene):
    prefs = bpy.context.preferences.addons["cycles"].preferences
    for kind in ("OPTIX", "CUDA", "HIP", "ONEAPI"):
        try:
            prefs.compute_device_type = kind
        except TypeError:
            continue
        prefs.get_devices()
        devices = [d for d in prefs.devices if d.type == kind]
        # Skip integrated GPUs when a discrete one is present.
        discrete = [d for d in devices if "(TM) Graphics" not in d.name]
        for d in prefs.devices:
            d.use = d in (discrete or devices)
        if devices:
            scene.cycles.device = "GPU"
            print("bake_room: GPU", kind, [d.name for d in discrete or devices])
            return
    scene.cycles.device = "CPU"
    print("bake_room: CPU")


def standin(cols):
    if any(o.type == "MESH" for o in cols["architecture"].objects):
        return 0
    n = 0
    for o in cols["reference"].objects:
        if o.type != "MESH":
            continue
        copy = o.copy()
        copy.data = o.data.copy()
        copy.name = f"standin_{o.name}"
        copy["standin"] = True
        cols["architecture"].objects.link(copy)
        n += 1
    return n


def neutral_standins(cols):
    """The glTF importer multiplies each material's colour by the vertex
    colours, which on the reference are the original's baked lighting. The
    stand-ins are relit here, so their copies of those colours go white
    (the locked reference keeps its own)."""
    n = 0
    for o in cols["architecture"].objects:
        if o.type != "MESH" or not o.get("standin"):
            continue
        for attr in o.data.color_attributes:
            values = np.ones(len(attr.data) * 4, dtype=np.float32)
            attr.data.foreach_set("color", values)
        n += 1
    return n


def lightmap_uvs(objs):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        if UV not in o.data.uv_layers:
            o.data.uv_layers.new(name=UV)
        # Edit the lightmap layer; materials keep sampling the first one.
        o.data.uv_layers.active = o.data.uv_layers[UV]
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=1.15, island_margin=0.0, scale_to_bounds=False)
    bpy.ops.uv.average_islands_scale()
    bpy.ops.uv.pack_islands(margin=0.002, rotate=True)
    bpy.ops.object.mode_set(mode="OBJECT")


def main():
    a = args()
    scene = bpy.context.scene
    cols = bpy.data.collections
    room_dir = os.path.dirname(bpy.data.filepath)

    if a.standin:
        print("bake_room: stand-in copies", standin(cols))
    print("bake_room: neutral stand-ins", neutral_standins(cols))
    objs = [o for name in LIGHTMAPPED if name in cols for o in cols[name].objects if o.type == "MESH"]
    if not objs:
        raise SystemExit("bake_room: nothing to bake (architecture and props_static are empty)")

    for c in cols:
        c.hide_select = False
    lightmap_uvs(objs)

    image = bpy.data.images.new("lightmap0", a.size, a.size, alpha=False, float_buffer=True)
    materials = {slot.material for o in objs for slot in o.material_slots if slot.material}
    for m in materials:
        m.use_nodes = True
        nodes = m.node_tree.nodes
        node = nodes.get("bake_target") or nodes.new("ShaderNodeTexImage")
        node.name = "bake_target"
        node.image = image
        nodes.active = node

    scene.render.engine = "CYCLES"
    use_gpu(scene)
    scene.cycles.samples = a.samples
    scene.render.bake.margin = 4
    scene.render.bake.use_pass_direct = True
    scene.render.bake.use_pass_indirect = True
    scene.render.bake.use_pass_color = False
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    print(f"bake_room: baking {len(objs)} objects into {a.size}x{a.size}, {a.samples} samples")
    bpy.ops.object.bake(type="DIFFUSE", pass_filter={"DIRECT", "INDIRECT"}, margin=4, use_clear=True)

    # Blender images start at the bottom row; KTX2 and the engine at the top.
    px = np.array(image.pixels[:], dtype=np.float32).reshape(a.size, a.size, 4)[::-1]
    px[:, :, 3] = 1.0
    out_dir = os.path.join(room_dir, "lightmaps")
    os.makedirs(out_dir, exist_ok=True)
    rel = "lightmaps/lightmap0.ktx2"
    ktx2.write(os.path.join(room_dir, rel), [px])
    preview = bpy.data.images.new("lightmap0_preview", a.size, a.size, alpha=False)
    tone = px[::-1].copy()
    tone[:, :, :3] = tone[:, :, :3] / (1.0 + tone[:, :, :3])
    preview.pixels[:] = tone.ravel()
    preview.filepath_raw = os.path.join(out_dir, "lightmap0_preview.png")
    preview.file_format = "PNG"
    preview.save()

    for o in objs:
        o["lightmap"] = rel
        o.data.uv_layers.active = o.data.uv_layers[0]
    print("bake_room: wrote", os.path.join(room_dir, rel),
          "mean", px[:, :, :3].mean(axis=(0, 1)), "max", px[:, :, :3].max())
    if not a.no_save:
        bpy.ops.wm.save_mainfile()


main()
