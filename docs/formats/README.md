# DMC1 data formats

Our own notes, rewritten from community research and from reading the files
of our own copies, and checked against our code.
**Status labels** on each section:

- `PS3-community`: described by community research on the PS3 build and not yet
  seen on PC.
- `PC-verified`: confirmed on a PC install with `dmc inventory` / `dmc export`
  or a parser test over every file of that type.
- `speculative`: our hypothesis.

Unless noted otherwise, offsets are in bytes, and multi-byte fields use the
file's byte order (`E`). The PC build is little-endian throughout. Primary
source for the PS3 facts: the comments in luismateusvargas/dmc-model-extractor
(see `DECISIONS.md`: facts only, no code). The PC facts are our own reading of
the data files; nothing here comes from the executable.

The PC inventory these notes were checked against is
[`docs/inventory/pc-58ed9634.md`](../inventory/pc-58ed9634.md).

---

## 0. PC install layout: `PC-verified`

The Steam *HD Collection* keeps DMC1 under `data/dmc1/`:

| Path | What | Format |
|---|---|---|
| `dmc1-0.nbz` | all game data (1.6 GB, 1,212 entries: 1,190 files + 22 directories) | plain ZIP, renamed (§0.1) |
| `audio/*.bank` | music, SFX and voice (4 banks, 117 MiB) | FMOD Studio banks (§10) |
| `Video/*.wmv` | FMV (31 files, 3 GiB) | ASF / Windows Media Video (§11) |

### 0.1 `.nbz` archives

An `.nbz` file is an ordinary ZIP file: a `PK\x03\x04` local header, a
central directory, and every file entry stored with Deflate (method 8). There
is no extra encryption or custom header. Directory entries are present (method
0, size 0). Entry names use `/` and mixed case (`Emd/`, `Etc/`, `etc_c/`,
`Fsd/`, `Pld/`, `status/`, `status_<lang>/`, `text/`, which is empty).

The `_<letter>` suffixes are per-language copies of the same file set (`c`,
`e`, `f`, `g`, `i`, `s`, `u`, `z`, plus the unsuffixed default). `e`, `f`,
`g`, `i`, `s` read naturally as English, French, German, Italian and Spanish;
`c`, `u`, `z` and the default are not confirmed. *(speculative)*

The ZIP directory records a CRC-32 and a date for every entry. `dmc inventory`
hashes names, sizes and CRCs into a **data-build fingerprint**, so two installs
can be compared without sharing any data. The newest entry date in the
Steam build is 2018-03-01.

## 1. Pipeworks bundle (`.BDP`): `PS3-community`

The HD port's archive on PS3, where DMC1's data sits in `BUNDLES/DMC1.BDP`.
**Not used on PC** (§0.1).

| Off | Type | Field |
|---|---|---|
| 0x00 | char[36] + `1A 00` | ASCII banner `Pipeworks bundle vX.YY (big endian)` padded with spaces |
| 0x26 | char[] | bundle name, padded with `0xEB` |
| 0x4C | u32 | total file size |
| 0x50 | u32 | type count |
| 0x54 | u32 | type table offset |
| 0x58 | u32 | type table size |
| 0x5C | u32 | entry count |
| 0x60 | u32 | entry table offset |
| 0x64 | u32 | secondary count |
| 0x68 | u32 | secondary table offset |
| 0x70 | u32 | data offset |
| last two words before the type table | u32, u32 | name-table offset, name-table size (0x74 in v1.20, 0x7C in v1.30) |

