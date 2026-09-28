# OpenDMC — End-to-End Plan

A clean-room Rust engine that runs **Devil May Cry (2001)** from the files of a
legally owned copy of the *Devil May Cry HD Collection* (Steam, app 631510), plus
an opt-in remaster layer on top.

The repository ships **engine code only**. Anyone running it must point it at
their own install. See [`DECISIONS.md`](../DECISIONS.md) for the clean-room
rules, which override anything in this plan.

---

## 0. What "done" means

| Tier | Name | Definition of done |
|---|---|---|
| T1 | **Viewer** | Every room, character, enemy, weapon and prop loads, renders textured, and plays its animations. |
| T2 | **Playable slice** | Mission 1 → Mission 3 playable: movement, fixed cameras, Rebellion + Ebony & Ivory, Marionettes, doors, orbs, HUD. |
| T3 | **Port** | All 23 missions, every weapon/enemy/boss, shop, saves, Secret Missions, difficulty modes, Dante Must Die. |
| T4 | **Remaster** | A toggleable "Enhanced" profile: modern lighting, shadows, high refresh, widescreen-aware cameras, rebinding, accessibility. |
| T5 | **Remake** | Our own content and design changes: new modes, camera options, a Bloody Palace, a mod API. Always kept separate from the Original profile. |

The engine always runs gameplay in **one of two fidelity profiles**:

- `Original`: aims to match the original's frame timing as closely as we can
  measure it. This is the reference behaviour, and parity tests run against it.
- `Enhanced`: the remaster and remake changes. It can differ from `Original`
  anywhere, but only through settings the player can see.

Gameplay code reads the profile from one `Rules` resource. There are no
scattered `if enhanced` checks.

---

## 1. Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│ opendmc (bin, Bevy)                                                       │
│   render · audio · input devices · UI · camera director · asset loading  │
└──────────────▲───────────────────────────────▲───────────────────────────┘
               │ plain data (poses, events)     │ decoded assets
┌──────────────┴──────────────┐   ┌─────────────┴────────────────────────────┐
│ dmc-sim (lib, no Bevy)      │   │ dmc-formats (lib, no Bevy)                │
│   fixed 60 Hz tick          │   │   endian-generic readers for every        │
│   input buffer              │   │   container the game ships                │
│   move tables / cancels     │   │   pure functions: &[u8] -> structs        │
│   hit/hurt boxes            │   └─────────────▲────────────────────────────┘
│   style meter / DT / orbs   │                 │
│   enemy AI state machines   │   ┌─────────────┴────────────────────────────┐
│   deterministic, replayable │   │ dmc (CLI)                                 │
└─────────────────────────────┘   │   inventory · extract · tex → PNG ·       │
                                  │   model → glTF · diff · report            │
                                  └──────────────────────────────────────────┘
