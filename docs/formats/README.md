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
(body) and 7 (coat) for players and 3 and 5 for enemies, and the PC files
agree: `pl00` has 11 sections (0 geometry, 6 body motions, 7 coat motions,
8 the T32 textures), and `em00` keeps motions in 3 and 5 (§6.1).

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
mesh descriptor       stride 56 (room props, §4f: stride 80):
    u16 numVerts, u16 texIndex
    4 × 0xCC
    u64 posOffset, nrmOffset, uvOffset, boneOffset, weightOffset
    8 bytes unknown (props: u64 colourOffset, 4 bytes/vertex,
                     then 8 zero bytes and 16 filler bytes)
```

Two differences beyond widening:

- Attribute arrays are packed **mesh-major**: all arrays of mesh 0, then all
  arrays of mesh 1. Each array starts on a 16-byte boundary (`0xCD` filler).
- The skeleton header keeps its four `u32` fields (§5), but its three offsets
  count from the **skeleton header**, not from the section.

Per-vertex encodings (positions, normals, UVs, bone bytes, weight words) are
unchanged, except that V is used as stored rather than flipped: the DDS
images are top-down, and with the flip Dante's hair samples the skin region
of his atlas (checked by eye).

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
    uv     i16×2 per vertex, /4096; V is flipped for sampling (1 − v)
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

**Texture V.** Room UVs sample correctly with V flipped (`1 − v`), unlike
the PC character models (§4.2), which use V as stored. Evidence: the
lettering on the rug in `r002` reads the right way round only with the flip.

The three colour bytes are vertex lighting. About a third of meshes are
grey; the rest are coloured, and the first byte runs lower than the other
two, but the channel order and scale (values reach 255) are not confirmed.
`dmc room` exports them as-is, divided by 255.

**Channel order, evidence so far.** Two readings are possible: R, G, B as
stored, or B, G, R, if the entry is a byte-reversed RGBA word. That fits the
flag byte coming first, and the PC files keep other words reversed (the
texture magics).
- **For B, G, R:** `r100`'s two chandeliers. Their vertices read
  (0.6, 1.0, 1.0) as stored, and the room's own light records (§4h) put
  lights of colour (255, 255, 130) exactly there. Read as B, G, R, the
  fixture glows the colour of its light.
- **Fog colour:** across 83 rooms it leans the B, G, R way, but only weakly
  (correlation +0.29).
- **Inconclusive:**
  - across the archive, the fixtures of strongly tinted lights match the
    B, G, R reading 101 times and the stored order 87 times;
  - the vertex lighting around them matches 127 against 117;
  - the ambient and key colours show no relation.

The order stays provisional until a capture of the original settles it.
Colour identity is one of the things the remake must keep
(`docs/REMAKE.md`).

Normals are unit length except in `r40b`, where they are scaled by about
1.8·10⁻⁵ (directions intact), and in `r305`, which has 4 NaN normals.

## 4c. Other room sections: `speculative` unless noted

Every `.fsd` has 35 section slots. From `r002.fsd` and a survey of all 106
rooms (62 distinct sets of non-empty sections):

| Section | Size in r002 | Observation |
|---|---:|---|
| 0 | 0x80 | small parameter block (floats, `0xFF` runs); always 0x80 |
| 1 | 0x90 | floats (e.g. 32.0, 48.0, 96.0); always 0x90 |
| 3 | 0x2C00 | fixed 11,264 bytes (105 rooms): **triggers**, doors among them (§4g) |
| 4, 5 | 0x6300 each | **light sets** (§4h): whole 3,168-byte sets, 1–8 per section; section 5 is missing in 6 rooms |
| 9 | 0x32C0 | **collision** (§4d, `PC-verified`), then an object culling tree and a table not parsed yet |
| 14 | — | room geometry (§4b, `PC-verified`) |
| 15 | 0x30 | a permutation of 0..0x20 (draw or object order?); always 0x30 |
| 17 | 0x400C90 | texture container: 8 × 512² DXT5 |
| 18 | 0x14D0 | offset pack (§9), same shape as `Etc/effcom.anm` |
| 19 | 0x22F80 | 0 to 11 MB. **Room props** (§4f) |
| 21 | — | in 51 rooms: `u32 count`, offsets, then records of keyframed channels (`u32 keys = 2`, `u32 frame = 299`, values). Channel values include 55.0 and 45.0 at the end, so probably **event-camera curves** |
| 22–29 | ~0x5B0–0xCE0 | offset/size packs (§9) of messages in the same glyph encoding as `.msg`. Probably the room's text in several languages |
| 30 | 0x206470 | **event package**: a `PLAYER DATA` block, then fixed 64-byte path records naming the enemy models used (`../data/emd/em20.emd`), the room's event scripts (`../event/stage0/r0020107.ecd`, …) and per-enemy motion curves (`../event/motion/em20/em200107.fcv`, …) |
| 34 | 0x372A10 | texture container used by the geometry (`texCount` images) |

## 4d. Room collision (`.fsd` section 9): `PC-verified` (106/106 rooms, 62,614 polygons)

A box tree whose leaves hold the collision polygons. It starts at offset 0 of
section 9. Other data follows it.

```
header (16 bytes):
    u32 topNodeCount
    u32 polygonCount
    u32 _ (always 1)
    u32 _ (always 0)
