# Decisions

This file is the project's clean-room policy and its architectural decision
log. The policy section is binding: any contribution that breaks it is reverted.

---

## Clean-room policy

### What OpenDMC is
A new engine, written from scratch in Rust, that reads the data files of a
legally owned copy of *Devil May Cry* (as shipped in the *Devil May Cry HD
Collection*) to recreate the game, the same way OpenMW, OpenRCT2 and IW4L work.

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
   files (upscaled textures, material maps, converted glTF) is written to the
   user's cache or config directory and never committed or distributed.

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
*Status: accepted, Phase 0.*
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
*Status: accepted.*
It is the newest Bevy that builds on the pinned toolchain (Rust 1.94). We'll
revisit when the toolchain moves up.

### ADR-007: Placeholder art is original
*Status: accepted.*
Without a game install, the shell renders a graybox room and a capsule
"test dummy", both our own primitives. No character likeness is recreated in
code or art.
