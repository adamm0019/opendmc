# OpenDMC: the modern graphical remake

**Status:** a non-negotiable product requirement, adopted 2026-09-29.

It replaces the earlier "opt-in remaster layer" framing (the old T4 tier and
Phase 9 in [`PLAN.md`](PLAN.md)). The clean-room policy in
[`DECISIONS.md`](../DECISIONS.md) still binds. ADR-008 to ADR-013 there
record how this requirement fits it.

- §1 states the requirement.
- §2 assesses the project as it stood when the requirement was adopted.
- §3 to §6 give the rendering pipeline, the asset pipeline, the benchmark room
  and what waits until the benchmark is proven.

---

## 1. The requirement

### Scope
OpenDMC becomes a **faithful modern graphical remake of DMC1**. Anything about
interconnected games, portals, shared worlds or "Continuum" is out of scope and
deferred. Nothing is designed for it now.

### Vision
It must not look like an upscaled PS2 game. The original decides layout,
composition, atmosphere, scale, silhouettes and art direction. What you see is
rebuilt to modern standards.

| Preserved from the original (authoritative) | Rebuilt to modern standards |
|---|---|
| Room dimensions and layouts | Architecture and environmental geometry |
| Traversal and combat-space dimensions | Props, interactive objects, destructibles |
| Collision boundaries | Characters, enemies, weapons, items, orbs |
| Doors, triggers, enemy placements | Textures and materials |
| Fixed-camera compositions | Lighting and atmosphere |
| Architectural silhouettes | Particles and combat effects |
| Atmosphere and colour identity | Cinematic presentation |
| Gameplay readability | |
| Animation timing, combat hitboxes | |

Gameplay collision, hurtboxes, hitboxes and weapon reach are kept completely
separate from the visual models. A more detailed model never changes gameplay.

### Art direction
A modern take on DMC1's gothic horror:
- saturated gothic colour;
- theatrical moonlight;
- deep coloured shadows and strong contrast;
- ancient, decaying architecture in damp stone, metal, wood, cloth, bone and
  demonic tissue;
- dense but controlled atmosphere;
- supernatural light that need not be physically correct;
- strong enemy silhouettes and readable attacks.

Avoid:
- generic grey photorealism;
- excessive darkness;
- uncontrolled bloom;
- excessive fog;
- any effect that hides combat.

### Rendering target
The renderer and content pipeline are designed around:
- **Colour and output:** linear HDR rendering, HDR display output, per-room
  exposure and colour grading.
- **Materials:** physically based materials.
- **Lighting:** hybrid global illumination (§ below), dynamic shadowed lights,
  reflection probes, screen-space reflections, ambient occlusion, optional
  hardware ray tracing later.
- **Atmosphere:** local volumetric fog, height fog, light shafts.
- **Surfaces and effects:** decals, wet surfaces and puddles, GPU particles.
- **Image quality and performance:** temporal anti-aliasing, upscaling and
  dynamic resolution, LOD/HLOD, occlusion culling.

Experimental GPU features are never mandatory. A reliable conventional path
comes first; ray tracing and mesh shaders are optional additions after it.

### Global illumination
DMC1's rooms are mostly static and seen from controlled cameras, so the
approach is hybrid and art-directed:
- high-quality baked GI for the architecture;
- dynamic direct light and shadows;
- probe lighting for Dante, enemies, weapons and moving props;
- reflection probes per room;
- screen-space effects for local detail;
- optional ray-traced enhancement at high settings.

We do not build a complicated fully dynamic GI system just to match another
engine's feature list.

### Asset pipeline
Blender is the main authoring tool. Visual scenes and models are glTF/GLB, and
textures are KTX2 (or another suitable GPU-ready compressed format). Gameplay
information is separate, versioned metadata.

A room is structured roughly as:

```
room
├── gameplay_collision
├── architecture
├── props_static
├── props_dynamic
├── doors
├── destructibles
├── decals
├── lights
├── reflection_probes
├── gi_probes
├── fog_volumes
├── audio_zones
├── cameras
└── gameplay_triggers
```