node (32 bytes):
    i32 centre x, y, z        room units
    i16 halfExtent x, y, z
    u16 childCount            0 = leaf
    u32 count                 a leaf: polygons that follow it
    u32 span                  bytes from this node to the end of its subtree;
                              0 on a last sibling
    u32 firstPolygon          a leaf: index of its first polygon
polygon (48 bytes), right after its leaf:
    u32 0xFFFFFFFF
    i16 × 3 × 4               vertices, relative to the leaf's centre
    u32 flags
    f32 nx, ny, nz            unit normal
    f32 _                     usually 0 (non-zero on 6% of polygons)
```

Top-level nodes follow the header one after another. A node's children, or a
leaf's polygons, follow the node. Checks that hold in every room: the
polygon indices form a permutation of `0..polygonCount`, and every non-zero
span equals the bytes the subtree actually occupies.

A polygon is a quad `(v0, v1, v2, v3)` drawn as triangles `(0, 1, 2)` and
`(2, 1, 3)`. It is a triangle when `v3 == v2` (about half of them). `(v0, v1,
v2)` winds counter-clockwise around the stored normal on 62,523 of 62,530
non-degenerate polygons. 4,262 quads are not planar within 2 units.

Flag bits, compared against the normal (floor `ny > 0.7`, ceiling
`ny < −0.7`, wall `|ny| < 0.3`):

| Bit | Seen on |
|---|---|
| `0x1` | floors only (11,409) |
| `0x2` | floors (5,737), a few slopes |
| `0x10` | walls, ceilings, slopes (40,414); almost never floors |
| `0x40000` | ceilings only (6,825) |
| `0x1000` | most polygons of every kind (51,950) |
| others (`0x8`, `0x20`, `0x40`, `0x80`, `0x100`, `0x200`, `0x8000`, `0x10000`, …) | not understood; surface materials or special volumes are likely |

Renders of `r100` (the castle hall) show the walls, pillars, stairs, the
statue's plinth and the chandeliers' hulls in place over the room geometry.

## 4e. Cameras (`.fsd` section 2): layout `PC-verified`, meaning partly inferred

Present in 98 rooms (560–46,480 bytes); absent from the prologue rooms
(`r002`, `r00d`, `r00e`) and from `r114`, `r318`, `r508`, `r50a`, `r50c`.
Parsed by `dmc_formats::camera` (`Room::cameras`): 1,575 cameras in 97 rooms.

Header (16 bytes): `u32 count`, 4 bytes of leftover memory, the tag
`ver`, 4 more leftover bytes. Only `r503` lacks the tag: it uses an
older, smaller record layout that is not handled yet.

Records follow the header back to back. A record is 0x220 bytes, plus
`16·n` rail bytes when `flags & 0x10`. This walks every version-2 room
exactly. Offsets are from the record start:

| Off | Field | Reading |
|---|---|---|
| 0x00 | f32×4 A (w = 1) | activation-zone corner |
| 0x10 | f32×4 B (w = 1) | opposite zone corner |
| 0x20 | 6 × f32×4 (w = 0) | outward unit normals of the zone's faces: faces 0–2 pass through B, faces 3–5 through A. The zone is the convex hexahedron they bound |
| 0x80 | f32×4 | look-at offset, usually `(0, 900, 0)`: Dante's head height (Dante ≈ 900 units tall) |
| 0x90 | f32×4 | the eye: where a camera without rails views from; equals the last point of the eye rail |
| 0xA0 | f32×4 | equals the last point of the track rail |
| 0xC0 | f32×4 | unknown point |
| 0xD0 | u8[128] | 750 of 1,575 cameras have all `0x01` and about 146 all `0x00`; the rest mix in `0xFF` and values like 10–200 in steps of 10 (100, 60, 50 are the most common). **Not occluder flags**: taking byte *k* as room object *k*, objects marked `0xFF` cross the line from the eye to the zone centre as often as those marked `0x01` (2.4% against 2.1%). Percent-like parameters per slot (lights? object groups?), unknown |
| 0x150 | u8[8] | type bytes, e.g. `00 00 00 01 02 02 6e 00`. Bytes 4 and 5 are the point counts of the eye and track rails (all 1,181 rail records; 310 records without rails carry counts too). The rest are unknown |
| 0x158 | u32 flags | `0x10`: the record ends with rails |
| 0x15C | u32 n | rail point count (both rails together) |
| 0x168 | f32, f32 | arc lengths of the eye and track rails; they match the points exactly |
| 0x180 | f32 | 55.0 in 44 of 53 `r100` records (also 54.6, 49.7, 42.6, 39.9, 25.1): the field of view in degrees, probably |
| 0x190 | i32×4 | usually −1 (links to other cameras?) |
| 0x220 | n × f32×4 | the eye rail (n/2 points; fourth component −0.65 to 1.12, median ≈ 0, unknown), then the track rail (n/2 points, w = 1) |

Evidence for the zone reading: in 1,574 of 1,575 records, A and B lie on
the zone's boundary. Across the 97 rooms, 87% of floor collision polygons
(§4d), sampled 100 units above their centres, fall inside at least one zone.
Arbitrary boxes would not cover the walkable floor like that. The rest are
probably floors the player can't reach (tops of props, ledges).

Evidence for which point is the eye. An earlier reading of these notes had
the two rails the other way round. It was tested against the data as
follows:
- Floor points were sampled inside each camera's zone (5×5 grid, lowest
  collision floor within the zone's height). This gives 22,803 samples
  under the 1,181 rail cameras and 5,678 under the others, in 97 rooms.
- A 900-unit player was placed at each sample, and the check was whether
  its middle falls inside the view (vertical FOV, 4:3).

| Reading | Player in view | Median eye distance |
|---|---|---|
| eye on the **first** rail, aimed at the player's head | 99.7% | 4,794 |
| eye on the first rail, aimed at the matching track point | 83.0% | 4,383 |
| eye on the second rail, aimed at the player's head | 87.5%\* | 2,474 |
| eye on the second rail, aimed at the matching first-rail point | 3.3%\* | — |
| no rails: eye at **+0x90**, aimed at the player | 99.5% | 4,761 |
| no rails: eye at +0xA0, aimed at the player | 94.2% | 4,616 |
| no rails: eye at +0x90, aimed at +0xA0 (never turning) | 49.8% | — |

\* first 12 rooms only.

Every variant here takes the fraction from the player's nearest point on
the track rail. Nearest in 3D versus in XZ, and by arc length versus by
segment index, all score within 1.5% of each other. The track rail runs
near the player's path at about chest height: the player's head is a median
1,593 units from it (90th percentile 4,612).

So, provisionally:
- The eye moves along the first rail. Its position on that rail is the
  fraction of the track rail's length at which the player's nearest track
  point lies.
- Cameras without rails stay at +0x90.
- Both kinds turn to keep the player in view.

Aiming at the matching track point, not at the player, loses the player in
17% of samples. Whatever the original aims at, it stays close to the
player.

Still to confirm against the running game:
- how the original blends between cameras;
- what the eye rail's fourth component, the +0xC0 point, the other type
  bytes and the 128 bytes at +0xD0 mean;
- whether the FOV is vertical or horizontal.

## 4f. Room props (`.fsd` section 19): table `PC-verified`, placement unknown

Parsed by `dmc_formats::props` (`Room::props`). The table walks exactly,
ending where the first geometry starts, in all 101 rooms that have the
section: 1,190 props and 8 empty slots.

```
u32 count
count slots, back to back:
    empty slot: one u32 0xFFFFFFFF
    prop:       u32 geometry offset (section-relative, 16-aligned)
                u32 field1..field5
                texture words (see below)
