# Lighting

A project plan for real-time lighting in Functor. Target aesthetic: **Doom 3**
— pitch-black-until-lit, dynamic per-light contribution, shadow casters &
receivers, and bright **emissive** surfaces (neon / sci-fi signage). Target
hardware: desktop **and** mobile VR (**Quest 2/3**), so every decision is made
tiled-GPU-first.

## Approach & key decisions

We adopt the Doom 3 *model* but not its *mechanisms* — its defining tech
(stencil shadow volumes, literal re-render-per-light multi-pass) is exactly what
tiled mobile GPUs are worst at (overdraw, stencil traffic, closed-mesh
silhouettes). The model ports; the mechanisms get swapped:

| Doom 3 did | We do (Quest-friendly) |
| --- | --- |
| Stencil shadow volumes | **Shadow maps** (depth-from-light + PCF) |
| Re-render geometry per light (multi-pass) | **Single-pass forward** with a bounded light array (cf. the skinned-joint uniform array) by default; **multi-pass additive** kept as the fallback for many-light scenes |
| Per-light bump/specular passes | Folded into the single forward pass |

This preserves the feel — dynamic lights, per-light shadows, neon emissive,
caster/receiver control — while staying tiler-friendly.

**Lighting strategy (two modes, same shading math):**
- **Single-pass, up to N = 8 lights per shader** is the default — all lights
  affecting a surface evaluated in one forward pass via a bounded uniform array.
  Cheapest on Quest; covers the common case.
- **Multi-pass additive** is the fallback for scenes with **more than 8** lights
  on a surface: render the base/emissive pass, then accumulate the extra lights
  in additive passes. Same per-light shading as the single-pass shader, just
  split across draws. Kept as a first-class path, not dropped.

**Quest-first constraints (baked in from the start):**
- Bounded light count (uniform array), single forward pass for the common case.
- Modest-resolution shadow maps with PCF; no stencil volumes.
- Keep overdraw low; skip a depth pre-pass unless profiling demands it (tilers
  usually don't need it).
- No full-screen post initially (bloom for neon is tempting but it's a
  full-screen pass — defer).
- Leave room for multiview stereo and, eventually, baked lightmaps for static
  scenes (the most Quest-friendly option of all).

## Architecture changes

- **Normals (prerequisite).** Today's vertex format is position+uv only, and the
  glTF importer drops `NORMAL`. Add a normal attribute to the vertex format(s),
  generate it for primitives (analytic), the heightmap (finite differences on
  the height field), and import glTF `NORMAL`. Skinned normals are transformed by
  the joint matrices alongside positions.
- **Lights in the `Frame`.** A serializable `Light` (type: directional / point /
  spot; color; intensity; range/attenuation; `castsShadow`), with
  `Frame { camera, scene, lights }`. Lights are pure data in the functional core
  — and show up in `/scene` introspection for free.
- **Material model.** Extend `Material::draw_opaque` to receive the light list
  (and shadow-map handle). Add a `LitMaterial` (albedo + ambient + N lights) and
  an **emissive** channel (texture/color added unconditionally). Keep today's
  `BasicMaterial` as the explicit **fullbright / unlit** path (neon, UI).
- **Passes.** Each shadow-casting light renders a depth-only pass into a shadow
  map; one forward lit pass then samples them and adds emissive. Caster/receiver
  flags select what's drawn into vs. what samples the shadow map.

## Frame & shadow-participation API

Lights are **frame-level** data (lighting is a per-frame concern, and it keeps the
scene tree about geometry). `Frame` grows a lights list; shadow *participation*
is a per-node property of the scene, set with modifiers like transforms.

```fsharp
// Frame = camera + scene + lights  (Frame.create camera scene still works; lights default to [])
Frame.createLit camera scene lights

// Lights (frame-level). `castShadows` opts a light into rendering a shadow map.
Light.directional dir color intensity
Light.point pos color intensity range
Light.spot pos dir color intensity range coneAngle
Light.directional dir color intensity |> Light.castShadows   // this light casts

// Shadow participation is per scene node, defaults to **casts + receives**, and
// propagates to the subtree (threaded through render like the current material).
neonSign  |> Scene3D.fullbright |> Scene3D.receivesShadows false   // self-lit, ignores shadow
skybox    |> Scene3D.castsShadows false |> Scene3D.receivesShadows false
floor     // unannotated → casts + receives, the common case
```

Two independent controls, so the simple case needs zero annotation:
- **Per light** — `Light.castShadows`: does this light render a shadow map at all?
  (Many lights, only a budgeted few casting — see the shadow budget below.)
- **Per node** — `castsShadows` / `receivesShadows` (default true): the shadow
  depth pass draws caster nodes; the lit pass lets receiver nodes sample the maps.

So: add one `castShadows` light and everything casts+receives automatically —
shadows appear with no per-node work. You only annotate the exceptions (emissive
signage, skybox, first-person weapon, etc.). Implementation-wise the two flags
ride on `Scene3D` (like `xform`) and thread down the render walk.

**Forward-compatible: booleans are a 1-bit shadow mask.** The boolean model is
deliberately the degenerate case of *shadow channels* (cf. Unreal "Lighting
Channels" / Unity "Rendering Layer Mask") — a small bitmask per light, caster,
and receiver, where a caster enters a light's map (and a receiver is darkened by
it) only when their masks overlap:

```
caster C in light L's map   iff  L.shadowMask & C.casterMask   != 0
receiver R darkened by L     iff  L.shadowMask & R.receiverMask != 0
```

The booleans map exactly onto this — `castShadows` ⟺ `shadowMask != 0`,
`castsShadows false` ⟺ `casterMask = 0`, `receivesShadows false` ⟺
`receiverMask = 0`, default all-ones ⟺ everything interacts. So we **ship the
booleans now** and can add a `shadowLayers`/mask modifier later with the
booleans as presets — no breakage.

Grow into the full mask when there's a concrete multi-group need (a first-person
weapon or "hero" object shadowed by its own light only; separately-lit indoor /
outdoor zones). It only becomes *usable* once there's more than one
shadow-casting light (a follow-up), and it's an **authoring** feature, not a
perf one — masks add selectivity, not cheaper passes (still one map per casting
light). Likely candidate when the VR target leans on per-light/per-zone shadow
scoping; a fixed 8-channel mask (Unreal-style) is the probable shape.

