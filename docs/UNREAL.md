# Moving OpenDMC to Unreal Engine 5.8

*Decided 2026-09-29 (ADR-014). This document is the transfer plan: what moves,
what stays, and how the two halves meet.*

## Contents
1. What moves and what stays
2. Repositories and folders
3. The export bundle (`dmc unreal`)
4. Rendering in Unreal
5. Porting the sim
6. Rooms in Unreal: cameras, doors, collision, lights
7. What is frozen or dropped
8. Status and next steps

---

## 1. What moves and what stays

The requirement in `docs/REMAKE.md` is unchanged: a faithful modern
graphical remake that preserves layouts, collision, doors and triggers,
camera compositions, silhouettes, colour identity, timing and hitboxes, and
rebuilds everything visual. Only the engine changes.

| Part | Before | After |
|---|---|---|
| Game shell, rendering | `opendmc` (Bevy 0.18) | the `OpenDMC-UE` Unreal 5.8 project |
| Gameplay core | `dmc-sim` (Rust) | a C++ port inside `OpenDMC-UE`, checked against `dmc-sim` (§5) |
| Reading the install | `dmc-formats`, `dmc` | unchanged, plus `dmc unreal` (§3) |
| Authored data (moves, rules) | RON in `data/` | stays authoritative here; exported as JSON for the port (§5) |
| Rebuilt art | Blender → glTF/KTX2 → Bevy | Blender → glTF/FBX → Unreal assets (§2) |
| Lighting | Cycles bakes, irradiance volumes, Solari trial | Lumen, MegaLights, virtual shadow maps (§4) |
| Policies | clean room, never commit game data | unchanged |

## 2. Repositories and folders

Everything lives side by side in `C:\Users\adam-\Downloads`:

| Folder | Git | Holds |
|---|---|---|
| `opendmc` (and its worktrees) | this repo, pushed | the toolchain: `dmc-formats`, `dmc`, `dmc-sim` (reference), the frozen Bevy shell, the Blender kit, the docs |
| `OpenDMC-UE` | new repo, Git LFS | the Unreal project: C++ source, config, import scripts, our own authored Unreal assets, golden traces |
| `opendmc-unreal-bundle` | none | the export bundle from the user's install (§3); game data, never committed |
| `opendmc-content` | private, local only | rebuilt likeness art and its Blender sources (ADR-009) |

Inside `OpenDMC-UE`:

```
OpenDMC.uproject
Config/                    engine, game and editor settings (§4)
Source/OpenDMC/            runtime module
    Sim/                   the sim port (§5)
    Rooms/                 room actors, camera director, doors (§6)
Content/OpenDMC/           our own assets: maps, materials, blueprints (committed, LFS)
Content/Imported/          generated from the bundle by Scripts/ (ignored: game data)
Plugins/OpenDMCArt/        rebuilt likeness art, a content-only plugin linked in
                           from opendmc-content (ignored)
Scripts/                   Python import scripts (UnrealEditor-Cmd -run=pythonscript)
Tests/Golden/              golden traces from dmc-sim (§5)
```

The game-data rule carries over unchanged. Nothing imported from the
install, and no rebuilt likeness art, is ever committed. `.gitignore` covers
`Content/Imported/` and `Plugins/OpenDMCArt/`. The import scripts rebuild
`Content/Imported/` from the bundle on any machine that has the game.

## 3. The export bundle (`dmc unreal`)

```
dmc unreal "<install>\data\dmc1\dmc1-0.nbz" "C:\Users\adam-\Downloads\opendmc-unreal-bundle"
```

It takes about 15 s and writes about 1.5 GB:

```
manifest.json
rooms/<room>/<room>.glb             the room's meshes, one node per room object
             <room>.collision.glb   collision polygons (§4d of the format notes)
             <room>.props.glb       props, where the room has them (§4f)
             <room>.cameras.json    cameras: zones, eye and track rails, FOV (§4e)
             <room>.triggers.json   triggers and doors (§4g)
             <room>.lights.json     every light set of sections 4 and 5 (§4h)
models/<Dir>/<model>.glb            every model, skinned, with its motions
```

