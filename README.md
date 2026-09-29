# OpenDMC

A clean-room Rust engine for **Devil May Cry (2001)** that runs from the files
of your own copy of the *Devil May Cry HD Collection*, as a faithful modern
graphical remake. The original's data decides layout, collision, cameras and
gameplay, and the visuals are rebuilt ([`docs/REMAKE.md`](docs/REMAKE.md)).

> **No game data is included or distributed.** You need to own the game. See
> [`DECISIONS.md`](DECISIONS.md) for the clean-room rules every contribution
> follows.

**The game is moving to Unreal Engine 5.8** (ADR-014, 2026-09-29), in the
`OpenDMC-UE` repository. This repository is its toolchain: it reads your
install, exports what Unreal imports (`dmc unreal`), and keeps the Rust sim
as the reference for the C++ port. The Bevy shell is frozen. See
[`docs/UNREAL.md`](docs/UNREAL.md); the full roadmap is in
[`docs/PLAN.md`](docs/PLAN.md).

## What works today

| Piece | State |
|---|---|
| `dmc-formats` | Readers for Pipeworks bundles, `T32`/`TM2` texture containers (DXT1/DXT5/ARGB), model section lists, geometry + skinning + skeletons, motion banks with Hermite curves, leg IK. Both byte orders, fully bounds-checked. Tested with synthetic fixtures only. |
| `dmc` CLI | `inventory`, `bundle-list`, `bundle-extract`, `info`, `tex` (→ PNG), `model` (→ textured, skinned `.glb`), `export` (batch with validation stats), `room` (rooms → `.glb` plus cameras, triggers and lights as JSON), `unreal` (the bundle the Unreal project imports). |
| `dmc-sim` | Deterministic 60 Hz combat core: input buffer, combo links, cancel windows, command moves under lock-on, hitboxes vs hurt capsules, hit-stop, launches and juggle gravity, style meter, Devil Trigger, a melee enemy brain, input tapes, state hashing. All values are placeholders until measured. `--example golden` writes the traces the C++ port must match. |
| `opendmc` | *Frozen (ADR-014).* Bevy 0.18 graybox and room viewer: fixed cameras with held-direction-across-cuts controls, interpolated rendering, HUD, hitbox view, tape recording, and `--model` to view a model file from your install. |

## Quick start

```sh
cargo test                       # formats, CLI and sim (no game data needed)
cargo run -p opendmc --release   # graybox training room
```

Controls: **WASD** move · **J** melee · **Space** jump · **Shift/L** lock-on ·
**U** Devil Trigger · **F1** hitboxes. With lock-on held: forward + J = lunge,
back + J = launcher. J in the air = air combo.

## Step 1 with your copy (Phase 1: Discovery)

Once the HD Collection has finished installing through Steam:

```sh
# 1. Map the install. Writes inventory.json (keep local) and inventory.md.
cargo run --release -p dmc-cli -- inventory \
    "C:/Program Files (x86)/Steam/steamapps/common/Devil May Cry HD Collection" \
    --out my-inventory

# 2. If it finds Pipeworks bundles, list/extract DMC1's.
cargo run --release -p dmc-cli -- bundle-list <path/to/bundle> --pattern "*.pld"
cargo run --release -p dmc-cli -- bundle-extract <path/to/bundle> extract/

# 3. Convert models to glTF for Blender, with validation stats.
cargo run --release -p dmc-cli -- export extract/ export/

# 4. See one in the engine.
cargo run --release -p opendmc -- --model extract/<...>/pl00.pld
```

Share `inventory.md` (names, sizes and statistics only) to drive the next
phase. Keep `extract/`, `export/` and `inventory.json` to yourself; `.gitignore`
already excludes them.

## Layout

```
crates/dmc-formats   parsers (no engine dependency)
crates/dmc-sim       deterministic gameplay core (no engine dependency)
crates/dmc-cli       the `dmc` tool
crates/opendmc       Bevy game shell (frozen)
data/                authored gameplay data: moves, cameras (with provenance)
docs/                plan, format notes, coverage matrix, inventory reports
```

## License

Code: MIT OR Apache-2.0. *Devil May Cry* is a trademark of Capcom. This project
is not affiliated with or endorsed by Capcom.
