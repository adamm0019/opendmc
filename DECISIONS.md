# Decisions

This file is the project's clean-room policy and its architectural decision
log. The policy section is binding: any contribution that breaks it is reverted.

---

## Clean-room policy

### What OpenDMC is
A new engine, written from scratch in Rust, that reads the data files of a
legally owned copy of *Devil May Cry* (as shipped in the *Devil May Cry HD
Collection*) to recreate the game, the same way OpenMW, OpenRCT2 and IW4L work.
It is a modern graphical remake:
- the original's data decides layout, collision, cameras and gameplay;
- the visuals are rebuilt ([`docs/REMAKE.md`](docs/REMAKE.md), ADR-008).

### Hard rules
1. **No game data in the repository.** That covers models, textures, audio,
   video, text, executables, extracted files, and screenshots or captures of
   copyrighted content. `.gitignore` blocks the usual extraction folders, and
   test fixtures are synthetic, produced by our own writers.
2. **No leaked, stolen or confidential material, ever.** This includes any
   Capcom source code, SDK material, internal tools or debug builds. If you come
   across such material, stop reading it and do not contribute to the related
   subsystem for this project.
3. **No code derived from the game executable.** Behaviour is re-derived from
   black-box observation of the running game (frame counting, captures, input
   tapes). Nobody disassembles or decompiles `dmc1.exe` to write engine code,
   and no addresses, constants or routines from it are copied into the engine.
4. **Community format research may be used as documentation, not as code.**
   Facts about data layouts (field offsets, magics, encodings) are written up
   again in our own words in `docs/formats/`, with a citation, and implemented
   independently. Code from a project with no licence is never copied.
5. **The end user supplies their own copy.** The engine asks for the install
   path and never downloads or bundles game content.
6. **Remaster derivatives stay local.** Anything generated from the user's
   files (upscaled textures, material maps, converted glTF, Blender reference
   layers) is written to the user's cache, config directory or private content
   store, and never committed or distributed.
7. **Rebuilt art stays out of this repository.** Modern models, textures,
   scenes and baked lighting that recreate the game's characters, weapons and
   places live in a private content store (ADR-009). This repository holds
   only the code, tools and scripts that make and load them, and our own
   original placeholders.

### Allowed sources

| Source | Licence | What we use | Where |
|---|---|---|---|
| Your own observation of the game you own | — | frame data, timings, AI behaviour, camera placement | `data/**/*.ron` with `source:` notes |
| [luismateusvargas/dmc-model-extractor](https://github.com/luismateusvargas/dmc-model-extractor) | **none stated** | *format facts only* (PS3): bundle header, mesh tables, texture container, skeleton, motion banks, IK encoding | rewritten in `docs/formats/*.md`; **no code copied** |
| [serpentiem/ddmk](https://github.com/serpentiem/ddmk) | zlib | only in the optional, separate memory-oracle tool, for reading player state from the running original to check parity | never linked into or embedded in the engine |
| Public references (Microsoft DXT/BCn docs, glTF 2.0 spec, PS2 TIM2 docs) | public | generic codecs | `dmc-formats` |

### Forbidden sources
- Any repository or archive presenting itself as Capcom or Pipeworks source code.
- Decompilations or disassembly dumps of any Devil May Cry executable.
- Ripped asset packs, whoever made them. Contributors extract from their own copy.

### Every contributor affirms
> I have not consulted leaked or decompiled Devil May Cry source code for my
> contribution, and it contains no game data.

(This goes in the PR template.)

---

## Architectural decision log

### ADR-001: Rust workspace split into formats / sim / cli / game
*Status: accepted, Phase 0; amended by ADR-014 (the game moves to Unreal;
this workspace becomes the toolchain, and `opendmc` is frozen).*
Parsers and gameplay rules are pure libraries with no engine dependency, so
both can be tested, fuzzed and replayed headless. Only the `opendmc` crate
depends on Bevy.

### ADR-002: Byte order is a runtime parameter
*Status: accepted.*
The community notes describe the PS3 build (big-endian). The PC build's byte
order is not yet confirmed. Every reader takes an `Endian` value, and container
detection tries both orders.

### ADR-003: Fixed 60 Hz simulation, interpolated rendering
*Status: accepted.*
The original's feel depends on whole-frame timing at 60 fps (the motions are
keyed at 60 fps). `dmc-sim` advances in integer ticks and never sees floating
wall-clock time, and the renderer interpolates for higher refresh rates.

### ADR-004: Gameplay values are authored data with provenance
*Status: accepted.*
Move frame data, cancel windows, damage and AI timings live in RON files. Each
entry carries a `source` string describing how it was measured (for example
`"capture 2026-10-02 m01 rebellion-combo, frames 112-151"`). Unmeasured values
are marked `source: "placeholder"`, and the parity report lists them.

### ADR-005: Two fidelity profiles
*Status: accepted.*
`Original` and `Enhanced` are chosen at start-up and exposed to gameplay as one
`Rules` value. Remake features can only change behaviour through `Rules`.

### ADR-006: Bevy 0.18 for the game shell
*Status: superseded by ADR-014, 2026-09-29.*
It is the newest Bevy that builds on the pinned toolchain (Rust 1.94). We'll
revisit when the toolchain moves up.

### ADR-007: Placeholder art is original
*Status: accepted; narrowed by ADR-009 (rebuilt likeness art is allowed in the
private content store).*
Without a game install, the shell renders a graybox room and a capsule
"test dummy", both our own primitives. No character likeness is recreated in
code or art.

### ADR-008: The product is a modern graphical remake
*Status: accepted, 2026-09-29. Supersedes the "opt-in remaster layer" (old T4,
Phase 9).*

What the original's data decides, the remake keeps:
- room layouts and dimensions;
- collision;
- doors, triggers and enemy placement;
- fixed-camera compositions;
- animation timing and hitboxes.

Everything visible is rebuilt to modern standards, under the art direction,
rendering target and benchmark set out in `docs/REMAKE.md`. That file is a
non-negotiable requirement. The original's own meshes and lighting remain as
`--look reference`, for comparison and for rooms not rebuilt yet.
Interconnected-game, portal, shared-world and "Continuum" work is out of
scope.

### ADR-009: Rebuilt art lives in a private content store
*Status: accepted, 2026-09-29.*

A modern Dante, Marionette or entrance hall recreates Capcom's characters and
places. It can't be the "original placeholder art" of ADR-007, and it can't
be distributed.

- Such content lives in a content store outside this repository: by default
  a sibling folder, or a separate private repository with Git LFS. The engine
  reads it through `--content <dir>` (or `OPENDMC_CONTENT`).
- The Blender reference layers built from the user's files live there too
  (policy rule 6).
- This repository holds the code, scripts and schemas, and our own
  placeholders.
- Nothing in the store is published or bundled with a release.
- Generated references (Higgsfield or similar) go in the store as concept
  material, never as authoritative geometry.

### ADR-010: One world unit is one metre (450 room units)
*Status: accepted, 2026-09-29.*

Room units ÷ 450 was already the sim's scale: Dante, about 900 room units,
stands 2 units tall. It is now declared to be metres, for three reasons:
- physically based light units, fog densities, probe spacing and Blender
  scenes all need a real unit;
- changing the unit once content exists would mean rescaling every asset and
  every authored value;
- only relative scale matters to gameplay, and it is unchanged.

The constant is defined once and shared by the sim, the renderer, the exports
and the Blender scripts.

### ADR-011: Gameplay never reads visual assets
*Status: accepted, 2026-09-29.*

These come only from the original's data and from `data/*.ron` (with
provenance):
- collision;
- camera zones and rails;
- triggers and doors;
- hurtboxes, hitboxes and reach.

