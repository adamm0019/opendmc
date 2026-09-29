"""Build a room's Blender scene from its reference exports (docs/REMAKE.md §4).

    blender --background --python reference_room.py -- --room-dir <store>/rooms/r100

Reads `<room-dir>/reference/<room>.glb` (+ `.collision.glb`, `.cameras.json`,
`.triggers.json`, `.props.glb`), all written by `dmc room`, and saves
`<room-dir>/<room>.blend` in metres (ADR-010) with:

- locked, unrenderable collections holding the original's data:
  `reference` (its visual geometry), `gameplay_collision`, `cameras` (one
  Blender camera per original camera, framed as the engine frames it for a
  player standing in the middle of its zone, plus its zone and rails) and
  `gameplay_triggers` (trigger volumes, doors tagged with their target room);
- the empty modern collections the export picks up: `architecture`,
  `props_static`, `props_dynamic`, `doors`, `destructibles`, `decals`,
  `lights`, `reflection_probes`, `gi_probes`, `fog_volumes`, `audio_zones`.

Everything here is derived from the user's own files, so the .blend stays in
the private content store (policy rule 6, ADR-009).
"""

import argparse
import json
import math
import os
import sys

import bpy
from mathutils import Matrix, Vector

# ADR-010; the same constant as dmc_sim::world::ROOM_UNITS_PER_METRE.
ROOM_UNITS_PER_METRE = 450.0

LOCKED = ["reference", "gameplay_collision", "cameras", "gameplay_triggers"]
MODERN = [
    "architecture", "props_static", "props_dynamic", "doors", "destructibles",
    "decals", "lights", "reflection_probes", "gi_probes", "fog_volumes",
    "audio_zones",
]
HAS_RAILS = 0x10  # camera flags bit (dmc_formats::camera::HAS_RAILS)


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser()
    p.add_argument("--room-dir", required=True)
    p.add_argument("--out")
    return p.parse_args(argv)


def to_blender(v):
    """Room units, Y up (as stored and as glTF) -> metres, Z up."""
    x, y, z = v[0], v[1], v[2]
    s = 1.0 / ROOM_UNITS_PER_METRE
    return Vector((x * s, -z * s, y * s))


def dir_to_blender(v):
    return Vector((v[0], -v[2], v[1]))


def collection(name, parent=None):
    c = bpy.data.collections.new(name)
    (parent or bpy.context.scene.collection).children.link(c)
    return c


def import_glb(path, into):
    """Import a glTF into `into`, scaled from room units to metres."""
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    scale = Matrix.Scale(1.0 / ROOM_UNITS_PER_METRE, 4)
    for o in new:
        for c in list(o.users_collection):
            c.objects.unlink(o)
        into.objects.link(o)
        if o.parent is None:
            o.matrix_world = scale @ o.matrix_world
    bpy.ops.object.select_all(action="DESELECT")
    for o in new:
        if o.type == "MESH":
            o.select_set(True)
    if any(o.type == "MESH" for o in new):
        bpy.context.view_layer.objects.active = next(o for o in new if o.type == "MESH")
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return new


def convex_mesh(name, corners, normals, into):
    """A convex volume given as in the room files: corners A and B, six
    outward normals, the first three through B and the last three through A
    (docs/formats/README.md §4e). Built from its eight plane intersections."""
    planes = []
    for i, n in enumerate(normals):
        n = dir_to_blender(n)
        o = to_blender(corners[1] if i < 3 else corners[0])
        planes.append((n, n.dot(o)))
    # Pair each face with its most opposite one.
    pairs, used = [], set()
    for i in range(6):
        if i in used:
            continue
        j = min((k for k in range(6) if k != i and k not in used),
                key=lambda k: planes[i][0].dot(planes[k][0]))
        pairs.append((i, j))
        used |= {i, j}
    verts = []
    for a in pairs[0]:
        for b in pairs[1]:
            for c in pairs[2]:
                m = Matrix((planes[a][0], planes[b][0], planes[c][0]))
                try:
                    verts.append(m.inverted() @ Vector((planes[a][1], planes[b][1], planes[c][1])))
                except ValueError:
                    return None
    faces = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata([tuple(v) for v in verts], [], faces)
    obj = bpy.data.objects.new(name, mesh)
    obj.display_type = "WIRE"
    into.objects.link(obj)
    return obj


def polyline(name, points, into):
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    spline = curve.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for p, q in zip(spline.points, points):
        p.co = (*to_blender(q), 1.0)
    obj = bpy.data.objects.new(name, curve)
    into.objects.link(obj)
    return obj


def length(path):
    return sum((path[i + 1] - path[i]).length for i in range(len(path) - 1))


def nearest_fraction(path, q):
    total = length(path)
    if total <= 0:
        return 0.0
    best, best_d, walked = 0.0, math.inf, 0.0
    for a, b in zip(path, path[1:]):
        ab = b - a
        l2 = ab.dot(ab)
        t = min(max((q - a).dot(ab) / l2, 0.0), 1.0) if l2 > 0 else 0.0
        d = (a + ab * t - q).length
        if d < best_d:
            best_d, best = d, (walked + math.sqrt(l2) * t) / total
        walked += math.sqrt(l2)
    return best


def at_fraction(path, f):
    total = length(path)
    if len(path) < 2 or total <= 0:
        return path[0]
    left = min(max(f, 0.0), 1.0) * total
    for a, b in zip(path, path[1:]):
        seg = (b - a).length
        if left <= seg:
            return a + (b - a) * (left / seg if seg > 0 else 0.0)
        left -= seg
    return path[-1]


