# Outfits sample assets

The `.glb` presets are built from Quaternius's free (Standard) packs by
`npm run build:characters` (`tools/character-presets/build.mjs`) and fetched by
`npm run fetch:assets` from the `characters-v1` GitHub release. They are
gitignored; `presets.lock.json` pins each file's SHA-256.

There is one glb per body and LOD (`male_lod0.glb` … `female_lod2.glb`). Each
holds every piece as a named part; `Scene.modelParts` picks the loadout.
Without it, every part draws at once: both outfits, all hair and the bare body.

## Sources

All four packs are CC0 1.0. Each archive's own licence file is copied verbatim
into `licenses/`.

| Pack (itch.io, free download) | Archive | Bytes | Archive SHA-256 | Licence file SHA-256 |
| --- | --- | ---: | --- | --- |
| [Universal Base Characters](https://quaternius.itch.io/universal-base-characters) | `Universal Base Characters[Standard].zip` | 128,968,391 | `fdbf1804c90dfc1ea03e992bff7da2dfd1a79318e13270a660180f9308455f40` | `0f4beaf0…` (`License_Standard.txt`) |
| [Modular Character Outfits – Fantasy](https://quaternius.itch.io/modular-character-outfits-fantasy) | `Modular Character Outfits - Fantasy[Standard].zip` | 294,347,394 | `c3468b18871cc8c8f05ab14df7712baf22cb9f389cbd870babf130e595187f70` | `2202cc2f…` (`License_Standard.txt`) |
| [Universal Animation Library](https://quaternius.itch.io/universal-animation-library) | `Universal Animation Library[Standard].zip` | 15,904,933 | `cc73fc4e495b82958207316596317a3f40b9fa38065bde1027937452da537724` | `6d01f55c…` (`License.txt`) |
| [Universal Animation Library 2](https://quaternius.itch.io/universal-animation-library-2) (inspected, unused) | `Universal Animation Library 2[Standard].zip` | 18,735,003 | `4008ea208a604773a2b2177d965f0f5d3195498b5bf838c3f5785d68e95f2a68` | `6d01f55c…` (`License.txt`) |

The build pins every file it reads (`SOURCE_SHA256` in `build.mjs`).

## Free catalogue

- **Bodies:** `Superhero_Male`, `Superhero_Female`. Each has eyes and eyebrows,
  and Dark/Light skin textures.
- **Hair:** six hairstyles and two eyebrow meshes (`Hairstyles/Rigged to Head
  Bone`). Each is fitted to one head: Buzzed, SimpleParted and Beard to the
  male, BuzzedFemale, Buns and Long to the female. The eyebrow meshes repeat
  the bodies' own.
- **Outfits:** Ranger and Peasant, each male and female, split into pieces:
  - Peasant: arms, body, feet, legs.
  - Ranger: arms (+ bracers), body (+ two belts), feet, legs, hood,
    pauldron(s).
  - One extra base-colour variant each (`T_Ranger_3`, `T_Peasant_2`).
- **Clips:** UAL1 and UAL2 have 43 clips each, plus a root-motion copy of
  each. UAL2 also ships a female mannequin.
- **No tactical gear:** a vest, helmet or backpack must be authored.

## Joint contract

All bodies, outfits, hair, and both UALs use one skin. It has the same 65
joints (`root`…`ball_leaf_r`) in the same order and hierarchy, so joint
indices need no remap. The build still remaps by name and fails on a missing
joint.

The bind poses are **not** identical:

| Pair | Difference |
| --- | --- |
| Female body vs female outfits and hair | Identical. |
| Male outfits vs male hair | Identical (the "Regular" male rig). |
| Male body vs male outfits | The outfits fit a narrower "Regular" male body, which isn't in the free tier. Shoulders and hips differ by 17–24 mm. Rest rotations differ by up to 2.5° (clavicles) and 13° (thumbs). Spine, neck and head match exactly. |
| UAL mannequin vs every body | Pelvis 32 mm lower. Rest rotations differ by up to 11° (`spine_03`). |

Hair binds to `Head` only. Every UAL clip keys translation on all 65 joints
using the mannequin's bone lengths. Copied verbatim, a clip would force the
mannequin's proportions onto the character.

To check any set of files against the first one's skin, run
`node tools/character-presets/inspect.mjs <files…>`. It prints skins, joint
order, parents, inverse-bind differences, geometry, textures and clips.

## Pipeline

1. **Canonical skin:** the outfits' skin. The male body is rebound onto it:
   each vertex is posed by its own skin into the canonical bind pose (up to
   36 mm, at the fingers). Joints that bind identically are left untouched,
   so the head, hair and every female piece are exact.
2. **Parts:** one node per part, one primitive per material in it.
   - Outfit pieces become `<outfit>_<slot>`; bracers join the arms and belts
     join the body, so each slot is one toggle.
   - The body splits by skin weight (below).
3. **Clips:** `Idle_Loop`, `Walk_Loop` and `Jog_Fwd_Loop` (no root motion) are
   retargeted to the canonical rig and named `Idle`, `Walk` and `Run`.
   - Each joint keeps the source's world rotation relative to its bind pose.
   - The pelvis translation becomes the target's rest position plus the
     source's offset.
   - Constant translation and scale channels are dropped.
4. **Textures:** base colour only (the runtime samples nothing else),
   box-downscaled to 1024 (LOD0), 512 (LOD1) and 256 (LOD2) px and deduped.
5. **LODs:** LOD1 and LOD2 keep 12% and 3% of each part's triangles,
   simplified by meshoptimizer with UV/normal error weights. The body
   simplifies whole and then splits, so its regions share every seam.

Output is byte-identical across runs on the same Node version (22.20 built the pinned hashes; PNG deflate comes from Node's zlib).

### Body regions

A body triangle is `body_head` if any corner has ≥0.5 weight on
`Head`/`neck_01`; the cut sits under every outfit's collar. Otherwise it takes
the region with the most weight over its corners:

| Region | Joints | Hidden when worn |
| --- | --- | --- |
| `body_head` | `Head`, `neck_01` | never (hoods sit over it) |
| `body_torso` | `spine_*`, `clavicle_*` | `*_body` |
| `body_arms` | upper/lower arm, hand, fingers | `*_arms` (they include the hands) |
| `body_legs` | `pelvis`, `thigh_*` | `*_legs` |
| `body_shins` | `calf_*` | `*_legs` **and** `*_feet` |
| `body_feet` | `foot_*`, `ball_*` | `*_feet` |

- **Shins** need both: trousers stop mid-calf and boots start below the knee.
  Trousers alone show the shin below the hem; boots alone show it above.
- **Hair** sits under the hood: hide it while `ranger_hood` is worn.
- **Waist band:** trousers start below the pelvis cut, so `body_torso` also
  holds the legs triangles with ≥0.3 spine weight (summed over corners). A
  bare torso over trousers then shows the underwear's waistband instead of a
  gap. The reverse isn't done: tops reach below the cut, and the band would
  poke through them.
- **Known limits:**
  - The female Peasant's low-rise trousers interpenetrate a bare torso at the
    waist.
  - With a top and bare legs, the underwear can poke through a top's hem.
  - The garments are fitted to a hidden body, and no weight cut matches every
    garment edge. Full outfits are clean; so are the other single-slot swaps
    tried.

### Parts

Triangles at LOD 0 / 1 / 2. A part with two materials is two draws. `body_torso`
repeats the waist band from `body_legs`.

| Part | Materials | Male | Female |
| --- | --- | ---: | ---: |
| `body_head` | MI_Superhero_* | 2912 / 264 / 71 | 3088 / 280 / 66 |
| `body_torso` | MI_Superhero_* | 1652 / 286 / 84 | 1936 / 270 / 84 |
| `body_arms` | MI_Superhero_* | 4592 / 466 / 112 | 4408 / 432 / 111 |
| `body_legs` | MI_Superhero_* | 1218 / 214 / 46 | 1286 / 242 / 61 |
| `body_shins` | MI_Superhero_* | 1110 / 134 / 36 | 1020 / 152 / 50 |
| `body_feet` | MI_Superhero_* | 1328 / 185 / 46 | 1312 / 204 / 34 |
| `eyes` | MI_Eyes | 768 / 92 / 22 | 768 / 92 / 22 |
| `eyebrows` | MI_Hair_1 (m), MI_Hair_2 (f) | 984 / 118 / 28 | 1480 / 176 / 42 |
| `hair_buzzed` | MI_Hair_1 | 830 / 98 / 24 | 830 / 99 / 24 |
| `hair_simple_parted` | MI_Hair_1 | 1301 / 155 / 39 | – |
| `hair_beard` | MI_Hair_1 | 1034 / 124 / 31 | – |
| `hair_buns` | MI_Hair_2 | – | 3284 / 393 / 97 |
| `hair_long` | MI_Hair_2 | – | 2906 / 347 / 86 |
| `ranger_hood` | MI_Ranger | 2136 / 256 / 64 | 2136 / 256 / 64 |
| `ranger_pauldrons` | MI_Ranger | 1376 / 164 / 40 | 1376 / 164 / 40 |
| `ranger_body` | MI_Ranger | 4606 / 551 / 132 | 4570 / 545 / 137 |
| `ranger_arms` | MI_Ranger + MI_Regular_* (hands) | 8564 / 1026 / 254 | 8508 / 1015 / 250 |
| `ranger_legs` | MI_Ranger | 1128 / 135 / 32 | 1204 / 143 / 35 |
| `ranger_feet` | MI_Ranger | 9172 / 1098 / 274 | 9172 / 1100 / 274 |
| `peasant_body` | MI_Peasant | 3856 / 462 / 113 | 2188 / 262 / 65 |
| `peasant_arms` | MI_Peasant (+ MI_Regular_Male hands) | 5338 / 639 / 158 | 6848 / 820 / 204 |
| `peasant_legs` | MI_Peasant | 1112 / 133 / 32 | 1944 / 232 / 58 |
| `peasant_feet` | MI_Peasant | 2588 / 310 / 77 | 2588 / 310 / 77 |

### Files

| File | Parts | Triangles | Vertices | Primitives | File MB |
| --- | ---: | ---: | ---: | ---: | ---: |
| `male_lod0.glb` | 21 | 57605 | 44432 | 23 | 5.50 |
| `male_lod1.glb` | 21 | 6910 | 6665 | 23 | 1.59 |
| `male_lod2.glb` | 21 | 1715 | 1974 | 23 | 0.69 |
| `female_lod0.glb` | 21 | 62852 | 51242 | 22 | 6.48 |
| `female_lod1.glb` | 21 | 7534 | 7806 | 22 | 1.87 |
| `female_lod2.glb` | 21 | 1881 | 2265 | 22 | 0.78 |

### Draw cost

Only selected parts draw (in every pass), so a loadout costs one draw per
selected primitive, whichever glb it comes from:

| Loadout | Draws | Before (merged presets) |
| --- | ---: | ---: |
| Full Ranger (head, eyes, brows, 6 slots) | 10 | 5 |
| Full Peasant + hair | 9 (male) / 8 (female) | 5 / 4 |
| Bare body + hair | 9 | 3 |

Toggles cost draws: a slot can only hide if it is its own primitive. One glb
per body, rather than per body × outfit, adds no draws. It stores the body
and hair once instead of once per outfit, and lets loadouts mix outfits
without switching assets.

GPU textures upload lazily, once per drawn primitive (the loader keeps a
texture per primitive, not per image): about 5.3 MB per textured draw at
LOD0, 1.3 MB at LOD1, 0.3 MB at LOD2 (eyes: 0.3 MB at every LOD).

## Rebuilding and publishing

To rebuild, download the three used archives (UAL2 isn't needed) and unzip
them side by side into one directory, then run:

```sh
npm run build:characters -- --src <dir>
```

To publish:

```sh
gh release create characters-v1 examples/outfits/*.glb --repo tommy-xr/functor \
  --title "Character presets v1" --notes "Quaternius CC0; see examples/outfits/ASSETS.md"
```