The imported original geometry stays in Blender as a **locked reference
layer**. Modern visual geometry is built separately, so the original
measurements are always there to compare against.

### Materials
**Channels.** A material set has base colour, normal, roughness, metalness,
ambient occlusion, emissive, height where it helps, and opacity or subsurface
data where needed.

**Families.** Reusable families cover:
- gothic stone, marble, plaster, damp masonry;
- carved wood, leather, cloth;
- corroded iron, brass, silver;
- bone, demonic tissue;
- stained glass;
- wet surfaces.

**Technique.** Trim sheets, tileable materials, decals and vertex blending, not
huge unique textures for every surface.

### Characters, enemies and weapons
Modern models keep the originals' proportions, silhouettes, attachment points
and gameplay scale.

**Modelling pipeline:** high-poly sculpts, retopologised runtime meshes, baked
normal maps, several LODs.

**Shaders:** skin, eyes, hair, cloth, armour, metal and demonic materials.

**Motion and damage:**
- secondary cloth and hair motion;
- surface damage and reactive wounds;
- breakable visual armour;
- burning, electrified and Devil Trigger material states;
- impact effects per material.

Secondary animation and visual damage never touch gameplay collision or combat
logic.

### Higgsfield
When it is available, authorised and genuinely useful, Higgsfield may support
visual development:
- environment mood;
- lighting and atmosphere studies;
- architectural variation sheets;
- material references;
- enemy and weapon concepts;
- colour-script frames;
- cinematic previs;
- quick comparisons of visual directions.

Limits:
- Its output never counts as gameplay geometry, collision, final topology,
  exact UVs or production assets without human review and reconstruction.
- Every generated reference is checked against DMC1's own art direction, so
  that nothing drifts toward generic fantasy, generic photorealism or
  inconsistent architecture.
- Licensing is respected, and generated material is never assumed to be
  redistributable.

### First visual benchmark
Before the whole game is rebuilt, one representative room is finished as the
quality benchmark: **`r100`, the castle entrance hall** (§5). It must include:
- gameplay-accurate dimensions and collision;
- modern architecture and high-quality PBR materials;
- a modern Dante with Alastor and Ebony & Ivory;
- one modernised Marionette;
- modern red and green orbs;
- a working door and at least one destructible;
- the original fixed-camera compositions;
- global illumination, dynamic shadows, reflections and volumetric atmosphere;
- combat particles and weapon effects;
- final colour grading;
- performance measurements.

That room then sets the visual and technical standard for every later asset.

> **Guiding rule.** Add geometry, material depth, lighting quality, atmosphere,
> character detail and effects. Keep the original's composition, gameplay
> scale, readability and gothic identity.

---

## 2. Assessment (2026-09-29)

### 2.1 What we keep

Everything built so far stays: it becomes the gameplay authority and the
reference layer.

**Parsers (`dmc-formats`).** These are the authority for everything the
requirement preserves:
- room collision (§4d of the format notes);
- camera zones, eyes, rails and FOV (§4e);
- triggers and doors (§4g);
- props (§4f);
- model geometry, skeletons and motions: proportions, attachment points and
  animation timing.

**Deterministic sim (`dmc-sim`).** It already works as the requirement asks:
- it runs its own collision world (`world::World`) at a fixed 60 Hz, with
  capsules and hitboxes that never read a render mesh;
- visuals follow it through `sync_visuals`;
- so modern models can replace the placeholders without touching gameplay.

**Room cameras (`opendmc::room_cameras`).** The fixed-camera compositions come
straight from the original's data. The director only moves a transform, so any
render stack can sit behind it.

**`dmc room` exports.**
- `room.glb`, `room.collision.glb`, `room.props.glb`, `cameras.json` and
  `triggers.json` are already what the Blender reference layer needs (§4).
