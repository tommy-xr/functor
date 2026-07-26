# Shooting-range example — asset sources

All assets are **CC0 (Creative Commons Zero / public domain)** by **Kenney**
(www.kenney.nl). No attribution required (crediting Kenney is appreciated but
optional). Both files are copied unmodified from `examples/asteroids/`.

## Audio

From the **Kenney "Sci-Fi Sounds" pack (v1.0)** — https://kenney.nl/assets/sci-fi-sounds
(zip: `kenney_sci-fi-sounds.zip`; license: CC0, per the pack's `License.txt`).
OGG-Vorbis, 44.1 kHz.

| File | Original pack file | Use |
| --- | --- | --- |
| `shot.ogg` | `Audio/laserSmall_000.ogg` | the weapon report (`Effect.play`) |
| `hit.ogg` | `Audio/explosionCrunch_000.ogg` | the plate ding, played spatially at the impact point (`Effect.playAt`) |

No models or textures: the range, the targets, and the weapon viewmodel are all
built from `Scene` primitives.

The game runs with these files missing — a missing sound logs an error and
plays nothing; everything visual is procedural.