```

Design rules:

1. **The sim never touches Bevy, files or wall-clock time.** It takes inputs and
   a tick, and returns state plus events. That keeps it testable, replayable and
   headless.
2. **Parsers never panic on bad data.** Every read is bounds-checked and returns
   `Result`. Real game files and fuzzers both hit these paths.
3. **Byte order is a runtime parameter.** The PS3 HD release is big-endian, and
   the PC release has to be checked (see §3). Each format is detected from its
   magic bytes in either order.
4. **Game behaviour is authored data, not constants.** Move frame data, cancel
   windows and AI timings live in `data/*.ron`, written by us from measuring the
   original. They never come out of the executable. Each value records how it
   was measured.
5. **Rendering is interpolated, simulation is fixed.** The sim ticks at 60 Hz.
   The renderer interpolates between the last two states, so 144 Hz is
   presentation only.

---

## 2. Phases

Time estimates assume one developer working part-time with heavy LLM help.
Treat them as rough.

### Phase 0: Foundations *(this commit)*
- Cargo workspace, CI-ready tests, clean-room policy, format notes.
- `dmc inventory`: walks your install, fingerprints every file (extension,
  magic, byte order, size, entropy), recurses into known containers, and writes
  `inventory.json` and `inventory.md`.
- Parsers written from the community's PS3 format notes, generic over byte
  order, and tested against synthetic fixtures that our own writers produce.
- The deterministic combat core (`dmc-sim`), with a graybox Bevy shell to drive
  it.

**Exit:** `cargo test` is green, and `dmc inventory <install>` produces a report.

**Status: done.**
- 61 tests pass: 27 format, 3 CLI (including an end-to-end run on a
  synthetic install, whose glTF output is checked by an independent reader),
  29 sim and 2 camera.
- clippy is clean with `-D warnings`.
- The graybox renders headless. `opendmc --demo --screenshot out.png
  --at-tick N` captures exactly tick N, ready for visual regression tests.
- Waiting on: your install → `dmc inventory` → Phase 1.

### Phase 1: Discovery *(1–3 weeks)*
The first real contact with the PC files.
1. Run `dmc inventory` on your install and commit only the report
   (`docs/inventory/pc-<build>.md`), never the files.
2. Answer the four gating questions:
   - Which container wraps DMC1's data on PC? Pipeworks `.BDP` like the PS3,
     loose files, or something else?
   - Byte order: is the data big-endian like the PS3, or was it swapped for x86?
   - Are the model (`.pld/.pws/.pwd/.emd/.fsd`) and texture (`T32`/`TM2`)
     layouts the same as the PS3 notes?
   - Where do room collision, camera placement, room scripts, audio and video
     live?
3. Build the **format coverage matrix** (`docs/formats/COVERAGE.md`): each file
   type is marked *parsed*, *partly understood* or *unknown*, with evidence for
   each.

**Exit:** every byte of DMC1 data belongs to a known container, and every
container type has a coverage status.

**Status: in progress.** Report:
[`docs/inventory/pc-58ed9634.md`](inventory/pc-58ed9634.md).
- Container: `data/dmc1/dmc1-0.nbz` is a plain ZIP. FMOD Studio banks and WMV
  videos sit loose beside it. No `.BDP`.
- Byte order: little-endian throughout. The texture magic bytes are the same
  as on PS3, so they can't signal byte order.
- Layouts: the same records as the PS3 notes, widened for 64-bit (u64
  offsets, `0xCC` padding). Textures are embedded DDS files. Rooms (`.fsd`)
  use their own geometry records. All 73 models and all 106 rooms export
  to glTF (`dmc export`, `dmc room`).
- Collision: `.fsd` section 9, parsed for all 106 rooms (`dmc room` writes
  it beside each room). Cameras: `.fsd` section 2, parsed for 97 rooms:
  activation zones, eye and rails, FOV. Which point is the eye was tested
  against the data (the player stays in view); blending is still to confirm
  in-game.
  Triggers: not located yet. Cutscene/event data: `.fsd` section 30. Audio: FMOD banks.
  Video: WMV.

### Phase 2: Asset pipeline *(3–6 weeks)*
| Asset | Source | Output | Validation |
|---|---|---|---|
| Textures | `T32`/`TM2` containers inside model files | RGBA8 + mip-chain | pixel dims match header; DXT decode round-trip; visual sheet |
| Static meshes | object/mesh tables | glTF | mean normal length ≈ 1.0, vertex totals match object records |
| Skeletons | geometry section | glTF skin | bone count matches motions; bind pose sane |
| Motions | motion banks (sections 6/7 players, 3/5 enemies) | glTF animations | 60 fps; root motion distance matches in-game capture |
| Leg IK | motion channels (target + hinge) | runtime 2-bone IK | feet planted on flat ground in idle/walk |
| Rooms | `.fsd` section 14 + textures in 34 | glTF / Bevy scene | 106/106 convert; object bounds cross-check; overlays match screenshots at known camera angles |
| Collision | `.fsd` section 9 (box tree of quads/triangles) | trimesh + surface flags | 106/106 rooms parse; counts and spans self-check; renders match geometry |
| Cameras | `.fsd` section 2 (zones, rails, FOV) | camera zones + rails | parsed for 97 rooms; confirm behaviour against captures; `camfit` stays the fallback |
| Props | `.fsd` section 19 (models + textures) | glTF / Bevy scene | 1,190/1,190 parse in 101 rooms; placement still to find (§4f) |
| Audio | **unknown** | decoded PCM streams | play in sync with animation events |
| Video | **unknown** (FMV) | external decoder or skip | — |

`dmc export` gives the user a local Blender-friendly dump for inspection. It is
never committed.

**Exit:** `dmc export --all` converts 100% of models with no validation failures.

### Phase 3: Room viewer *(2–4 weeks)*
- The Bevy app loads a room by id with a free-fly camera and a prop overlay.
- **Camera recovery.** DMC1's fixed cameras define the whole game. If camera
  data turns up in the room files, parse it. If not, build `tools/camfit`: take
  a screenshot from the original, mark 4+ point correspondences, solve PnP for
  the camera pose and FOV, and save the result to `data/cameras/<room>.ron`.
- Camera zones are trigger volumes that switch between cameras. They ship with
  the original's cut rules: a hard cut, and the stick direction held across the
  cut.

**Exit:** Mission 1's rooms can be walked with the free camera, and switching
to authored cameras matches screenshots within ~1° and ~5% FOV.

### Phase 4: Dante on screen *(2–3 weeks)*
- Skinned model, runtime IK, coat skeleton (the second motion bank, with
  optional spring physics in the Enhanced profile).
- An animation state graph driven by `dmc-sim` events rather than Bevy logic.
- A motion browser for scrubbing any motion frame by frame beside a capture of
  the original.

### Phase 5: Movement & collision *(3–5 weeks)*
- Kinematic character controller: walk, run, jump, air control, wall hike, roll.
- Camera-relative input that keeps working across camera cuts.
- **Measure, then author.** Record the original at 60 fps with an on-screen
  input display, count frames, and write the values into
  `data/moves/dante.ron` with a `source:` note for each.

**Status: started.** `dmc_sim::world::World` holds a room's collision
triangles in sim units (room units ÷ 450, so Dante is 2 tall) in an X/Z grid.
It answers ground queries (steps, slopes, snapping down stairs) and pushes
bodies out of walls. `Sim::with_world` swaps it in for the graybox floor.
Body sizes are placeholders until measured. Tests: synthetic walls, steps,
ledges, fast falls and replay determinism. With `OPENDMC_GAME_DIR` set, the
player also runs on every real room: 331 runs, and 7 leave the static mesh
through openings the game closes some other way (doorway props, a stairwell
gap in `r200`/`r211`).

### Phase 6: Combat core *(6–10 weeks)*
The part that decides whether the project succeeds.
- A move table per weapon: startup, active and recovery frames, cancel windows,
  hitboxes attached to bones and live on specific frames, damage, stun and
  launch values.
- Input buffer and command parser: lock-on plus direction plus button, rolling
  inputs, charge shots, Devil Trigger.
- Hit reactions and hitstop, juggle gravity, the Stinger/High Time/Helm Breaker
  families, the gun-juggle rules.
- Style meter, orb drops, Devil Trigger gauge and the DT form of each weapon.
- **Parity harness.** Scripted input tapes run the sim headless and produce
  trace files (position, state and frame for each actor on each tick). Traces
  are diffed against measurements from the original (see §4).

**Exit:** a frame-data spreadsheet for Rebellion, Alastor, Ifrit and the guns,
checked against captures, with parity tests in CI.

### Phase 7: Enemies & bosses *(8–12 weeks)*
- Each enemy gets a behaviour state machine in `data/enemies/<id>.ron`, authored
  from observation: Marionette, Blade, Shadow, Sin Scissors, Plasma, Fetish,
  Nobody, Frost, the Nightmare phases, Phantom, Griffon, Mundus.
- Shared building blocks: perception, spacing, attack tokens (a limit on how
  many enemies attack at once), hit reactions and death.

### Phase 8: Game flow *(6–10 weeks)*
- Missions, room transitions, doors and locks, key items, puzzles, the Divinity
  Statue shop, Secret Missions, ranking, save/load, difficulty modes.
- Our own UI and HUD art. The original HUD textures can be loaded for the
  `Original` look when they are found in the user's files.
- Audio: music, SFX tied to animation events, voice. FMVs are played through a
  decoder if the format is standard, otherwise skipped with a link to the
  original.

**Exit:** a full playthrough in the `Original` profile.

### Phase 9: Remaster layer *(ongoing)*
| Feature | Approach |
|---|---|
| Lighting | Clustered forward lighting with authored light probes per room. Original vertex-lit look kept as an option. |
| Shadows | Cascaded / spot shadows for Dante & enemies (original blob shadows kept as option). |
| Materials | Generate normal/roughness maps offline **on the user's machine** from their textures. Never distributed. |
| Resolution & FPS | Render at native res and uncapped FPS; sim interpolation keeps 60 Hz feel. |
| Widescreen cameras | Per-camera FOV/position adjustments in `data/cameras/*.enhanced.ron`. |
| Input | Full rebinding, modern DMC control layout option, input display, training mode. |
| Accessibility | Game speed, auto-lock, colour-blind-safe UI, subtitle styling. |
| Upscaling | Optional local ML upscaler hook (user-supplied), with results cached in the user's config dir. |

### Phase 10: Remake (T5)
Everything that isn't "the original, better". It lives behind the `Enhanced`
profile or in separate mode crates: Bloody Palace, practice mode, a mod API
(Lua or WASM scripting over the `dmc-sim` event stream), and new cameras.

---

## 3. PC-specific unknowns (resolved in Phase 1)

The community research in `docs/formats/` was done on the **PS3** HD
Collection, where DMC1 is big-endian and packed in `DMC1.BDP` bundles. The PC
port was built from the same HD codebase, so the layouts are *probably* shared,
but that is not confirmed. The inventory tool is built for exactly this doubt:

- It looks for every known magic in **both** byte orders (for example
  `Pipeworks bundle` headers mention `(big endian)` or `(little endian)`, and
  `T32`/`TM2` magics are stored word-reversed).
- It tallies what it finds per directory, so a changed layout shows up at once.
- Unknown files are reported with their first 32 bytes, entropy and size, which
  is enough to start on the next format.

---

## 4. Parity & testing strategy

| Layer | Test |
|---|---|
| Parsers | Synthetic fixtures built by our own writers (round-trip); `cargo fuzz` targets per format; optional `OPENDMC_GAME_DIR` integration tests that run only on your machine. |
| Sim | Unit tests per rule; property tests (determinism: same tape gives the same hash); golden traces. |
| Parity | **Video capture:** record the original at 60 fps with an input overlay, annotate frame numbers, and store measurements in the RON `source:` fields. **Memory oracle (optional, Windows):** a separate tool reads player position and state from the running original each frame to produce reference traces. The zlib-licensed DDMK mod documents where those live in memory. The engine never links or embeds it. |
| Visual | Screenshot diff for authored cameras (per-room SSIM against captures taken locally, never committed). |

---

## 5. Repository layout

```
opendmc/
├── DECISIONS.md            clean-room policy & architectural decisions (ADR log)
├── docs/
│   ├── PLAN.md             this file
│   ├── formats/            our own format notes + COVERAGE.md
│   └── inventory/          inventory reports (no game data)
├── crates/
│   ├── dmc-formats/        parsers (no Bevy)
│   ├── dmc-sim/            deterministic gameplay core (no Bevy)
│   ├── dmc-cli/            `dmc` tool: inventory, extract, export
│   └── opendmc/            Bevy game shell
└── data/                   authored gameplay data (moves, enemies, cameras)
```

---

## 6. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| PC formats differ from PS3 notes | Medium | Inventory first; parsers are endian-generic and validated structurally. |
| Collision/camera data not found | Medium | Fallbacks designed up front (derived collision, `camfit`). |
| Combat feel doesn't match | High | Measure-then-author discipline, parity harness, fidelity profiles. |
| Scope creep (remake ideas early) | High | T5 work is blocked until T2 exit criteria pass. |
| Legal | Low–Med | DECISIONS.md rules; no assets or exe-derived code in repo; users supply their own copy. |
| Motivation (multi-month project) | High | Each phase ends in something you can see or play. |