data (aligned to 16)
```

**Texture words.** If the first texture word has its top bit set
(`0x8000_0000 | image`), the prop uses the room's own textures (section 34).
There is then one word per texture slot, and the count is byte 2 of the
geometry header: `r100` prop 0 has two slots, `R20 R27`. Otherwise there is
exactly one word:
- `0xFFFFFFFF`: no texture;
- an offset to an embedded `T32` container holding all the prop's images;
- `0x4000_0000 | offset`: an `ipum` frame sequence (§10), an animated texture.

Across all props: 864 room-image words, 418 embedded containers, 57
sequences and 81 without a texture.

**Fields 1–5.** Only partly read:
- Field 1 is −1 or an offset to motion-like data (217 start with a `u32`
  count of 2).
- Field 2 is occasionally another offset.
- Field 3 is always −1.
- Field 4 is a second geometry in 30 props.
- Field 5 holds references (top bit set) or small records.

**Geometry.** The §4.2 records, except that mesh descriptors are 80 bytes,
not 56. They add a sixth array of 4 bytes per vertex, which by size and
position matches the room meshes' vertex colours. Every one of the 1,175
multi-mesh prop objects walks correctly only with the 80-byte stride.
`geometry::Variant::Pc64Prop` reads this layout, and all 1,190 geometries
parse. 1,184 have unit normals; the other six are copies of one 112-vertex
model in `r408`/`r40b`. Renders of `r100`'s props (a carved stone block on
room images 20 and 27; three copies of a 17-bone model with an embedded
texture) show coherent shapes and textures.

**Texture V** is taken as stored, as for the PC characters (§4.2), but only
weakly supported. The check:
- Take the triangles on images that are partly transparent.
- Count those whose UV centroid lands on a transparent texel.
- Props: 45,283 of 148,135 as stored, 49,325 flipped.
- Room meshes, as a control: 127,899 of 519,939 as stored, 85,669 flipped.
  They are known to need the flip, so the test points the right way, but it
  separates the two conventions weakly.

Confirm on a prop with lettering.

**Placement is not in this section.** Every prop is modelled around its own
origin (bone 0 at 0,0,0). Several sets of copies share one geometry and
differ only in their texture or motion field (`r100` props 1–3, `r104`
props 3–7). The candidates checked do not hold transforms:
- section 3 is door and trigger records;
- section 21 is keyframed curves;
- sections 0 and 1 are small fixed parameter blocks.

Placement may come from the motion data or from per-room code. Until it is
found, `dmc room` writes each room's props side by side in
`<room>.props.glb`, for inspection.

## 4g. Triggers and doors (`.fsd` section 3): layout `PC-verified`, doors `PC-verified`

Parsed by `dmc_formats::triggers` (`Room::triggers`). The section is exactly
64 records of 0xB0 bytes in 105 rooms; `r114` has none. An earlier reading
took a 0x80-byte header and 63 slots, which paired each head with the next
record's vectors.

```
record (0xB0):
    f32×4 × 8      the volume (below)
    head (0x30):
        u8 kind, u8 sub, u8 flags[2]    kind 0 = empty (5,650 records, flags 00 11)
        u32 0
        f32×3 point                     doors: where the player arrives
        u32
        u16 room                        doors: the room they lead to (r%03x)
        u8 extra                        doors: 0, 2 or 3
        …                               zero in 842 of 1,070 records