- These exports are derived from the user's files, so they stay local
  (policy rule 6).

**The original-look renderer.** Unlit, vertex-lit rooms and skinned models from
the user's files stay as `--look reference`. They're the side-by-side check for
composition and scale, and the fallback for rooms not rebuilt yet.

**Tick-exact capture (`--screenshot --at-tick`).** This becomes the benchmark's
comparison and regression capture.

**Bevy 0.18.** Its conventional path already has what the target needs:
- **Core rendering:** HDR, deferred PBR, cascaded shadows, SSAO, SSR, TAA.
- **Post-processing:** bloom, auto-exposure, colour grading.
- **Atmosphere:** distance fog, local fog volumes, volumetric light shafts.
- **Baked and probe lighting:** lightmaps, irradiance volumes, reflection
  probes (including runtime prefiltering).
- **Surfaces:** clustered and forward decals.
- **Performance:** visibility ranges for LOD, experimental GPU occlusion
  culling.
- **Formats:** KTX2 with zstd.
- **Optional extras:** meshlets, DLSS and ray-traced GI (Solari).

Nothing forces an engine change.

### 2.2 What conflicts with the target

1. **ADR-007: "placeholder art is original; no character likeness".** A modern
   Dante, Marionette, Alastor and Ebony & Ivory recreate Capcom's characters.
   - ADR-009 replaces it: likeness art is allowed, but only in a private
     content store outside this repository.
2. **The Phase 9 "remaster layer".** Normal and roughness maps generated from
   the original textures, plus an ML upscaler hook, produce exactly the
   upscaled-PS2 look the requirement rules out. They are dropped as a goal:
   - at most, a stopgap for rooms that have not been rebuilt yet;
   - never the product.
3. **Rooms render unlit, with vertex colours as the lighting.** Materials use
   `unlit: true`, are double-sided and alpha-masked. The camera is a default
   `Camera3d`: no HDR, MSAA on, no prepasses. SSR needs deferred rendering,
   and SSAO and TAA need MSAA off, so the modern path needs a different camera
   stack. The old one survives only as the reference look.
4. **Visuals and gameplay come from one `.fsd`, in one Startup system.**
   `load_room` builds the meshes, the collision world and the camera director
   together, once, from CLI flags. Consequences:
   - a room cannot be swapped at runtime (doors need that);
   - the visual layer cannot be replaced independently of the gameplay data.
5. **The unit is only "metre-ish".**
   - `ROOM_SCALE = 2/900` lives in the Bevy crate, and the sim's tests use
     room units ÷ 450 separately.
   - Light intensities in lux and lumens, fog densities, probe spacing, shadow
     cascades and Blender scene units all assume a real unit.
   - ADR-010 fixes it: **1 world unit = 1 m = 450 room units**.
6. **Textures are decoded to uncompressed RGBA8 on the CPU at load.** That's
   fine for the reference look. Modern content ships GPU-compressed KTX2 with
   mips instead.
7. **Prop placement is unknown** (§4f of the format notes). It is only a
   problem for the reference layer: the benchmark's modern props are placed in
   Blender against the reference geometry and captures of the original, but
   `r100`'s original props can't be shown in place yet.

### 2.3 The smallest changes to make now

These cost little today, but they are expensive to retrofit once content
exists.

1. **One unit.** A single `units` constant (room units per metre = 450),
   shared by the sim, the renderer and the Blender scripts. Blender scenes are
   authored in metres.
2. **Split gameplay from visuals per room.**
   - `RoomGameplay` holds collision, cameras, triggers and doors. It is built
     only from the original data plus authored overrides in `data/rooms/*.ron`.
   - `RoomVisual` is either `Reference` (the original meshes) or `Scene`
     (a modern glTF from the content store).
   - Gameplay systems never query visual entities.
3. **Runtime room lifecycle.** A `LoadRoom` message despawns the current room
   root and spawns the next. Door transitions need this now, and streaming
   and benchmark captures need it later.
