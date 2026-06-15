# Render targets

A model for render-to-texture (RTT) in Functor: render a sub-scene to an
offscreen texture, then sample that texture as a material elsewhere — in-world
monitors, mirrors, portals, minimaps, 3D thumbnails. Closely linked to
[lighting](lighting.md): shadow maps and environment cubemaps are *special-case*
render targets, so this is the shared foundation under all three.

## Core model: a render target is a `Frame` rendered into a named texture

The root render is already "render a `Frame` (camera + scene + lights) to the
screen." A render target is the same operation aimed at a texture, so the model
is **recursive**:

```
RenderTarget = { name; width; height; frame: Frame }
```

This gives the sub-scene its **own camera** (essential — a monitor/mirror sees
from a different viewpoint), its **own lights**, and natural **nesting** (a
target whose scene samples another target). It also makes shadow maps and
cubemaps fall out as internal render targets rather than bespoke code.

## Frame-level, not a visible scene node

Render targets live on the `Frame` (alongside lights), **not** in the visible
`Scene3D` tree:

- **Ordering / no 1-frame lag.** The texture must be produced *before* the
  geometry that samples it. Frame-level targets render in a pre-pass, always
  before the main scene; a tree node's render order would depend on position and
  a back-reference would lag a frame.
- **It isn't located anywhere.** A tree node implies a transform/position; an
  offscreen render has none — its geometry lives in its own scene.

## API sketch

```fsharp
// Define an offscreen view (own camera + scene, rendered to a 512² texture).
let monitor = RenderTarget.create "monitor" 512 512 (Frame.create monitorCam monitorScene)

// Use its output as a texture/material anywhere in the main scene.
let screenMat = Material.texture (Texture.renderTarget "monitor")
quad() |> material screenMat |> Transform.translateZ 4.0f      // an in-world screen

// The frame carries its targets; they render first, in dependency order.
Frame.create camera scene |> Frame.withRenderTargets [ monitor ]
```

The only new piece in existing types is a `TextureDescription::RenderTarget(name)`
variant next to today's `File(path)` — so anything that samples a texture
(materials now, cubemaps later) accepts a render target's output for free.
Targets are identified by **string name** (like `Texture.file "path"`); name
collisions are the author's responsibility.

## Connection to lighting (shared foundation)

All three need the **same `RenderTexture` / FBO abstraction** — render a `Frame`
into an attachment (color and/or depth):

- **Shadow maps** — depth attachment, light's camera, comparison sampling.
- **Environment cubemaps** — color, ×6 faces from a point (IBL + reflections).
- **User render targets** — color, an arbitrary camera/scene.

Build that abstraction once. In practice the shadow-map phase of the lighting
project introduces it; user render targets generalize it into a feature. Because
a target *is* a `Frame`, its sub-scene is lit (and casts/receives shadows)
independently, with no special-casing.

## Lifecycle, ordering, cost

- **Caching:** the texture + FBO are cached by `(name, size)` and recreated on
  resize — same pattern as the heightmap mesh cache and planned shadow maps.
  Eviction deferred.
- **Render order:** topological over target → texture references. Cycles
  (mirror-in-mirror, recursive monitors) get a **recursion-depth cap**; true
  cycle support is deferred.
- **Quest cost:** each target is a separate render pass = a tile load/store the
  tiler can't hide. Keep count and resolution modest. Most targets re-render
  every frame (unlike a baked cubemap), so a later "refresh every N frames" knob
  is worthwhile.

## Open questions

- `Frame.withRenderTargets` (builder) vs. an extra `Frame.create` argument.
- Whether shadow maps / cubemaps are *literally* expressed as `RenderTarget`s or
  just share the FBO abstraction (leaning: share the abstraction, keep their
  specialized passes — depth/comparison and 6-face are different enough).
- Depth/stencil attachment options for color targets (e.g. a monitor that needs
  its own depth buffer — almost always yes).