## Roadmap (small, ordered PRs)

1. **Normals** — vertex-format attribute + generation/import + a debug
   "normals as color" view to verify. No other visible change. Gates everything.
2. **Emissive / fullbright** — cheapest visible win; self-lit surfaces & neon.
3. **Lights in `Frame` + `LitMaterial`** with one **directional** light
   (Lambert + ambient). Verify: the synthwave terrain catching a "sun."
4. **Point + spot lights** with attenuation, in the bounded-light forward shader.
5. **Shadow maps** — directional first (ortho depth pass + PCF) with
   `castsShadow` / `receivesShadow`; then point/spot shadows. Introduces the
   shared **render-to-texture / FBO** foundation that cubemaps and user
   [render targets](render-targets.md) also build on (shadow maps and cubemaps
   are effectively special-case render targets). Refinements beyond the
   directional MVP, in rough order:
   - **Skinned shadow casters** — the depth pass must deform geometry by the
     joint matrices (as the lit pass does), or skinned models cast a wrong
     rest-pose shadow. Until then they're skipped (no shadow).
   - **Scene-fit ortho frustum** — fit the directional light's orthographic box
     to the visible scene (or a cascaded split) instead of a fixed box, so
     resolution isn't wasted and nothing clips out of the map.
   - **Web/wasm path** — the shadow pass is desktop-only at first; the web
     runtime renders unshadowed until the FBO pass is ported (WebGL2 supports
     it). Keep depth portable (RGBA8-packed, not a sampled depth texture).
6. **Normal mapping + specular** (the Doom 3 bump look) — needs tangents
   (glTF provides; compute otherwise).
7. **Multi-pass additive path** — the fallback when a surface exceeds N lights
   (base/emissive pass + additive per-extra-light passes, reusing the same
   shading). Lands once single-pass + shadows are solid.
8. *(Optional / deferred)* bloom post for neon glow; baked lightmaps for static
   scenes.

## Future / related: environment cubemaps (reflections + IBL)

The plan above is the **direct** lighting term; cubemaps are the **indirect**
term, and pair cleanly:

```
final = emissive + ambient/IBL(cubemap) + Σ direct lights (with shadows)
```

This is the HL2 `env_cubemap` lineage — bake a cubemap (render the scene to 6
faces from a point), then sample it for specular reflections. It's the
higher-fidelity answer to the "ambient model" question below (image-based
lighting vs. flat/hemisphere ambient), and the same cubemap doubles as the
**skybox**.

Why it fits this project:
- **Shares render-to-texture with shadow maps** — a cubemap pass is "render to a
  texture from a viewpoint" ×6; the shadow-map phase introduces that machinery.
- **Reuses normals** — reflection sampling is `reflect(viewDir, normal)`.
- **Quest-ideal when baked** — rendered once at load, then a single cheap texture
  lookup per pixel. (Real-time re-rendered cubemaps are costly; keep optional.)

Adds: a `TextureCube` type, the 6-face render-to-cubemap pass, and cubemap
sampling in the lit material. Start HL2-simple (one global env cubemap +
reflectivity factor); later, multiple placed cubemaps with nearest-selection and
prefiltered-mip + BRDF-LUT for roughness-aware PBR. **Lands after shadow maps.**

## Open questions

- ~~`Light` API shape — `Frame.lights` vs. `Scene3D` nodes~~ — **decided:
  frame-level `Frame.lights`** (see the Frame & shadow API section). Remaining:
  builder vs. extra `Frame.create` arg, and the exact `Light` constructors.
- ~~Bounded light count for the single pass~~ — **decided: N = 8** per surface;
  beyond that, fall back to the multi-pass additive path (above).
- Shadow-map resolution / count budget for Quest; how many shadow-casting lights
  to support at once (likely 1 directional + a few local).
- Ambient term: flat ambient vs. a cheap hemisphere/gradient (synthwave skies
  benefit from a gradient) vs. eventually an environment cubemap / IBL (see
  Future / related above).
