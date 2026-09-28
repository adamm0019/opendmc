# Format coverage matrix

This matrix is updated after each `dmc inventory` run on a real install.
Section numbers (§) refer to [`README.md`](README.md). Evidence is the PC
inventory [`docs/inventory/pc-58ed9634.md`](../inventory/pc-58ed9634.md)
(Steam HD Collection, data build `58ed9634`, newest entry 2018-03-01) plus
the checks named in each row.

**Status:** *parsed* (a parser in the repo handles every file of the type),
*partly understood* (container or header known, contents not), *unknown*.

## Containers

| Container / type | Where (PC) | Parser | Status | Evidence |
|---|---|---|---|---|
| `.nbz` archive | `data/dmc1/dmc1-0.nbz` | `dmc-cli::archive` (zip crate) | parsed | plain ZIP, Deflate; 1,190 files listed and read (§0.1) |
| Pipeworks bundle `.BDP` | not on PC | `dmc_formats::bdp` | PS3 only | no bundle in the PC install |
| Offset pack / offset-size pack | `.dat`, `.itm`, `.anm`, `.msg`, `mssn_clr.bnd`, room sections 18, 22–29 | — | partly understood | headers checked on every file of these types (§9) |
| `ipum` image sequence (`.ip2`) | `Etc/`, `status*/` (470) | — | partly understood | all 470 walk exactly: `frmj` frames of DDS (§10) |

## Textures

| Type | Where (PC) | Parser | Status | Evidence |
|---|---|---|---|---|
| `T32` / `TM2` container, PC layout | loose `.t32`/`.tm2` (321), `mssn_ini.bnd` (7), inside models, rooms and packs | `dmc_formats::texture` | parsed | all 321 loose files parse; textures checked by eye (§2.2) |
| DDS (DXT1/3/5, BGRA8) | inside every PC texture container and `ipum` frame | `dmc_formats::dds` | parsed | public DDS spec; unit tests |
| DXT1 / DXT3 / DXT5 / ARGB8888 pixels | inside DDS | `dmc_formats::dxt` | parsed | spec (BCn); unit tests |
| `T32`/`TM2`, PS3 layout | not on PC | `dmc_formats::texture` | PS3-community | synthetic fixtures only |

## Models

| Type | Where (PC) | Parser | Status | Evidence |
|---|---|---|---|---|
| Section table, `Counted` | `.pld/.pws/.pwd/.emd` (73) | `dmc_formats::model` | parsed | 73/73 (§3.2) |
| Section table, `Counted64` | `.fsd` (106) | `dmc_formats::model` | parsed | 106/106: 35 slots each (§3.2) |
| Geometry + mesh tables, `Pc64` | model section 0 | `dmc_formats::geometry` | parsed | `dmc export`: 73/73 to glTF, mean normal length 0.993–1.000 (§4.2) |
| Skeleton + IK flags | model section 0 | `dmc_formats::geometry` | parsed | bone counts match the geometry header on 73/73 (§5) |
| Motion bank | pl sections 6/7, em 3/5 | `dmc_formats::motion` | parsed | `pl00` §6/§7 (215 + 11 motions) and `em00` §3/§5 (84 + 5) parse (§6.1) |
| Motion channel 0 word blocks | motion bank | `dmc_formats::motion` (skipped) | partly understood | block layout known, meaning unknown (§6.1) |
| Motion event table | motion bank | — | unknown | — |

## Rooms (`.fsd`)

| Type | Where (PC) | Parser | Status | Evidence |
|---|---|---|---|---|
| Room geometry + object placement | `.fsd` section 14 | `dmc_formats::room` | parsed | `dmc room`: 106/106 to glTF; 99.5% of 21,053 objects match their stored bounds; renders of `r002` and `r100` checked by eye (§4b) |
| Room textures | `.fsd` section 34 (geometry), 17 (unknown use) | `dmc_formats::texture` | parsed | T32 containers; `texIndex` never exceeds section 34's image count |
| Room vertex colours | `.fsd` section 14 | `dmc_formats::room` | partly understood | channel order and scale unconfirmed (§4b) |
| Room collision | `.fsd` section 9 (start) | `dmc_formats::collision` | parsed | 106/106 rooms, 62,614 polygons; counts, index permutation and subtree spans check in every room; `r100` render (§4d) |
| Collision surface flags | `.fsd` section 9 | `dmc_formats::collision::surface` | partly understood | ground/wall/ceiling bits from normals; the rest unknown (§4d) |
| Object culling tree | `.fsd` section 9 (after collision) | — | partly understood | 40-byte records repeating the objects' i16 bounds (§4c) |
| Lighting | `.fsd` sections 4, 5 | — | unknown | float colours and positions in 3,168-byte blocks (§4c) |
| Door / entry points | `.fsd` section 3? | — | unknown | 63 fixed slots of floor point + facing; not cameras (§4c) |
| Cameras | `.fsd` section 2 | `dmc_formats::camera` | partly understood | 97/98 rooms parse (1,575 cameras; `r503` uses an older layout); zones cover 87% of floor collision; rail arc lengths match; behaviour still to confirm in-game (§4e) |
| Triggers / areas | ? | — | unknown | not located |
| Event package (cutscene actors, `.ecd` scripts, `.fcv` curves) | `.fsd` section 30 | — | partly understood | path records read in `r002` (§4c) |
| Room text | `.fsd` sections 22–29 | — | partly understood | offset/size packs of messages (§4c) |
| Other room sections (0, 1, 15, 18, 19) | `.fsd` | — | unknown | §4c |

## Everything else

| Type | Where (PC) | Parser | Status | Evidence |
|---|---|---|---|---|
| Audio | `data/dmc1/audio/*.bank` (4) | — | partly understood | FMOD Studio banks: `RIFF` + `FEV ` (§11) |
| FMV | `data/dmc1/Video/*.wmv` (31) | — | partly understood | ASF container, standard WMV (§11) |
| Text / strings | `.msg` (9), room sections 22–29 | — | partly understood | offset/size packs; glyph encoding not decoded (§9) |
| UI / HUD images | `status*/` packs, `.ip2`, `.t32`/`.tm2` | `dmc_formats::texture` | partly understood | TM2 payloads inside packs parse (§9) |
| Effects | `Etc/effcom.anm`, `efflife.anm`, `effcom.eca`, `def_efm.omd` | — | unknown | `.anm` are packs; `.eca`, `.omd` unknown |
| `Etc/first.dat` | `Etc/`, `etc_c/` | — | unknown | not a pack (§9) |
| Enemy spawn tables | ? | — | unknown | not located; room section 30 lists a room's event actors only |
