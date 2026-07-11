# Asteroids example — asset sources

All assets are **CC0 (Creative Commons Zero / public domain)** by **Kenney** (www.kenney.nl).
No attribution required (crediting Kenney is appreciated but optional).

## Audio

From the **Kenney "Sci-Fi Sounds" pack (v1.0)** — https://kenney.nl/assets/sci-fi-sounds
(zip: `kenney_sci-fi-sounds.zip`; license: CC0, per the pack's `License.txt`).
All files are OGG-Vorbis, 44.1 kHz.

| File | Original pack file | Duration | Use |
| --- | --- | --- | --- |
| `laser.ogg` | `Audio/laserSmall_000.ogg` | ~0.24 s | shoot |
| `explosion.ogg` | `Audio/explosionCrunch_000.ogg` | ~0.78 s | asteroid pop |
| `ship-explosion.ogg` | `Audio/explosionCrunch_002.ogg` | ~1.26 s | ship death |
| `thrust-loop.ogg` | `Audio/thrusterFire_000.ogg` | 5.0 s | thruster loop (steady burn, loopable) |

## Model

| File | Source | License |
| --- | --- | --- |
| `ship.glb` | **Kenney "Space Kit" (v2.0)** — https://kenney.nl/assets/space-kit — `Models/GLTF format/craft_racer.glb` (zip: https://kenney.nl/media/pages/assets/space-kit/20874c75ac-1677698978/kenney_space-kit.zip) | CC0 |

Note: `*.glb` is gitignored in examples — `ship.glb` must be fetched (see
`scripts/fetch-assets.mjs`) rather than checked in.