It holds all 106 rooms and 73 models. Section numbers refer to
`docs/formats/README.md`.

**Units and axes.** Meshes and JSON share one space: the game's room units,
glTF axes (right-handed, +Y up). 450 room units make a metre (ADR-010),
which is 100/450 ≈ 0.2222 Unreal centimetres per room unit.

The import script checks Unreal's glTF axis conversion itself. It imports a
small marker whose three arms have different lengths along +X, +Y and +Z,
reads the imported bounds, and places everything from JSON with the same
mapping. A change in the importer's convention then fails the import instead
of misplacing cameras and doors.

**Manifest.** The manifest holds the schema, the units, and, for every
room, its files, bounds and counts. It also gives each room's doors: the
trigger index, the target room and the arrival point. The whole door graph
can be read without parsing a trigger file.

## 4. Rendering in Unreal

The rendering target in `docs/REMAKE.md` §1 maps onto Unreal's defaults
almost directly. The bake pipeline built for Bevy is no longer needed.

| Target (REMAKE.md) | Unreal 5.8 |
|---|---|
| Hybrid GI, bounce light | Lumen global illumination, hardware ray tracing (RX 7800 XT) |
| Reflections | Lumen reflections; screen traces on wet stone |
| The original rig of 40–60 lights per room | MegaLights: many shadowed lights at a fixed cost |
| Shadows | virtual shadow maps |
| Fog, light shafts, dust | exponential height fog with volumetric fog; local fog volumes; Niagara dust |
| Anti-aliasing and upscaling | TSR; FSR as a plugin option |
| Exposure and grading | a post-process volume per room: clamped auto exposure, local exposure, colour grading, bloom |

**Per-room looks.** The fields of the Bevy `look.ron` map onto one
post-process volume and one fog actor per room:
- EV100 becomes the exposure range;
- the tonemapper stays Unreal's filmic one;
- fog colour and density come from the light set's fog;
- the grading fields become colour grading.

Colour identity starts from the original: each room's first light set
(section 4) gives the light colours, ambient and fog colour, as in `r100`'s
warm torches and cool window lights.

**Performance target.** 60 fps minimum at 1440p on the reference machine
(RX 7800 XT), as in `REMAKE.md` §5 B4.

## 5. Porting the sim

`dmc-sim` is small (2,332 lines) and deterministic, which makes it cheap to
port and easy to check.

**Shape in Unreal.** A `UWorldSubsystem`, `UDmcSimSubsystem`, owns the sim
state and steps it at exactly 60 Hz from an accumulator (ADR-003). Rendering
interpolates between the last two ticks. Actors only read the sim's state
(ADR-011): the sim never reads meshes, animation or physics.

**Module map:**