4. **A content root.**
   - `--content <dir>` (or `OPENDMC_CONTENT`) points at the private store.
   - Each room has `rooms/r100/room.ron`: a manifest with a schema version,
     naming its visual scene, baked lighting, probes and look.
   - A room without a manifest falls back to the reference look.
5. **One render-stack module.**
   - `render::Look` (`Reference` / `Modern`) configures the camera stack.
   - A per-room `look.ron` covers exposure range, grading, fog and bloom
     limits, with per-camera overrides keyed by the original camera index.
   - The camera director keeps moving transforms only.
6. **A naming contract for Blender collections and glTF nodes.** Collection
   names map to Bevy components: lights, probes, fog volumes, decals, audio
   zones. Parameters travel in glTF `extras`. A validator enforces the contract
   (§4).
7. **Sockets, not meshes, bind visuals to gameplay.**
   - Character and weapon visuals attach through named sockets (a socket map
     per model).
   - Hitboxes stay in `data/moves/*.ron`, in sim space against the sim's
     bones, so a new mesh cannot move one.
8. **Per-camera data is keyed by (room, original camera index).** Look
   overrides, visibility sets and composition checks all survive rebuilding
   the visuals.

---

## 3. Rendering pipeline (conventional path)

Everything below is built into Bevy 0.18 unless marked *custom*.

| Stage | Choice |
|---|---|
| Frame | Linear HDR (`Hdr`, `Rgba16Float`); deferred opaque pass + forward transparents; depth, normal, motion-vector and deferred prepasses; MSAA off |
| Direct light | Moonlight as a directional light with cascaded shadows (fixed cameras allow tight, per-camera cascade bounds); torches and candles as shadowed spot/point lights. PCSS soft shadows optional (`experimental_pbr_pcss`) |
| Static indirect | Lightmaps baked with Blender Cycles into a second UV set; bicubic sampling. Stored as HDR KTX2 |
| Dynamic indirect | One or more irradiance volumes per room (3D ambient-cube textures). Baked from the same Cycles scene (*custom bake script*) |
| Specular | Reflection probes per room: cubemaps baked in Blender, prefiltered offline or at runtime (`GeneratedEnvironmentMapLight`). SSR on top for floors, puddles and polished stone |
| AO | Static AO baked into lightmaps; SSAO for actors and contact detail |
| Atmosphere | Exponential distance fog per room; `FogVolume` local volumes with `VolumetricLight` shafts from moonlit windows. Height fog uses fog volumes with a density texture; a small *custom* pass only if that falls short |
| Decals | Clustered decals (`pbr_clustered_decals`) for grime, damp, blood and scorch marks; forward decals for small details |
| Wet surfaces | Material-level wetness mask (vertex-colour or texture channel) darkening albedo and lowering roughness, plus SSR. *Custom* extended material |
| Particles | GPU particles for combat and ambience; the library is chosen at B3, and `bevy_hanabi` is a candidate if it tracks Bevy 0.18 |
| Post | Bloom with conservative thresholds; auto-exposure clamped by the room's EV range, plus per-camera bias; tonemapping (AgX or TonyMcMapface, chosen on the benchmark for how they hold saturated colour); per-room `ColorGrading`, with LUTs later |
| AA / resolution | TAA at native resolution first. Dynamic resolution later. DLSS as an optional feature (NVIDIA only). FSR is not in Bevy, so it is deferred |
| Culling / LOD | Frustum culling, plus **precomputed visible sets per fixed camera** (each camera sees a known part of the room, so this is cheap and exact), plus `VisibilityRange` LOD crossfades. GPU occlusion culling optional |
| HDR display output | Not exposed by Bevy 0.18. The frame is HDR internally, so it can be added when the engine supports it. Deferred |
| Optional later | Ray-traced GI and reflections (Solari), meshlet virtual geometry. Never required |

The reference look stays available through `--look reference` for
side-by-side checks.

---

## 4. Blender to runtime