**Entry** (16 bytes): `u32 offset`, `u8 flags1 + u24 storedSize`,
`u8 flags2 + u24 rawSize`, `u32 hash`. When `storedSize != rawSize` the entry is
presumably compressed (the codec isn't known yet, and the extractor reports it).

**Name table**: `char[32] tag`, `u32 recordCount`, `u32 stringCount`, then
`recordCount × {u32 hash, u16 dirStr, u16 nameStr, u16 extStr, u16 _}`, then
`recordCount × u32 hash`, then `stringCount × u32 stringOffset`, then the string
blob. An entry's path is `dir/name.ext`, joined through the shared `hash`.
Several entries can share one hash (chunked payloads).

## 2. Texture container (`T32` / `TM2`)

Embedded inside model and room files, and also loose (`.t32`, `.tm2`) and
inside packs (§9). A model's `texIndex` counts from 0 within one container.

The first four bytes are the fixed sequence `00 32 33 54` (`T32`) or
`00 32 4D 54` (`TM2`) on **both** PS3 and PC. They are the ASCII tag
reversed, which looks like a byte-order hint but is not one: the PC files put
these same bytes in front of little-endian fields. Byte order therefore comes
from the header fields, never from the magic. (Our reader also accepts the
ASCII form `T32\0` in case another build uses it.)

| Off | Type | Field |
|---|---|---|
| 0x00 | u8[4] | magic |
| 0x04 | u32 | image count `n` |
| 0x08 | u32 | header size |
| 0x0C | u32 | data size |
| 0x10 | … | image headers |

### 2.1 PS3 layout: `PS3-community`

The header size is exactly `0x10 + 0xA0·n`. **Image header** (0xA0 bytes,
relative to its start): `+0x00 u32 index`, `+0x24 u32 dataSize`,
`+0x38 u8 format`, `+0x40 u16 width`, `+0x42 u16 height`. Raw pixel data for
all images follows the headers back to back, in header order.

`format & 0x0F`: `5` = ARGB8888, `6` = DXT1 (BC1), `8` = DXT5 (BC3).
`format & 0x20` means no mipmaps. The HD port re-encoded the art at 2× PS2
resolution. UVs are normalised, so that makes no difference to meshes.

### 2.2 PC layout: `PC-verified` (all 321 loose `.t32`/`.tm2` files parse)

- Header fields are little-endian.
- Image header slots are **0xA8** bytes, and the header block is padded to a
  multiple of 16. The header size lies in `[0x10 + 0xA8, align16(0x10 + 0xA8·n)]`.
- Some containers have fewer slots than images (`mssn_f00.tm2`: `n = 2`, one
  slot), so the count, not the slots, says how many images follow.
- Each image is a **complete DDS file** (`"DDS "` magic, 124-byte header, all
  mip levels), per Microsoft's public DDS documentation. Images follow the
  header back to back; when a DDS file's size leaves the next one unaligned,
  the next one starts at the next multiple of 16.
- The per-slot size field (`+0x2C`) does not always match the DDS size
  (`em10`), so a reader walks the DDS files by their own computed size.
- Pixel formats seen: DXT1, DXT3, DXT5, and 32-bit BGRA
  (`masks R=0x00FF0000, G=0x0000FF00, B=0x000000FF, A=0xFF000000`).
  Rooms use DXT3 heavily; PS3 notes don't mention it.

## 3. Model file sections

`.pld` (player), `.pws`/`.pwd` (weapon), `.emd` (enemy) and `.fsd` (room)
start with a table of section offsets. An offset of `0` means an empty
section.

### 3.1 PS3: `PS3-community`

A flat list of `u32` offsets, running until the first section starts:

- Player bodies `pl00`, `pl05`: the list starts at 0.
- Devil Trigger bodies (`pl01`, `pl03`, `pl06`) and all enemies: a 0x800-byte
  texture directory comes first, the list starts at 0x800, and offsets are
  relative to 0x800.

### 3.2 PC: `PC-verified` (all 73 `.pld/.pws/.pwd/.emd` and all 106 `.fsd`)

| Files | Table |
|---|---|
| `.pld/.pws/.pwd/.emd` | `u32 count`, then `count × u32 offset` (Layout `Counted`) |
| `.fsd` | `u32 count` (always 35), 4 bytes `0xCC`, then `count × u64 offset` (Layout `Counted64`) |

There is no texture directory; offsets count from the start of the file.
Section offsets are not always in ascending order (`r002.fsd` sections 25
and 26), so a section ends at the next-highest offset, not at the next entry.

Section 0 holds the geometry of characters, weapons and enemies. Rooms keep
theirs in section 14 (§4b). The PS3 notes put the motion banks in sections 6
(body) and 7 (coat) for players and 3 and 5 for enemies. On PC, `em00`
section 5 parses with the PS3 motion layout, but `pl00` section 6 and `em00`
section 3 do not yet (tracked in `COVERAGE.md`).

## 4. Geometry section (characters, weapons, enemies)

Offsets are relative to the section start (`base`).

### 4.1 PS3: `PS3-community`

```
u8  objectCount
u8  boneCount
u8  texCount
u8  _
u32 skeletonOffset
object[objectCount]   stride 16:
    u8  meshCount
    u8  0
    u16 totalVerts        (sum over its meshes)
    u32 meshDescOffset
    8 bytes unknown
mesh descriptor       stride 32:
    u16 numVerts
    u16 texIndex
    u32 posOffset         f32×3 per vertex
    u32 nrmOffset         f32×3 per vertex
    u32 uvOffset          i16×2 per vertex, /4096, V flipped
    u32 boneOffset        4 bytes per vertex (bytes 1..3 = bone index × 4)
    u32 weightOffset      u16 per vertex (bit 15 = strip restart; low 15 bits =
                          three 5-bit weights)
    8 bytes unknown
```

All meshes of one object share their attribute arrays back to back. Positions
of mesh 0, then mesh 1, and so on, then all normals, and so on. That packing
plus `totalVerts` is a strong validity check. Primitives are **triangle strips**,
and a vertex whose weight word has bit 15 set starts a new strip. `0xCDCDCDCD`
is filler between blocks. Vertices are in model space, already in bind pose.

### 4.2 PC (`Pc64`): `PC-verified` (73/73 models)

The same records, widened for a 64-bit build. Every offset becomes a `u64`,
and the 4 bytes between the small fields and the first offset are `0xCC`
padding.

```
header (16 bytes):
    u8 objectCount, u8 boneCount, u8 texCount, u8 _
    4 × 0xCC
    u64 skeletonOffset
object[objectCount]   stride 24:
    u8 meshCount, u8 0, u16 totalVerts
    4 × 0xCC
    u64 meshDescOffset
    8 bytes unknown
mesh descriptor       stride 56:
    u16 numVerts, u16 texIndex
    4 × 0xCC
    u64 posOffset, nrmOffset, uvOffset, boneOffset, weightOffset
    8 bytes unknown
```

Two differences beyond widening:

- Attribute arrays are packed **mesh-major**: all arrays of mesh 0, then all
  arrays of mesh 1. Each array starts on a 16-byte boundary (`0xCD` filler).
- The skeleton header keeps its four `u32` fields (§5), but its three offsets
  count from the **skeleton header**, not from the section.

Per-vertex encodings (positions, normals, UVs, bone bytes, weight words) are
unchanged.

## 4b. Room geometry (`.fsd` section 14): `PC-verified`

Rooms use their own records, unlike §4. Offsets are relative to the section.
`dmc room` converts all 106 rooms (21,053 objects, 3.2 M vertices, 2.0 M
triangles); renders of `r002` (Dante's office) and `r100` (the castle hall)
match the game, with backface culling on, which confirms the up axis (+Y) and
the winding rule below.

```
header (16 bytes):
    u16 objectCount          (16-bit: 29 rooms have more than 255 objects)
    u8  texCount             (images in section 34)
    u8  _
    4 × 0xCC
    u64 matrixOffset         (objectCount 4×4 matrices, one per object)
object[objectCount]   stride 40:
    u8  meshCount
    u8  0
    u16 totalVerts           (sum over its meshes)
    4 × 0xCC
    u64 firstMeshOffset
    8 bytes flags            (unknown; e.g. 00 20 04 00 00 06 00 00)
    i16 × 6                  world bounds: x max, x min, y max, y min, z max, z min
    4 × 0xCD
```

The object table ends where the first mesh begins.

**Placement.** Vertices are in object space. Object *i* is placed by matrix
*i* at `matrixOffset + 64·i`: 16 `f32`, row-major, `world = M · [x, y, z, 1]`
(translation in the last column, bottom row `0 0 0 1`). The stored bounds
check this: for 99.5% of objects, the transformed vertices span exactly the
stored bounds (±2 units). The bounds are `i16` and wrap in the large rooms
(coordinates reach about ±50,000), so they compare modulo 65,536. The other
0.5% (102 objects, about half of them in `r10c`, `r30a`, `r408` and `r40d`)
disagree for a reason not known yet.

Meshes of an object form a **chain**: each mesh is a 32-byte descriptor,
16-byte aligned, followed by its own vertex arrays, and the next mesh starts
where this one ends.

```
mesh descriptor (32 bytes), fields in 16-byte units from the descriptor:
    u16 numVerts
    u16 texIndex             (into the room's texture container, section 34)
    u16 posAt                (always 2: arrays follow the descriptor)
    u16 nrmAt
    u16 uvAt
    u16 colourAt
    u16 _                    (always 0)
    u16 nextAt               (the next mesh, or the end of the chain)
    8 × 0x00, 8 × 0xCD
arrays (each padded to 16 with 0xCD):
    pos    f32×3 per vertex
    nrm    f32×3 per vertex
    uv     i16×2 per vertex, /4096 (as §4)
    colour 4 bytes per vertex: flags, then three colour bytes
```

The first byte of each colour entry drives the triangle strip:

| Bit | Meaning |
|---|---|
| 0 | winding: 1 → triangle `(i-2, i-1, i)` is front-facing as written; 0 → reversed |
| 1 | restart: this vertex does not close a triangle |
| 2 | always set |
| 3 | unknown (set on ~15% of vertices) |

Checked on 30 rooms (5,645 meshes): the first two vertices of every mesh have
bit 1 set, and the winding bit agrees with the vertex normals on 99.4% of 450k
triangles. The rest are probably curved surfaces, where the smoothed normals
disagree with the flat face.

The three colour bytes are vertex lighting. About a third of meshes are
grey; the rest are coloured, and the first byte runs lower than the other
two, but the channel order and scale (values reach 255) are not confirmed.
`dmc room` exports them as-is, divided by 255.

Normals are unit length except in `r40b`, where they are scaled by about
1.8·10⁻⁵ (directions intact), and in `r305`, which has 4 NaN normals.

## 4c. Other room sections: `speculative`

Every `.fsd` has 35 section slots. From `r002.fsd` and a survey of all 106
rooms (62 distinct sets of non-empty sections):

| Section | Size in r002 | Observation |
|---|---:|---|
| 0 | 0x80 | small parameter block (floats, `0xFF` runs) |
| 1 | 0x90 | floats (e.g. 32.0, 48.0, 96.0) |
| 3 | 0x2C00 | float records: positions, unit vectors and 3×3 rotations. **Camera candidate** |
| 4, 5 | 0x6300 each | same header shape as each other. **Collision candidate** |
| 9 | 0x32C0 | 32-byte records of i32 coordinates and small ints. **Trigger/area candidate** |
| 14 | — | room geometry (§4b) |
| 15 | 0x30 | a permutation of 0..0x20 (draw or object order?) |
| 17 | 0x400C90 | texture container: 8 × 512² DXT5 |
| 18 | 0x14D0 | offset pack (§9), same shape as `Etc/effcom.anm` |
| 19 | 0x22F80 | same header shape as `Etc/def_efm.omd` (effect models?) |
| 22–29 | ~0x5B0–0xCE0 | offset/size packs (§9) of messages in the same glyph encoding as `.msg`. Probably the room's text in several languages |
| 30 | 0x206470 | **event package**: a `PLAYER DATA` block, then fixed 64-byte path records naming the enemy models used (`../data/emd/em20.emd`), the room's event scripts (`../event/stage0/r0020107.ecd`, …) and per-enemy motion curves (`../event/motion/em20/em200107.fcv`, …) |
| 34 | 0x372A10 | texture container used by the geometry (`texCount` images) |

## 5. Skeleton: `PS3-community`, layout `PC-verified`

At `base + skeletonOffset`:

```
u32 hierarchyOffset     → u8 parent[boneCount] (0xFF = root)
u32 ikFlagsOffset       → u8 flag[boneCount]   (IK chain markers, see §7)
u32 transformsOffset    → boneCount × {f32 x, y, z (offset from parent), f32 length}
u32 boneCount
```

On PC the three offsets count from the skeleton header itself (§4.2).
Bones have no names. We call them `bone00`, `bone01`, and so on, in file order.

## 6. Motion bank: `PS3-community`

```
u32 count
u32 0
count × { u32 motionOffset, u32 eventOffset }     (from bank start)

motion (at motionOffset):
    u16 frames
    u8  channelCount - 1
    u8  _
    (channelCount - 2) × u8 channelId, padded to 4
    channelCount × u32 channelOffset (from motion start; ≥ eventOffset - motionOffset → empty)

channel = 3 subtracks (x, y, z), each:
    u16 n
    n × u16 frame
    pad to 4
    n × { f32 value, f32 inTangent, f32 outTangent }   cubic Hermite
```

Channel 0 is a rotation that is not applied to the body. Channel 1 is root
motion (a translation added to bone 0). Every other channel id is
`bone | 0x80·second`, where the first channel of a bone is Euler rotation
(X, then Y, then Z, in radians). The second channel is:

| Bone kind | Second channel |
|---|---|
| bone 0 | translation |
| IK root | knee hinge axis (in root's parent space) |
| IK end | target position (model space) |
| flag 8 | translation |
| other | scale |

Motions are keyed at **60 fps**. On PC, some banks parse with this layout and
others do not yet (§3.2).

## 7. Leg IK: `PS3-community`

The skeleton's flag table marks two-bone chains. A root flag (`3, 4, 5, 6, 0x11,
0x12, …`) is followed by two bones flagged `1` (middle, end). The motion stores
the end target and a hinge axis, not joint rotations. At runtime the chain is
solved with the law of cosines in the plane normal to the hinge. Odd and even
root flags bend in opposite directions (mirrored legs), and an unreachable
target straightens the chain towards it. The end bone's rotation channel is
already in model space.

The event table (`eventOffset`) isn't documented yet. It probably holds
hitbox, SFX and effect triggers, which Phase 6 will need.

## 8. Unknown / to discover on PC

Room collision, camera placement, room scripts and triggers and enemy spawn
tables are not decoded yet; §4c lists the candidate room sections. Track them
in [`COVERAGE.md`](COVERAGE.md).

## 9. Packs: `PC-verified` (headers), contents partly known

Many `.dat`, `.itm`, `.bnd`, `.anm` and `.msg` files, and several room
sections, are simple tables of payloads with no magic. Two shapes occur:

| Shape | Header | Seen in |
|---|---|---|
| **offset pack** | `u32 count`, `count × u32 offset`, padded to 16. Payload `i` runs to the next-highest offset or the end of the file | 109 of 119 `.dat` (`memcard*.dat`, `item_st*.dat`, `memicon.dat`, `memtex.dat`, …), 7 `mssn_clr.bnd`, 3 `.anm` (`effcom.anm`), room section 18 |
| **offset/size pack** | `u32 count`, `count × {u32 offset, u32 size}`, padded to 16 | all 69 `.itm`, all 9 `.msg`, 8 `act_init.dat`, 2 `.anm` (`efflife.anm`), room sections 22–29 |

The first offset always equals the padded header size, which makes both shapes
easy to recognise. Payloads found so far are TM2 containers (§2) in the
`.dat`/`.bnd`/`.itm` UI packs, and messages in `.msg`. Messages are strings of
glyph indices with control bytes (`0x7E` separators, `0x05 xx` and
`0x0E 00 50` sequences). The encoding is not decoded yet.

Not packs: the other 7 `.bnd` (`mssn_ini.bnd`) are bare TM2 containers.
`Etc/first.dat` (starts with `u32 0`), `def_efm.omd` and `effcom.eca` are
still unknown.

## 10. `ipum` image sequences (`.ip2`): `PC-verified` (headers)

The PS2 game played short clips with the IPU (MPEG-2 intra frames). The HD
port replaced each clip with a sequence of DDS frames:

```
char[4] "ipum"
u16 _ (0), u16 _ (1)
u16 width, u16 height
u32 frameCount            (30 to 120)
frameCount × {
    char[4] "frmj"
    u32 size
    DDS file (size bytes)
}
```

All 470 files walk exactly: the frame count matches and the last frame ends
at the end of the file.

440 of the 470 `.ip2` files are `mssn*` mission title cards, across the
per-language `status_*` directories.

## 11. Audio and video: `PC-verified` (containers only)

- `audio/*.bank` start with `RIFF`, then `FEV ` at offset 8: **FMOD Studio**
  banks. `MasterBank.strings.bank` holds the event names. Decoding the sample
  data (FSB5 inside the bank) is not implemented yet.
- `Video/*.wmv` are ASF files (GUID `30 26 B2 75 8E 66 CF 11 …`), i.e. Windows
  Media Video. Standard decoders (FFmpeg and others) read them.
