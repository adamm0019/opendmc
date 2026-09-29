"""Render a room scene through its original cameras (docs/REMAKE.md §4).

    blender --background <room>.blend --python render_cameras.py -- \
        --out <dir> [--cameras 1,4,7] [--layer reference|modern] [--engine workbench|eevee|cycles]

`--layer reference` renders the locked original geometry (flat textured, for
checking composition against the engine); `--layer modern` renders the
modern collections as authored. Images go to `<dir>/<layer>_camNN.png`.
"""

import argparse
import os
import sys

import bpy

MODERN = [
    "architecture", "props_static", "props_dynamic", "doors", "destructibles",
    "decals", "lights", "reflection_probes", "gi_probes", "fog_volumes",
]


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser()
    p.add_argument("--out", required=True)
    p.add_argument("--cameras", default="")
    p.add_argument("--layer", default="reference", choices=["reference", "modern"])
    p.add_argument("--engine", default="workbench", choices=["workbench", "eevee", "cycles"])
    p.add_argument("--samples", type=int, default=64)
    p.add_argument("--view", default="Filmic", help="view transform (Filmic matches the engine)")
    return p.parse_args(argv)


def main():
    a = args()
    scene = bpy.context.scene
    cols = bpy.data.collections
    for c in cols:
        c.hide_render = True
    if a.layer == "reference":
        cols["reference"].hide_render = False
        if "props_sheet" in cols:
            cols["props_sheet"].hide_render = True
    else:
        for name in MODERN:
            if name in cols:
                cols[name].hide_render = False

    if a.engine == "workbench":
        scene.render.engine = "BLENDER_WORKBENCH"
        scene.display.shading.light = "FLAT"
        scene.display.shading.color_type = "TEXTURE"
    elif a.engine == "eevee":
        scene.render.engine = "BLENDER_EEVEE_NEXT"
    else:
        scene.render.engine = "CYCLES"
        scene.cycles.samples = a.samples
        scene.cycles.use_denoising = True

    scene.view_settings.view_transform = a.view
    wanted = {int(x) for x in a.cameras.split(",") if x}
    cams = sorted((o for o in cols["cameras"].objects if o.type == "CAMERA"),
                  key=lambda o: o["index"])
    os.makedirs(a.out, exist_ok=True)
    for cam in cams:
        if wanted and cam["index"] not in wanted:
            continue
        scene.camera = cam
        scene.render.filepath = os.path.join(os.path.abspath(a.out), f"{a.layer}_{a.view.lower()}_cam{cam['index']:02}.png")
        bpy.ops.render.render(write_still=True)
        print("render_cameras:", scene.render.filepath)


main()
