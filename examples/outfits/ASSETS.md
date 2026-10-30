# Outfits sample assets

The `.glb` presets are built from Quaternius's free (Standard) packs by
`npm run build:characters` (`tools/character-presets/build.mjs`) and fetched by
`npm run fetch:assets` from the `characters-v1` GitHub release. They are
gitignored; `presets.lock.json` pins each file's SHA-256.

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
- **Hair:** six hairstyles and two eyebrow meshes, shipped separately (not used yet).
- **Outfits:** Ranger and Peasant, each male and female, also split into
  parts:
  - Peasant: arms, body, feet, legs.
  - Ranger: arms, body, feet, legs, hood, pauldron(s).
  - One extra base-colour variant each (`T_Ranger_3`, `T_Peasant_2`).
- **Clips:** UAL1 and UAL2 have 43 clips each, plus a root-motion copy of
  each. UAL2 also ships a female mannequin.
- **No tactical gear:** a vest, helmet or backpack must be authored.

## Joint contract

All bodies, outfits, parts, and both UALs use one skin. It has the same 65
joints (`root`…`ball_leaf_r`) in the same order and hierarchy, so joint
indices need no remap. The build still remaps by name and fails on a missing
joint.

The bind poses are **not** identical:

| Pair | Difference |
| --- | --- |
| Female body vs female outfits | Identical (≤0.04°, 0 mm). |
| Male body vs male outfits | The outfits fit a narrower "Regular" male body, which isn't in the free tier. Shoulders and hips differ by 17–24 mm. Rest rotations differ by up to 2.5° (clavicles) and 13° (thumbs). Spine, neck and head match exactly. |
| UAL mannequin vs every body | Pelvis 32 mm lower. Rest rotations differ by up to 11° (`spine_03`). |

Every UAL clip keys translation on all 65 joints using the mannequin's bone
lengths. Copied verbatim, a clip would force the mannequin's proportions onto
the character.

To check any set of files against the first one's skin, run
`node tools/character-presets/inspect.mjs <files…>`. It prints skins, joint
order, parents, inverse-bind differences, geometry, textures and clips.

## Pipeline

1. **Canonical skin:** the outfit's skin (or the body's skin for `base`).
   - From the body, only triangles with ≥0.5 corner weight on `Head`/`neck_01`
     are kept. The outfit pack's readme says only the head is needed; the cut
     sits under the collar.
   - The head's joints bind identically on both male rigs, so this is exact.
   - The pack has no per-region masking data; `base` keeps the whole body.
2. **Clips:** `Idle_Loop`, `Walk_Loop` and `Jog_Fwd_Loop` (no root motion) are
   retargeted to the canonical rig and named `Idle`, `Walk` and `Run`.
   - Each joint keeps the source's world rotation relative to its bind pose.
   - The pelvis translation becomes the target's rest position plus the
     source's offset.
   - Constant translation and scale channels are dropped.
3. **Materials:** primitives that share a texture are joined, so there is one
   draw per material.
   - Only base colour is kept; the runtime samples nothing else.
   - Textures are box-downscaled to 1024 (LOD0), 512 (LOD1) and 256 (LOD2) px.
4. **LODs:** LOD1 and LOD2 keep 12% and 3% of the triangles, simplified by
   meshoptimizer with UV/normal error weights.

Output is byte-identical across runs.

| File | Triangles | Vertices | Draws | Texture MB (GPU) | File MB |
| --- | ---: | ---: | ---: | ---: | ---: |
| `male_base_lod0.glb` | 14318 | 8483 | 3 | 11.0 | 1.71 |
| `male_base_lod1.glb` | 1716 | 1148 | 3 | 3.0 | 0.66 |
| `male_base_lod2.glb` | 426 | 366 | 3 | 1.0 | 0.38 |
| `male_ranger_lod0.glb` | 31646 | 27675 | 5 | 21.7 | 3.91 |
| `male_ranger_lod1.glb` | 3794 | 4453 | 5 | 5.7 | 1.23 |
| `male_ranger_lod2.glb` | 943 | 1220 | 5 | 1.7 | 0.56 |
| `male_peasant_lod0.glb` | 17558 | 11657 | 5 | 21.7 | 2.89 |
| `male_peasant_lod1.glb` | 2105 | 1576 | 5 | 5.7 | 1.04 |
| `male_peasant_lod2.glb` | 520 | 507 | 5 | 1.7 | 0.51 |
| `female_base_lod0.glb` | 15060 | 8844 | 3 | 11.0 | 1.79 |
| `female_base_lod1.glb` | 1804 | 1216 | 3 | 3.0 | 0.68 |
| `female_base_lod2.glb` | 448 | 389 | 3 | 1.0 | 0.39 |
| `female_ranger_lod0.glb` | 32302 | 27955 | 5 | 21.7 | 4.00 |
| `female_ranger_lod1.glb` | 3867 | 4462 | 5 | 5.7 | 1.26 |
| `female_ranger_lod2.glb` | 954 | 1213 | 5 | 1.7 | 0.57 |
| `female_peasant_lod0.glb` | 18904 | 13331 | 4 | 16.3 | 2.63 |
| `female_peasant_lod1.glb` | 2266 | 1971 | 4 | 4.3 | 0.93 |
| `female_peasant_lod2.glb` | 561 | 618 | 4 | 1.3 | 0.47 |

"Texture MB (GPU)" is RGBA8 plus mips, counted per draw: the loader uploads
one texture per mesh.

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