All files described here live in the **content store**: a private location
outside this repository (ADR-009). It holds everything derived from the user's
files (reference layers) and all rebuilt art. This repository ships only the
tools and scripts.

1. **Reference export.** `dmc room <r100.fsd> --out <store>/rooms/r100/reference/`
   writes:
   - `room.glb`, the original visual geometry;
   - `collision.glb`;
   - `props.glb`;
   - `cameras.json` (zones, eyes, rails, FOV);
   - `triggers.json` (volumes and doors).
2. **Reference scene.** `tools/blender/reference_room.py` runs headless and
   builds `r100.blend` in metres. It contains:
   - a locked, unrenderable `reference` collection holding the original
     geometry, `gameplay_collision`, `cameras` (Blender cameras with the
     original FOV; rails as curves) and `gameplay_triggers` (boxes and
     cylinders, doors tagged with their target room);
   - the empty modern collections from §1: `architecture`, `props_static`,
     `props_dynamic`, `doors`, `destructibles`, `decals`, `lights`,
     `reflection_probes`, `gi_probes`, `fog_volumes`, `audio_zones`.

   Looking through an original camera shows the original composition.
3. **Authoring.** Modern geometry and materials go in the modern collections.
   Materials come from a shared library (`materials.blend`) of the families
   in §1, built on trim sheets and tileables. Custom properties carry
   parameters: probe extents, fog density, light flicker, destructible
   health.
4. **Bake.** `tools/blender/bake_room.py`:
   - bakes Cycles lightmaps into UV1 (unwrapping with a lightmap pack where
     UV1 is missing);
   - renders a cubemap per reflection probe;
   - samples each irradiance volume.
5. **Export.** `tools/blender/export_room.py` writes the modern collections
   only (never `reference`) to `visual.glb`, with glTF `extras` for
   parameters. It writes lightmaps, probes and textures as KTX2 (BC7 colour,
   BC5 normals, BC6H HDR, zstd, full mips), then updates the room's `room.ron`
   manifest (schema-versioned). The KTX2 encoder is our own `dmc` subcommand
   unless an external tool proves better.
6. **Validate.** `dmc content check <store>/rooms/r100` checks:
   - the manifest and naming contract, the units, and the texture formats;
   - that visual floors lie within tolerance of collision floors;
   - **composition**: every original camera renders both the reference and
     the modern layer to depth, and the report lists where the modern
     silhouettes stray beyond tolerance;
   - that nothing in the visual scene is read as gameplay data.
7. **Load.** `opendmc --content <store> --room <r100.fsd> --walk`:
   - gameplay comes from the `.fsd` (plus `data/rooms/r100.ron` overrides);
   - visuals come from the manifest;
   - with no manifest, the reference look is used.

Characters, enemies and weapons follow the same path. The original model (from
`dmc export`) is the locked reference for proportions, bone positions and
sockets. The modern mesh is skinned to a skeleton that matches the original's
bone layout, so the original motions, and their timing, drive it. Sculpt →
retopology → bake → LODs happens in Blender. Socket maps and material-state
parameters (burn, shock, Devil Trigger) are exported as `extras`.

---

## 5. Benchmark room: `r100`, the castle entrance hall

`r100` has:
- the red-carpeted staircase and statue;
- 53 cameras, most of them on rails;
- collision;
- doors to `r101`, `r106`, `r110`, `r116` and `r11b`.

It is the hub of the opening missions.

### Milestone B0 (next): the pipeline proven on the original geometry
Before any new art is made, run the whole modern pipeline on `r100`'s
reference geometry. It must meet each of these:

1. **Doors.** `--walk` crosses between `r100` and its neighbours through
   runtime room loading (the lifecycle in §2.3). *Done 2026-09-29.*
2. **Units.** One shared metre constant; the sim, renderer and exports agree.
   *Done 2026-09-29: `dmc_sim::world::ROOM_UNITS_PER_METRE`.*