```

**Volumes.** Most records use the camera-zone box (§4e): corners `A` and `B`
(w = 1), then six outward unit normals (w = 0), the first three through
`B` and the last three through `A`. In 986 of 1,070 non-empty records both
corners lie on the box. In 107 records the normal slots hold `(0, 0, 0, 1)`
instead. Vector 0 is then a centre and vector 1 a size such as
`(500, 500, 0)`, so probably a cylinder; `Volume::Round` takes it as one,
unconfirmed.

**Doors.** Doors are the records with sub-kind 0 and a room id: 249 of them,
across kinds 1 (1), 2 (3), 3 (217) and 7 (28). An earlier reading took only
kinds 2 and 3, which missed `r106`'s exits (kind 7) and one in `r110`
(kind 1). Checked against the other rooms (`dmc-cli/tests/game_data.rs`):
- 245 name a room in the archive. The other four name `r00f`, `r105` and
  `r20a` (twice), which it lacks.
- 203 of those 245 arrival points lie within 150 units of a floor in the
  target room's collision (§4d).
- 205 target rooms have a door leading back. For 134 of them, the arrival
  point is within 800 units of that return door's volume, so you arrive
  beside the door you'd use to go back.

`r100` alone has doors to `r101`, `r11b`, `r110`, `r116` and `r106`. Records
with other sub-kinds (1–14) sometimes carry a value in the room-id slot too,
but those values rarely name a room in the archive, so they probably mean
something else. The other trigger kinds and sub-kinds are not identified yet;
their heads carry small integers or points instead. Whether the original
opens a door on contact or on a button press, and which way the player faces
on arrival (perhaps the `extra` byte), is still to be checked against
captures.

## 4h. Light sets (`.fsd` sections 4 and 5): layout `PC-verified`, meaning partly understood

Parsed by `dmc_formats::lights` (`Room::light_sets`). Both sections are
whole sets of 0xC60 bytes: 216 sets in section 4 over 106 rooms, and 241 in
section 5 over 100. Rooms carry one to eight sets per section, and the two
sections are identical in only five rooms. Which set applies when (mission
state, geometry or characters) is not known yet.

```
set (0xC60):
  header (0x60):
    u32×8          not understood (zero in some rooms)
    f32×3, pad     ambient colour, 0–255             r100: 25.5 grey
    f32×3, pad     a key colour, 0–255 (role unconfirmed)
    f32×4          fog: amount at near, amount at far (0–255), near, far
                   (room units)                      r100: 0, 220, 7,700, 114,900
    u32×3, pad     fog colour, 0–255                 r100: 40, 38, 33
  64 light slots (0x30 each; empty slots are all zero):
    f32×3          position (room units)
    f32            1 / near²
    f32×3          colour, 0–255
    u32            type: low byte = kind, top byte = flags (0x80 on some)
    f32×3          colour / 255
    f32            1 / far²
