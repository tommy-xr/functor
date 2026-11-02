# Terrain sample assets

`heightmap.png` is generated deterministically by `generate-heightmap.mjs` and is
checked in, as are the maps `generate-colormaps.mjs` bakes from it (colormaps,
normal map, splat). The detail albedo maps are fetched by `npm run fetch:assets`
and are gitignored.

| File | Source | License |
| --- | --- | --- |
| `detail-low.jpg` | [Forest Ground 04](https://polyhaven.com/a/forest_ground_04) — Poly Haven | CC0 |
| `detail-high.jpg` | [Aerial Grass Rock](https://polyhaven.com/a/aerial_grass_rock) — Poly Haven | CC0 |
| `detail-rock.jpg` | [Rock Face](https://polyhaven.com/a/rock_face) — Poly Haven | CC0 |
| `detail-snow.jpg` | [Snow 02](https://polyhaven.com/a/snow_02) — Poly Haven | CC0 |

The `Terrain.materials` layers are checked in as 512² albedo JPEGs,
downsampled with wrap padding so they still tile:

| File | Source | License |
| --- | --- | --- |
| `layer-grass.jpg` | [Forrest Ground 01](https://polyhaven.com/a/forrest_ground_01) — Rob Tuytel, Poly Haven | CC0 |
| `layer-dirt.jpg` | [Ground067](https://ambientcg.com/view?id=Ground067) — ambientCG | CC0 |
| `layer-rock.jpg` | [Rock032](https://ambientcg.com/view?id=Rock032) — ambientCG | CC0 |
| `layer-snow.jpg` | [Snow006](https://ambientcg.com/view?id=Snow006) — ambientCG | CC0 |

CC0 waives attribution, but the sources are recorded here so the maps can be
re-fetched or swapped deliberately.