| Rust (`crates/dmc-sim/src`) | C++ (`Source/OpenDMC/Sim`) |
|---|---|
| `math.rs` (`V3`) | `DmcMath.h` (`FDmcV3`, float) |
| `input.rs`, `tape.rs` | `DmcInput.h` (input frames, buffer, tapes) |
| `controls.rs` | `DmcControls.h` |
| `moves.rs` | `DmcMoves.h` (move sets from JSON) |
| `actor.rs`, `meters.rs` | `DmcActor.h`, `DmcMeters.h` |
| `ai.rs` | `DmcAI.h` |
| `rules.rs` | `DmcRules.h` |
| `world.rs` | `DmcWorld.h` (room collision from the bundle's collision) |
| `sim.rs` | `DmcSim.h/.cpp`, then `UDmcSimSubsystem` |

**Acceptance test.** The command below writes the port's fixtures:

```
cargo run -p dmc-sim --example golden -- <out>
```

- `data.json`: the move sets and rules, as the Rust sim parsed them.
- One trace per scenario and profile: `demo`, `idle`, and three seeded fuzz
  runs of 1,800 ticks, for both `original` and `enhanced`.

Each tick records the input, the events, every actor's state and the state
hash. The traces cover walking, jumping, lock-on, the combo, launches, style
ranks, enemy attacks and deaths.

They live in `OpenDMC-UE/Tests/Golden/`. An Unreal automation test replays
each input stream through the C++ sim. It must match the state hash on every
tick, so the port is done when every trace matches.

**Determinism rules for the port:**
- `float` (32-bit) everywhere the Rust uses `f32`, and the same order of
  operations.
- No `/fp:fast`, and no fused multiply-add the Rust doesn't have.
- The state hash is FNV-1a over little-endian bytes, in the field order of
  `Sim::state_hash`.
- Both sides call the same C runtime for `sin`, `cos` and `atan2` on
  Windows. A divergence there shows up as the first mismatched tick.

**Data.** `data/` in this repo stays the source of truth (ADR-004). The
golden example's `data.json` is the exported form. Unreal loads it as JSON
and, where designers need it, as DataTables.

## 6. Rooms in Unreal: cameras, doors, collision, lights

`Scripts/import_room.py` turns one bundle room into `Content/Imported/Rooms/<room>/`
and a level:

- **Meshes:** the room glTF as static meshes, one per room object, at the
  object's transform.
- **Collision:** the collision glTF as an invisible static mesh with complex
  collision, used as simple. The sim uses its own collision (`DmcWorld`,
  from the same data); Unreal collision serves the camera and effects only.
- **Cameras:** one actor per room camera, carrying the zone (a convex
  hexahedron: two corners and six outward normals), the eye and track rails,
  the look-at offset and the FOV. The camera director ports
  `opendmc/src/room_cameras.rs`:
  - the live camera is the one whose zone holds the player, with hysteresis;
  - a rail camera's eye sits at the same fraction along its eye rail as the
    player's nearest point on the track rail;
  - cameras without rails stay at the eye;
  - every camera turns to keep the player's head in view.
- **Triggers and doors:** one box volume per trigger. On doors the target
  room and arrival point are tagged. The door rule ports
  `opendmc/src/room_doors.rs`: a door arms only after the player has been
  outside every door volume, so arriving on a door never bounces straight
  back. Rooms stream in as levels.
- **Lights:** the first light set, kinds 3 and 4, as point lights: colour
  from the float colour, attenuation radius from `far`. This is the
  starting rig for art direction, as the Blender kit's was.

## 7. What is frozen or dropped

- **`opendmc` (Bevy)** is frozen at its last commit, the `--look rt` Solari
  trial. It still builds and runs as a reference viewer.
- **Kept:** the Blender kit's `reference_room.py` and `render_cameras.py`,
  for the locked reference layer and camera renders.
- **Dropped:** `bake_room.py`, `probes_room.py`, `export_room.py` and
  `ktx2.py`, which served Bevy's baked lighting. Lumen replaces them.
- The r100 lighting study still stands:
  - the Higgsfield mood study (`opendmc-content/concept/r100/`);
  - Adam's notes: less bright and warm, more gothic, visible fog, dust.

## 8. Status and next steps

Done (2026-09-29):
- ADR-014.
- `dmc unreal`: all 106 rooms and 73 models export, none fail.
- The golden traces.
- The `OpenDMC-UE` scaffold: project, module, config, `.gitignore` and LFS,
  import script.

Next:
1. Import `r100` automatically once Unreal 5.8 has finished installing,
   including the axis check.
2. Port the sim to C++ and pass every golden trace.
3. Camera director and doors in C++; walk `r100` and cross into its
   neighbours.
4. The first art-directed Lumen pass on `r100`, reviewed by Adam against the
   notes in §7.
5. Import every room; rooms stream as levels.
