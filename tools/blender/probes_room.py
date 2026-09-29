"""Bake a room's light probes in Cycles (docs/REMAKE.md §4, step 4).

    blender --background <store>/rooms/r100/r100.blend --python probes_room.py -- \
        [--defaults] [--face 256] [--samples 64] [--spacing 1.5]

Probes are empties (display: cube of size 0.5, so an empty's scale is the
box's full size, the same as the engine's unit-cube light probes) in:
- `reflection_probes`: a cubemap is rendered from the box centre (or from
  its `capture_height` above the box floor) and written to
  `probes/<name>.ktx2`, six 90° Cycles faces laid out as the engine samples
  them;
- `gi_probes`: an irradiance volume of ambient cubes, voxels `spacing`
  apart. It is baked in one Cycles pass from a tiny cube per voxel, invisible
  to every ray, whose six faces catch the indirect light from each direction.
  Direct light reaches actors from the engine's dynamic lights instead. It is
  written as the engine's (Rx, 2Ry, 3Rz) 3D texture;
- `fog_volumes`: nothing to bake; their `density` and `colour` properties
  travel as extras.

`--defaults` (Milestone B0) adds one of each around the architecture when a
collection is empty. Each probe is tagged with its file for the export.
"""

import argparse
import math
import os
import sys
import tempfile

import bpy
import numpy as np
from mathutils import Matrix, Vector

sys.path.append(os.path.dirname(os.path.abspath(__file__)))
import ktx2  # noqa: E402

# Engine space (Y up) to Blender space (Z up): (x, y, z) -> (x, -z, y).
E2B = Matrix(((1, 0, 0), (0, 0, -1), (0, 1, 0)))
MODERN = ["architecture", "props_static", "props_dynamic", "doors", "destructibles", "decals", "lights"]
# Reflection cubemap texels are clamped to this radiance (Blender units).
MAX_PROBE_RADIANCE = 50.0


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser()
    p.add_argument("--defaults", action="store_true")
    p.add_argument("--face", type=int, default=256)
    p.add_argument("--samples", type=int, default=64)
    p.add_argument("--spacing", type=float, default=1.5)
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
        discrete = [d for d in devices if "(TM) Graphics" not in d.name]
        for d in prefs.devices:
            d.use = d in (discrete or devices)
        if devices:
            scene.cycles.device = "GPU"
            return
    scene.cycles.device = "CPU"


def only_modern_renders():
    for c in bpy.data.collections:
        c.hide_render = c.name not in MODERN


def architecture_bounds():
    """1st to 99th percentile of the architecture's vertices, Blender space."""
    pts = []
    for o in bpy.data.collections["architecture"].objects:
        if o.type != "MESH":
            continue
        co = np.empty(len(o.data.vertices) * 3, dtype=np.float32)
        o.data.vertices.foreach_get("co", co)
        m = np.array(o.matrix_world)
        pts.append(co.reshape(-1, 3) @ m[:3, :3].T + m[:3, 3])
    p = np.concatenate(pts)
    return np.percentile(p, 1, axis=0), np.percentile(p, 99, axis=0)


def box_empty(name, collection, lo, hi):
    e = bpy.data.objects.new(name, None)
    e.empty_display_type = "CUBE"
    e.empty_display_size = 0.5
    e.location = Vector((lo + hi) / 2)
    e.scale = Vector(hi - lo)
    bpy.data.collections[collection].objects.link(e)
    return e


def add_defaults():
    lo, hi = architecture_bounds()
    made = []
    if not bpy.data.collections["reflection_probes"].objects:
        e = box_empty("probe_hall", "reflection_probes", lo, hi)
        e["capture_height"] = 2.0
        made.append(e.name)
    if not bpy.data.collections["gi_probes"].objects:
        made.append(box_empty("gi_hall", "gi_probes", lo, hi).name)
    if not bpy.data.collections["fog_volumes"].objects:
        # Low-lying haze over the floor: the lower third of the hall.
        top = lo.copy()
        top[2] = lo[2] + (hi[2] - lo[2]) / 3
        e = box_empty("fog_hall", "fog_volumes", lo, np.array([hi[0], hi[1], top[2]]))
        e["fog_volume"] = True
        e["density"] = 0.03
        e["colour"] = [0.95, 0.88, 0.75]
        made.append(e.name)
    return made


