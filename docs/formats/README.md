# DMC1 data formats

Our own notes, rewritten from community research and checked against our code.
**Status labels** on each section:

- `PS3-community`: described by community research on the PS3 build and not yet
  seen on PC.
- `PC-verified`: confirmed on a PC install with `dmc inventory` / `dmc export`.
- `speculative`: our hypothesis.

Unless noted otherwise, offsets are in bytes, and multi-byte fields use the
file's byte order (`E`). Primary source for the PS3 facts: the comments in
luismateusvargas/dmc-model-extractor (see `DECISIONS.md`: facts only, no code).

---

## 1. Pipeworks bundle (`.BDP`): `PS3-community`

The HD port's archive. On PS3, DMC1's data sits in `BUNDLES/DMC1.BDP`.

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

**PC check:** the banner text itself says the byte order, so detection needs no
guessing.

## 2. Texture container (`T32` / `TM2`): `PS3-community`

Embedded inside model files. A model's `texIndex` counts from 0 within one
container.

The magic is `"T32\0"` or `"TM2\0"` *stored word-reversed*. On a big-endian
disc the first four bytes are `00 32 33 54` (`T32`) or `00 32 4D 54` (`TM2`).
The same bytes read as a little-endian u32 give `0x54333200`. We try both.

| Off | Type | Field |
|---|---|---|
| 0x00 | u32 | magic |
| 0x04 | u32 | image count `n` |
| 0x08 | u32 | header size (= `0x10 + 0xA0·n`) |
| 0x0C | u32 | data size |
| 0x10 | `n × 0xA0` | image headers |

**Image header** (0xA0 bytes, relative to its start): `+0x00 u32 index`,
`+0x24 u32 dataSize`, `+0x38 u8 format`, `+0x40 u16 width`, `+0x42 u16 height`.
Pixel data for all images follows the headers back to back, in header order.

`format & 0x0F`: `5` = ARGB8888, `6` = DXT1 (BC1), `8` = DXT5 (BC3).
`format & 0x20` means no mipmaps. The HD port re-encoded the art at 2× PS2
resolution. UVs are normalised, so that makes no difference to meshes.

## 3. Model file sections: `PS3-community`

`.pld` (player), `.pws`/`.pwd` (weapon), `.emd` (enemy) and `.fsd` (room props)
start with a flat list of `u32` section offsets:

- Player bodies `pl00`, `pl05`: the list starts at 0.
- Devil Trigger bodies (`pl01`, `pl03`, `pl06`) and all enemies: a 0x800-byte
  texture directory comes first, the list starts at 0x800, and offsets are
  relative to 0x800. An offset of `0` means an empty section.

Section 0 holds the geometry, including the skeleton. The motion banks are
sections 6 (body) and 7 (coat) for players, and sections 3 and 5 for enemies.

## 4. Geometry section: `PS3-community`

Offsets are relative to the section start (`base`).

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

## 5. Skeleton: `PS3-community`

At `base + skeletonOffset`:

```
u32 hierarchyOffset     → u8 parent[boneCount] (0xFF = root)
u32 ikFlagsOffset       → u8 flag[boneCount]   (IK chain markers, see §7)
u32 transformsOffset    → boneCount × {f32 x, y, z (offset from parent), f32 length}
u32 boneCount
```

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

Room collision, camera placement, room scripts and triggers, enemy spawn
tables, audio banks, FMV and UI textures are not yet located. Track them in
[`COVERAGE.md`](COVERAGE.md).
