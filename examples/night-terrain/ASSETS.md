# Night terrain sample assets

`night-heightmap.png` is generated deterministically by `generate-heightmap.mjs`
and is checked in.

The four detail maps are checked in as well: 1024² JPGs (quality 85) converted from
the packs' diffuse TGAs. The packs ship only as `.7z` archives, with no per-file URLs,
so `npm run fetch:assets` has nothing to fetch them from.
`Terrain.textured` uses each map for structure only; the band colors in `game.fun`
set the hue.

| File | Terrain band | Source | Author | License |
| --- | --- | --- | --- | --- |
| `night-detail-valley.jpg` | low (valley, lower hills) | `pattern_388/diffuse.tga` from [Free Textures Pack 66](https://opengameart.org/content/free-textures-pack-66) | Nobiax / Yughues | CC0 |
| `night-detail-hills.jpg` | high (upper hills, foothills) | `pattern_391/diffuse.tga` from [Free Textures Pack 66](https://opengameart.org/content/free-textures-pack-66) | Nobiax / Yughues | CC0 |
| `night-detail-rock.jpg` | rock (steep slopes) | `pattern_306/diffuse.tga` from [Free Textures Pack 61](https://opengameart.org/content/free-textures-pack-61) | Nobiax / Yughues | CC0 |
| `night-detail-snow.jpg` | snow (peaks) | `HPST 97 diffuse.tga` from [Free Textures Pack 69](https://opengameart.org/content/free-textures-pack-69) | Nobiax / Yughues | CC0 |

Each pack's bundled readme dedicates it to the public domain under CC0 1.0 Universal.
