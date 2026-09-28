# Format coverage matrix

This matrix is updated after each `dmc inventory` run on a real install.
Evidence links point to inventory reports in `docs/inventory/`.

| Container / type | Where (PC) | Parser | Status | Evidence |
|---|---|---|---|---|
| Pipeworks bundle `.BDP` | ? | `dmc_formats::bdp` | PS3-community | — |
| Texture `T32` / `TM2` | inside model files | `dmc_formats::texture` | PS3-community | — |
| DXT1 / DXT5 / ARGB8888 pixels | inside textures | `dmc_formats::dxt` | spec (BCn) | unit tests |
| Model section list (`.pld/.pws/.pwd/.emd/.fsd`) | ? | `dmc_formats::model` | PS3-community | — |
| Geometry + mesh tables | model section 0 | `dmc_formats::geometry` | PS3-community | — |
| Skeleton + IK flags | model section 0 | `dmc_formats::geometry` | PS3-community | — |
| Motion bank | pl sections 6/7, em 3/5 | `dmc_formats::motion` | PS3-community | — |
| Motion event table | motion bank | — | unknown | — |
| Room collision | ? | — | unknown | — |
| Cameras | ? | — | unknown | — |
| Room scripts / triggers / spawns | ? | — | unknown | — |
| Audio | ? | — | unknown | — |
| FMV | `data/dmc1/Video`? | — | unknown | — |
| UI / HUD / fonts | ? | — | unknown | — |
| Text / strings | ? | — | unknown | — |