3. **Modern look.** `--look modern` uses the HDR deferred stack from §3:
   shadows, SSAO, SSR, TAA, bloom, clamped exposure and grading. It is driven
   by a per-room `look.ron`, with the reference look kept alongside.
   *Stack in place 2026-09-29 (`opendmc::render`, `data/looks/default.ron`,
   `--look`, `--content`). The rooms are lit only by the moon and an ambient
   term until item 6 bakes their lighting.*
4. **Content store.** It holds a manifest loader and falls back to the
   reference look.
5. **Blender kit.** `reference_room.py` builds `r100.blend` with the §1
   collection tree, locked reference, cameras and triggers.
6. **Baked lighting.** A Cycles bake of the reference geometry (moonlight
   plus torches), with:
   - one reflection probe;
   - one irradiance volume;
   - one fog volume with light shafts.

   All of it is exported and loaded, with the original vertex colours as the
   colour-identity reference for the lighting.
7. **Benchmark mode.** `--bench` visits every `r100` camera and writes, for
   each one:
   - a reference/modern screenshot pair;
   - CPU and GPU frame times.

   The report stays local.
8. **Composition check.** `dmc content check` reports zero composition
   deviation for the reference layer, which is the check's own sanity test.

### B1: architecture and materials
- The material families in §1, and a trim-sheet kit for the hall's gothic
  vocabulary: columns, balustrades, arches, carved panels.
- `r100`'s architecture rebuilt against the locked reference; composition
  check green on every camera.
- Decals, wetness, lighting and the fog pass art-directed per camera; final
  grade.

### B2: characters, weapons, pickups
- Modern Dante, Alastor, Ebony & Ivory, a Marionette, and red and green orbs.
- Each skinned to the original skeleton layout and driven by the original
  motions; socket maps; LODs.
- Material states: Devil Trigger, burning and electrified on at least one
  target.

### B3: play
In `r100`:
- the Alastor and Ebony & Ivory move sets and one Marionette's behaviour
  (Phases 6 and 7 applied to this room);
- orb drops and pickup;
- the working door;
- one destructible;
- GPU particles for hits, muzzle flashes, Alastor's lightning, orb bursts and
  Marionette deaths.

### B4: sign-off
Performance targets are measured on the reference machine (60 fps minimum at
1440p on the conventional path). All cameras are captured next to the
original, and a visual review is signed off. The room then becomes the
standard.

### Who makes what
**Claude builds (automatable end to end):**
- the engine, pipeline and validation tools;
- the Blender scripts, blockouts, procedural materials and trim sheets;
- lighting, bakes, fog and grading drafts;
- performance work.

**Needs a human artist, or AI-assisted generation with human review and
reconstruction:**
- hero-quality sculpts: Dante's face and coat, the Marionette, the weapons'
  ornament;
- final judgment on art direction.

This is the benchmark's main schedule risk, and B2 is where it lands.

---

## 6. Deferred until the benchmark is signed off

**Rebuilding beyond `r100`:**
- rebuilding any room other than `r100`;
- characters and enemies other than Dante and the Marionette.

**Advanced character visuals:**
- cloth and hair simulation beyond simple springs;
- reactive wounds and breakable armour (the pipeline keeps room for them, but
  none are made yet).

**Optional rendering:**
- hardware ray tracing (Solari), meshlets and mesh shaders;
- HDR display output (waiting on the engine);
- DLSS/FSR integration and dynamic resolution;
- GPU occlusion culling (the per-camera visible sets come first).

**Other work:**
- audio zones beyond placeholders;
- ML texture upscaling of the original textures, and generating normal maps
  from them;
- Higgsfield use beyond mood and reference boards;
- interconnected-game, portal, shared-world and "Continuum" work (out of scope
  entirely);
- the T5 extras in `PLAN.md`.

Gameplay reverse engineering continues alongside: prop placement, trigger
kinds, cut rules, motion timing. It feeds the preserved half of the table in
§1.