Visual models attach to gameplay through named sockets. Secondary animation,
cloth, hair, visual damage and a model's level of detail never feed back into
the sim. Each room is split into gameplay data (`RoomGameplay`) and a
swappable visual layer (`RoomVisual`: reference or modern scene).

### ADR-012: A conventional rendering path first
*Status: superseded by ADR-014, 2026-09-29 (the Bevy stack below is kept
for the record; Unreal's is in `docs/UNREAL.md` §4).*

**Core stack (Bevy 0.18):**
- linear HDR, deferred PBR;
- cascaded and spot shadows;
- SSAO, SSR, TAA;
- bloom, clamped auto-exposure, per-room colour grading.

**Lighting and atmosphere:**
- Cycles-baked lightmaps for architecture;
- irradiance volumes for actors;
- baked reflection probes;
- fog volumes with volumetric light;
- clustered decals.

**Optional only, never required:** hardware ray tracing (Solari), meshlets,
DLSS, HDR display output and GPU occlusion culling. The full table is in
`docs/REMAKE.md` §3.

### ADR-013: Content formats
*Status: accepted, 2026-09-29; amended by ADR-014 (glTF stays the
interchange; runtime assets become Unreal assets in the private project).*

- **Scenes and models:** glTF 2.0 binary (`.glb`), with parameters in node
  `extras` under a fixed naming contract.
- **Textures, lightmaps and probes:** KTX2 in GPU formats with zstd
  supercompression and full mips (BC7 colour, BC5 normals, BC6H HDR).
- **Room manifests and looks:** RON, with a `schema` version field. The
  loader rejects a newer schema than it knows.

### ADR-014: Unreal Engine 5.8 for the game
*Status: accepted, 2026-09-29. Supersedes ADR-006 and ADR-012; amends
ADR-001 and ADR-013.*

The remake's visual bar (ADR-008) needs dynamic global illumination,
reflections, volumetrics and many shadowed lights. Bevy 0.18 reached it only
through bakes and workarounds: its deferred path mis-compiles with
irradiance volumes, alpha-masked deferred meshes draw black over Solari, and
Solari has no denoiser outside NVIDIA's. Unreal 5.8 ships Lumen, MegaLights,
virtual shadow maps, volumetric fog and TSR, and a mature editor for the art.

- **The game** lives in a new repository, `OpenDMC-UE`, beside this one:
  a C++ Unreal 5.8 project, with Git LFS for Unreal assets.
- **This workspace becomes the toolchain.** `dmc-formats` and `dmc` keep
  reading the user's install. `dmc unreal` writes the bundle Unreal
  imports (every room and model, and a manifest). The Blender kit stays.
- **The sim is ported to C++** inside the Unreal project (a fixed 60 Hz
  world subsystem, ADR-003). `dmc-sim` stays as the reference: its golden
  traces (`cargo run -p dmc-sim --example golden`) are the port's
  acceptance test, tick for tick, by state hash.
- **`opendmc` (the Bevy shell) is frozen.** It still builds and runs as a
  reference viewer. It gets no new features.
- **The clean-room policy and game-data rule are unchanged.** Assets imported
  from the user's install and rebuilt likeness art stay out of every
  repository. In the Unreal project they live under ignored content folders
  and are rebuilt by its import scripts.

Details are in `docs/UNREAL.md`.