def engine_box(e):
    """An empty's box in engine space: centre and full size."""
    c = E2B.transposed() @ e.location
    s = e.scale
    return np.array(c), np.array([s.x, s.z, s.y])


# Cube faces in the engine's sampling order (+X, -X, +Y, -Y, +Z, -Z), as
# (forward, up) in engine space. The engine samples a cubemap at (x, y, -z),
# so faces +Z/-Z look along -Z/+Z.
FACES = [((1, 0, 0), (0, 1, 0)), ((-1, 0, 0), (0, 1, 0)), ((0, 1, 0), (0, 0, 1)),
         ((0, -1, 0), (0, 0, -1)), ((0, 0, -1), (0, 1, 0)), ((0, 0, 1), (0, 1, 0))]


def render_exr(scene, path):
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    img = bpy.data.images.load(path)
    w, h = img.size
    px = np.array(img.pixels[:], dtype=np.float32).reshape(h, w, 4)[::-1].copy()
    bpy.data.images.remove(img)
    os.remove(path)
    return px


def bake_reflection(scene, e, a, room_dir, tmp):
    centre, size = engine_box(e)
    at = centre.copy()
    if "capture_height" in e:
        at[1] = centre[1] - size[1] / 2 + float(e["capture_height"])
    data = bpy.data.cameras.new("probe_camera")
    data.sensor_fit = "VERTICAL"
    data.angle = math.pi / 2
    data.clip_start = 0.05
    cam = bpy.data.objects.new("probe_camera", data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    scene.render.resolution_x = scene.render.resolution_y = a.face
    faces = []
    for forward, up in FACES:
        f, u = Vector(forward), Vector(up)
        r = f.cross(u)
        cols = [E2B @ r, E2B @ u, -(E2B @ f)]
        cam.matrix_world = Matrix.Translation(E2B @ Vector(at)) @ Matrix(
            [[cols[j][i] for j in range(3)] for i in range(3)]).to_4x4()
        faces.append(render_exr(scene, os.path.join(tmp, "face.exr")))
    bpy.data.objects.remove(cam)
    rel = f"probes/{e.name}.ktx2"
    # Lights seen directly are tiny, blinding spots; clamp them so the
    # runtime prefilter spreads a highlight, not a firefly.
    faces = [np.dstack([np.minimum(f[:, :, :3], MAX_PROBE_RADIANCE), np.ones(f.shape[:2])]) for f in faces]
    ktx2.write(os.path.join(room_dir, rel), faces)
    e["reflection_probe"] = rel
    return rel, [round(float(f[:, :, :3].mean()), 3) for f in faces]


def bake_irradiance(scene, e, a, room_dir):
    centre, size = engine_box(e)
    res = [max(2, min(32, int(math.ceil(s / a.spacing)))) for s in size]
    rx, ry, rz = res
    lo = centre - size / 2
    step = size / np.array(res)
    h = 0.02
    cell, n_quads = 4, rx * ry * rz * 6
    grid = int(math.ceil(math.sqrt(n_quads)))
    atlas = grid * cell
    verts, faces, uvs, keys = [], [], [], []
    for z in range(rz):
        for y in range(ry):
            for x in range(rx):
                c = lo + (np.array([x, y, z]) + 0.5) * step
                for axis in range(3):
                    for sign in (1, -1):
                        n = np.zeros(3)
                        n[axis] = sign
                        t1, t2 = np.zeros(3), np.zeros(3)
                        t1[(axis + 1) % 3], t2[(axis + 2) % 3] = h, h
                        if sign < 0:
                            t1, t2 = t2, t1
                        base = c + n * h
                        quad = [base - t1 - t2, base + t1 - t2, base + t1 + t2, base - t1 + t2]
                        q = len(keys)
                        i0 = len(verts)
                        verts += [tuple(E2B @ Vector(p)) for p in quad]
                        faces.append((i0, i0 + 1, i0 + 2, i0 + 3))
                        cx, cy = (q % grid) * cell, (q // grid) * cell
                        uvs += [((cx + 0.5) / atlas, (cy + 0.5) / atlas), ((cx + 3.5) / atlas, (cy + 0.5) / atlas),
                                ((cx + 3.5) / atlas, (cy + 3.5) / atlas), ((cx + 0.5) / atlas, (cy + 3.5) / atlas)]
                        keys.append((x, y, z, axis, sign))
    mesh = bpy.data.meshes.new("gi_cubes")
    mesh.from_pydata(verts, [], faces)
    uv = mesh.uv_layers.new(name="gi")
    for loop, (u, v) in zip(uv.data, uvs):
        loop.uv = (u, v)
    obj = bpy.data.objects.new("gi_cubes", mesh)
    scene.collection.objects.link(obj)
    # Cycles only bakes surfaces camera rays can see; every other ray passes
    # through them, so they light and shadow nothing.
    for flag in ("visible_diffuse", "visible_glossy", "visible_shadow",
                 "visible_transmission", "visible_volume_scatter"):
        setattr(obj, flag, False)
    image = bpy.data.images.new("gi_atlas", atlas, atlas, alpha=False, float_buffer=True)
    mat = bpy.data.materials.new("gi_bake")
    mat.use_nodes = True
    node = mat.node_tree.nodes.new("ShaderNodeTexImage")
    node.image = image
    mat.node_tree.nodes.active = node
    mesh.materials.append(mat)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    scene.render.bake.margin = 1
    bpy.ops.object.bake(type="DIFFUSE", pass_filter={"INDIRECT"}, margin=1, use_clear=True)

    px = np.array(image.pixels[:], dtype=np.float32).reshape(atlas, atlas, 4)
    vol = np.zeros((3 * rz, 2 * ry, rx, 4), dtype=np.float32)
    vol[..., 3] = 1.0
    for q, (x, y, z, axis, sign) in enumerate(keys):
        cx, cy = (q % grid) * cell, (q // grid) * cell
        # Blender image rows start at the bottom, as UV v does.
        value = px[cy + 1:cy + 3, cx + 1:cx + 3, :3].mean(axis=(0, 1))
        # The engine reads the negative side from the second half in y.
        vol[z + axis * rz, y + (0 if sign > 0 else ry), x, :3] = value
    bpy.data.objects.remove(obj)
    bpy.data.meshes.remove(mesh)
    bpy.data.images.remove(image)
    rel = f"probes/{e.name}.ktx2"
    ktx2.write_3d(os.path.join(room_dir, rel), vol)
    e["irradiance_volume"] = rel
    return rel, res, float(vol[..., :3].mean())


def main():
    a = args()
    scene = bpy.context.scene
    room_dir = os.path.dirname(bpy.data.filepath)
    os.makedirs(os.path.join(room_dir, "probes"), exist_ok=True)
    scene.render.engine = "CYCLES"
    use_gpu(scene)
    scene.cycles.samples = a.samples
    scene.cycles.use_denoising = True
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"
    only_modern_renders()
    if a.defaults:
        print("probes_room: added", add_defaults())
    saved = (scene.camera, scene.render.resolution_x, scene.render.resolution_y)
    with tempfile.TemporaryDirectory() as tmp:
        for e in bpy.data.collections["reflection_probes"].objects:
            print("probes_room: reflection", bake_reflection(scene, e, a, room_dir, tmp))
    for e in bpy.data.collections["gi_probes"].objects:
        print("probes_room: irradiance volume", *bake_irradiance(scene, e, a, room_dir))
    scene.camera, scene.render.resolution_x, scene.render.resolution_y = saved
    scene.render.image_settings.file_format = "PNG"
    if not a.no_save:
        bpy.ops.wm.save_mainfile()


main()