def add_cameras(cams, into):
    """One camera per original camera, framed as `opendmc::room_cameras`
    frames it for a player on the floor in the middle of its zone."""
    for i, c in enumerate(cams):
        lo = [min(a, b) for a, b in zip(*c["zone_corners"])]
        hi = [max(a, b) for a, b in zip(*c["zone_corners"])]
        player = [(lo[0] + hi[0]) / 2, lo[1], (lo[2] + hi[2]) / 2]
        target = to_blender([p + o for p, o in zip(player, c["look_offset"])])
        rails = c["flags"] & HAS_RAILS and len(c["eye_rail"]) >= 2
        if rails:
            eye_rail = [to_blender(p) for p in c["eye_rail"]]
            track_rail = [to_blender(p) for p in c["track_rail"]]
            eye = at_fraction(eye_rail, nearest_fraction(track_rail, target))
        elif any(c["eye"]):
            eye = to_blender(c["eye"])
        else:
            eye = target + Vector((0.0, 7.0, 4.0))
        fov = c["fov"] if 1.0 < c["fov"] < 170.0 else 55.0

        data = bpy.data.cameras.new(f"cam{i:02}")
        data.sensor_fit = "VERTICAL"
        data.angle = math.radians(fov)
        data.clip_start, data.clip_end = 0.05, 200.0
        cam = bpy.data.objects.new(f"cam{i:02}", data)
        cam.location = eye
        cam.rotation_euler = (target - eye).to_track_quat("-Z", "Y").to_euler()
        cam["index"] = i
        cam["fov"] = fov
        cam["rails"] = bool(rails)
        into.objects.link(cam)

        zone = convex_mesh(f"cam{i:02}_zone", c["zone_corners"], c["zone_normals"], into)
        if zone is not None:
            zone.parent = None
        if rails:
            polyline(f"cam{i:02}_eye_rail", c["eye_rail"], into)
            polyline(f"cam{i:02}_track_rail", c["track_rail"], into)


def add_triggers(triggers, into):
    for t in triggers:
        name = f"trigger{t['index']:02}_k{t['kind']}_s{t['sub']}"
        room = t["room"]
        if t["sub"] == 0 and room:
            name = f"door{t['index']:02}_to_r{room:03x}"
        vol = t["volume"]
        if "Box" in vol:
            obj = convex_mesh(name, vol["Box"]["corners"], vol["Box"]["normals"], into)
        else:
            r = vol["Round"]
            bpy.ops.mesh.primitive_cylinder_add(
                radius=r["size"][0] / ROOM_UNITS_PER_METRE,
                depth=max(r["size"][1], 1.0) / ROOM_UNITS_PER_METRE,
                location=to_blender(r["centre"]) + Vector((0, 0, r["size"][1] / ROOM_UNITS_PER_METRE / 2)),
            )
            obj = bpy.context.active_object
            obj.name = name
            for c in list(obj.users_collection):
                c.objects.unlink(obj)
            into.objects.link(obj)
            obj.display_type = "WIRE"
        if obj is None:
            continue
        obj["kind"], obj["sub"], obj["room"] = t["kind"], t["sub"], room
        if t["sub"] == 0 and room:
            obj["target_room"] = f"r{room:03x}"
            obj["arrival_room_units"] = list(t["point"])


def main():
    a = args()
    room_dir = os.path.abspath(a.room_dir)
    room = os.path.basename(room_dir.rstrip("/\\"))
    ref = os.path.join(room_dir, "reference")
    out = os.path.abspath(a.out or os.path.join(room_dir, f"{room}.blend"))

    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.name = room
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.scale_length = 1.0
    scene.render.engine = "CYCLES"
    scene.render.resolution_x, scene.render.resolution_y = 1280, 720

    cols = {name: collection(name) for name in LOCKED + MODERN}
    import_glb(os.path.join(ref, f"{room}.glb"), cols["reference"])
    col_path = os.path.join(ref, f"{room}.collision.glb")
    if os.path.exists(col_path):
        for o in import_glb(col_path, cols["gameplay_collision"]):
            o.display_type = "WIRE"
    cam_path = os.path.join(ref, f"{room}.cameras.json")
    if os.path.exists(cam_path):
        add_cameras(json.load(open(cam_path))["cameras"], cols["cameras"])
    trig_path = os.path.join(ref, f"{room}.triggers.json")
    if os.path.exists(trig_path):
        add_triggers(json.load(open(trig_path))["triggers"], cols["gameplay_triggers"])
    props_path = os.path.join(ref, f"{room}.props.glb")
    if os.path.exists(props_path):
        sheet = collection("props_sheet", cols["reference"])
        import_glb(props_path, sheet)

    for name in LOCKED:
        cols[name].hide_select = True
        cols[name].hide_render = True
    # The props sheet lays props side by side, not in place: keep it out of
    # the way unless asked for.
    if "props_sheet" in bpy.data.collections:
        layer = bpy.context.view_layer.layer_collection.children["reference"].children["props_sheet"]
        layer.exclude = True

    cams = [o for o in cols["cameras"].objects if o.type == "CAMERA"]
    if cams:
        scene.camera = cams[0]
    scene["room"] = room
    scene["room_units_per_metre"] = ROOM_UNITS_PER_METRE
    bpy.ops.wm.save_as_mainfile(filepath=out)
    print(f"reference_room: {out}: {len(cams)} cameras, "
          f"{len(cols['gameplay_triggers'].objects)} triggers, "
          f"{len(cols['reference'].objects)} reference objects")


main()