```

Each of the 20,800 lights stores its colour twice, and the second copy
always equals the first divided by 255, which pins the record layout.

**Kinds:**
- 3 (5,131 lights) and 4 (3,388) are most placed lights.
- 0 (10,196) marks slots whose position lies on the Y axis, (0, y, 0),
  with white colour; their meaning is unknown.
- The others are 1, 2, 5, 6, 7 and 15.

**Falloff.** The two falloff terms read as distances with near < far for
point lights: `r100`'s chandelier lights are 2,500 and 3,260 units, its
torches 1,900 and 2,800.

**In `r100`:** the float colours are warm for torches (255, 122, 57) and
chandeliers (255, 255, 130), and cool for a few high lights by the windows
(60, 100, 150). Floats have no byte-order doubt, so these lights are the
most direct record of the original's lighting design. The remake's lighting
starts from them (`docs/REMAKE.md` §5).

`dmc room` writes every set as `<room>.lights.json`.

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

Motions are keyed at **60 fps**.

### 6.1 PC motion banks: `PC-verified` (`pl00` §6: 215 motions, §7: 11; `em00` §3: 84, §5: 5)

The bank layout is the same as above: `u32 count`, `u32 0`, pairs of `u32`
motion and event offsets, and 32-bit channel tables. The difference is
channel 0, which is not a Hermite rotation on PC. It holds one or more
blocks, placed before the motion's event table:

```
u16 count
u16 flags                 (0x0000, 0x8000, or uninitialised 0x4444)
count × u32 word          (steps by 0x40 per frame; flag bits such as 0x08000000)
```

Motions that share a body (the same motion offset, different event offsets)
see a different number of these blocks before their own event table. What the
words mean is not known yet.

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

Camera placement, room scripts and triggers, and enemy spawn tables are not
decoded yet; §4c lists the candidate room sections. Collision is §4d. Track
them in [`COVERAGE.md`](COVERAGE.md).

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
