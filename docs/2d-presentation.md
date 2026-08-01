# The 2D presentation layer

Functor's 2D pass — `Camera2D`, `Sprite`, `Frame.create2D` / `Frame.with2D` —
is a real subsystem with a good core: a declarative view extent that
letterboxes instead of stretching, painter-order composition, and `Sprite.t` as
plain inspectable data. It was also, until this document's first slice, missing
the two primitives every 2D game needs.

This note records the design of the 2D content surface and the order the
remaining pieces should land in. It exists because a multi-entry game jam
produced four independent 2D games and all four hit the same two walls.

## The evidence

Four jam entries built 2D games docs-first, as external users. Ranked findings,
condensed:

- **No text, at all.** `Sprite` had 17 functions and zero glyphs. `asteroids2d`
  hand-typed a 5x3 bitmap font as a 130-line `font.fun` module in order to print
  the word `SCORE`; `rpg`'s dialog was literal glyph lists, and its NPC's `!`
  speech marker was assembled from three rectangles. Every 2D game needs a
  score, so every 2D game had to rasterize its own glyphs. The existing
  `Ui.text` does not help: it is a separate screen-space overlay with corner
  anchors, so it cannot be positioned relative to the playfield, and diegetic
  text (damage numbers, floating pickups, signs, name labels) is impossible.
- **No filled non-rectangular shape.** `rectangle` and `square` were the only
  fills. `asteroids2d` faked vector outlines with a 64-line `shape.fun` that
  assembles rotated thin rectangles, and hit two non-obvious artifacts on the
  way: segments need to be `thickness` longer than the span or every corner
  notches (there are no line joins), and rotating the assembled group instead of
  the points makes stroke width vary with angle. A game wanting a solid
  triangle, a filled planet, or a health pie had no path short of authoring a
  PNG.
- **No text measurement**, so column layout was abandoned outright: a
  `label | value` grid needs the label column's width, which became a magic
  `7.0 * 4.0 * hudCell` constant hand-fitted to the widest label.
- **No screen-space anchoring**, so a HUD is arithmetic over the view extent —
  and would scale *with* the field under `Camera2D.zoom`, which is exactly wrong
  for a HUD. One entry deleted its `ui` hook (and its only mouse affordance)
  rather than float a DOM-ish corner panel over a letterboxed vector field.

One bug from that jam shaped this design more than any feature request. A menu's
three lines were spaced 1.0 world units apart while a glyph was 1.1 tall, so
they overlapped illegibly. There was no line-height concept to lean on, so line
spacing was the game's problem — **and it was invisible in code review**. Both
reviewing engines missed it; only a capture caught it. Any text API that leaves
line height implicit reproduces that bug in every game that uses it.

## Slice 1 (landed): text and measurement

```
type metrics = { width: float, height: float }

let text    : (Color.t, float, string) => t
let measure : (float, string) => metrics
```

### Decisions, and why

**`size` is the line height, in world units.** Not cap height, not an em. The
built-in font's cell is square and self-spacing, so one number is simultaneously
the line height, the cell size, and the per-character advance. That collapses
the whole metric story into something an author cannot get wrong, and it is what
kills the overlap bug: `measure(size, s).height` is `size` per line, always, so
stacking blocks by their measured height is overlap-free *by construction*
rather than by the author remembering a fudge factor.

Text lives in world space like every other sprite, so it scrolls, zooms, and
letterboxes with the field. That is correct for diegetic text and wrong for a
HUD — the HUD case needs the anchoring layer below, and no amount of camera
features substitutes for it.

**Centered origin**, both axes, like `rectangle` / `square` / `image`.
Consistency beats the baseline-left convention here: `Sprite.move` then places
text exactly the way it places every other primitive, and centered text was the
jam's actual hard case. Alignment is a half-width shift, which is one line and
documented in the signature docs so nobody rederives the sign:

```functor
// left edge at x
Sprite.text(color, size, s) |> Sprite.move(x + Sprite.measure(size, s).width * 0.5, y)
```

**`\n` breaks lines** — each line stacked at exactly one `size` and centered
within the block; a trailing newline is a real, empty final line. This was
deliberately *not* deferred. A silent hole in the middle of a line is the same
invisible-in-review class of bug as the overlap above, and deferring it would
be a compatibility ratchet: making `\n` break later would change every capture
that had shipped with a hole. `measure` and `text` route through one layout
function, so they cannot disagree — width is the widest line's, height is
`size` per line.

The stride being exactly the glyph cell means lines cannot overlap but do sit
tight: a descender nearly meets the next line's capitals, as in a terminal. That
is the deliberate trade for making `size` a single self-consistent metric —
looser leading is available by drawing lines individually and spacing them more
than `size` apart, and an explicit line-height parameter belongs to the
follow-up `textBlock`.

**Unsupported characters occupy their cell and draw nothing.** Skip, not a tofu
box: there is no atlas cell to spare for tofu, and a monospace gap already
localizes the problem legibly. Because the decision is made in shared lowering
code, no renderer can disagree, and `measure` counts the gap so measurement
still matches what is drawn.

**Unicode, honestly.** Layout iterates Unicode scalar values, matching
`Text.chars`. Non-ASCII scalars each consume a blank cell. Combining marks are
*wrong*, not merely absent: `"e\u{301}"` is two scalars, so it takes two cells
and the mark does not compose onto its base. There is no shaping, bidi, kerning,
or ligatures. This is an ASCII bitmap font, and the docs say so rather than
implying coverage it does not have. Full text is a font-loading feature.

**Color multiplies from white.** The atlas is white glyphs on transparent black,
so the author's color is reached exactly, and the existing `Sprite.tint` /
`Sprite.fade` multiplies compose on top with the semantics they already
document. This deliberately avoids adding a second coloring mechanism, and it
avoids the "tint cannot lighten" trap — white is the brightest input a multiply
can have.

### The font

The printable-ASCII subset (U+0020..U+007E) of **unscii-8** by Viznut
(<http://viznut.fi/unscii/>), which is in the **public domain**. Only the
separate `unscii-16-full` variant carries a GPL obligation, because it embeds
GNU Unifont; it is not used here.

It ships as a **table of glyph bitmaps in Rust source** — one byte per pixel row
in `runtime/functor-runtime-common/src/sprite_font.rs` — not as a PNG and not via
`include_bytes!`. So the font diffs, reviews, and greps like code, and the atlas
is expanded procedurally on first bind. The font is part of rendered output, so
changing it changes captures and goldens; a test pins the exact set of
full-width glyphs so a font swap has to consciously restate its metrics instead
of silently shifting every capture.

Six glyphs (`*`, `/`, `X`, `Y`, `\`, `_`) do paint the rightmost column, so
`XX` and `//` touch, as they do in any 8px-cell terminal font. `_` reaching the
cell edge is *required* — otherwise underlines break into dashes. The
alternative, a 9/8 advance, would guarantee gaps at the cost of breaking
underlines and making `measure` stop being `n * size`. Cell advance won.

### Plain data is preserved

The sprite tree stores the string:

```
Sprite.text(Color.rgb(1, 0.5, 0.25), 2, "HI")   ==>   Sprite.Text(2, 1, 0.5, 0.25, "HI")
```

That is a `Value::Variant`, not host data, so structural equality, `serde`
serialization, model storage, hot reload, and time travel all keep working —
`Sprite.t`'s differentiating property survives the addition of text. Glyph
expansion happens strictly later, during lowering at `Frame.create2D` /
`Frame.with2D` time, in exactly the place `Sprite.image` already lowers. (Note
for anyone reading the architecture: sprite lowering is *eager at frame
construction*, not lazy at draw, so "expansion at render" means "at lowering,
never in the value".)

### One code path for every target

The entire sprite pass is shared Rust in `functor-runtime-common`; the desktop
and web crates contain no sprite code whatsoever. Lowering, layout, atlas
generation, cell selection, and blend state are therefore literally one
implementation, and native GL and WebGL2 cannot drift.

The one thing that genuinely differs per target is asset byte loading
(`fs::read` vs `fetch`), which is why the atlas is a compiled-in
`TextureDescription::Builtin` (protocol v7) rather than a texture with a
locator: no asset cache, no IO, no fetch, ready on frame one, and no magic path
string a game could collide with. Bleed between neighbouring cells is prevented
by the half-texel inset clamp already shipping in the shared sprite shader — the
same mechanism `Sprite.region` atlases rely on — so no padding gutter was added
on top of it. There is deliberately no pixel snapping: text is world-space, and
snapping to device pixels would make it jitter as the camera scrolls.

## Slice 2 (landed): filled shapes

```
let circle  : (Color.t, float) => t                                  // radius
let polygon : (Color.t, List<Input.point2>) => t                      // filled, CONVEX
let line    : (Color.t, float, Input.point2, Input.point2) => t       // thickness, from, to
```

Same plain-data discipline as text: the sprite tree keeps the author's
parameters (`Sprite.Circle(3, …)`, not 32 expanded vertices), and triangulation
happens at lowering.

### The point type is `Input.point2`, and that is not a stylistic choice

Record literals resolve **nominally** by field set, and two same-shaped
declarations make a bare literal an *ambiguous record literal* — a hard check
error. Verified directly:

```
error: ambiguous record literal: fields match PointA and PointB — annotate which one is meant
```

So declaring a second `{ x, y }` type for geometry would have broken every game
with a bare point literal anywhere. There can be exactly one `{ x, y }` record in
the prelude, and `Input.point2` already existed. The cross-module reference reads
slightly oddly from `Sprite`, and it is still the only correct answer.

### Convexity: an error, never a wrong fill

The fill is a triangle fan from the first vertex, which is correct only for a
convex outline; on a concave one it paints outside the shape. Rather than
document that as undefined behavior, `Sprite.polygon` **validates convexity at
construction** — an O(n) cross-product sign scan, pure and unit-tested — and
rejects a concave outline with a teaching error that names the fix (split it into
convex pieces and group them). `examples/shapes2d` does exactly that with a
notched asteroids hull, which is the shape a real game reaches for first.

**Either winding is accepted.** A game computing points from angles can
legitimately produce clockwise or counter-clockwise, nothing culls back faces, and
demanding one winding would be an invisible trap (a silently empty screen).
Collinear vertices are fine — they are convex — but a zero-area outline is
rejected, since it cannot be filled and would give the mesh a degenerate
bounding box.

**Sign-consistency alone is not enough, which cost a review round.** The first
implementation only checked that consecutive cross products agreed in sign. That
is necessary but *not sufficient*: a **star** turns the same way at every vertex
yet self-intersects, so a pentagram passed and filled as overlapping fan
triangles — the exact failure this validation exists to prevent, in a shape a 2D
game reaches for as readily as the notched hull. Convexity now also requires the
outline to wind around exactly once (a pentagram winds twice, a 7/3 star three
times).

That second test is needed only above 4 points, and the bound is exact rather
than cautious: if every turn has the same sign then each `|turn| < pi`, so the
total turning is under `count * pi`; the total is also a whole number of
revolutions, so `revolutions < count / 2`. Two or more revolutions therefore
requires at least 5 points, and a consistently-turning triangle or quadrilateral
is always simple. So the common cases skip the transcendentals and the check is
free — `frame_bench shapes` measured 1.15 us/shape after the fix against 1.16
before it.

### Lines have no caps and no joins, on purpose

`Sprite.line` is one segment: it stops flat (butt caps) at each endpoint. Two
lines meeting at an angle therefore leave a notch — the jam's P1-1 complaint is
only *partly* answered by this slice, and pretending otherwise would be worse
than saying so. What IS fixed is the other half of that finding: thickness is
applied in the segment's own frame (scale, then rotate), so it is exact at every
angle, where a game rotating an assembled group gets thickness that varies with
direction. `examples/shapes2d` renders a 14-spoke fan precisely so that invariant
is visible rather than asserted.

Thickness is **geometry, not a screen-space stroke**: `Sprite.scale(k)`
multiplies it along with the length, and `scaleXY` with unequal factors distorts
it for any line that is not axis-aligned. That is the honest behavior of an
affine transform on geometry, and it is why a `Stroke` record with a
screen-space width is not quietly implied by this surface.

### Rendering

One new protocol variant, `Shape::ConvexPolygon { points }` (v8), carrying its XY
points inline exactly as `Shape::Heightmap` carries heights. The renderer keeps
one persistent mesh **per point count** and re-uploads vertices in place, so
there is no per-frame VAO/VBO churn.

`Sprite.circle` exploits that deliberately: every circle lowers to the *same*
unit-radius 32-gon plus a scale transform, so all circles in a process share one
mesh and upload nothing at all. Author-supplied polygons of equal vertex count
share a mesh and each re-uploads before its own draw — measured at 432 B/frame
for three distinct triangles, which is the honest cost of the sharing.

Still deferred: `polyline` / `outline` with real joins and caps (a `Stroke`
record: width, cap, join), rounded rectangles, dashes, and concave fills via ear
clipping.

### What this slice does and does not close

Worth stating precisely, because it is easy to read "filled shapes landed" as
more than it is. The jam's *fill* findings are closed: a solid triangle, a filled
planet, a health-pie wedge, and a seeded rock n-gon are all expressible now. The
jam's *stroke* findings are only **half** closed. `Sprite.line` fixes the
artifact where thickness varied with the segment's angle, but it has no joins, so
a closed outline assembled from segments still notches at every corner — which
was the other half of the same finding.

Concretely: `asteroids2d`'s `shape.fun` is **reduced, not deleted**. Its
`segment` becomes one `Sprite.line`, and its rock silhouettes become
`Sprite.polygon`, but its `outline` still belongs to game code until `polyline`
exists. That is why jointed strokes head the follow-up queue rather than sitting
further down it.

## Later slices, in priority order

0. **Jointed strokes** — `polyline` / `outline` over a point list, with a
   `Stroke` record (width, cap, join). This is the remaining half of the jam's
   stroked-geometry finding: `Sprite.line` fixes angle-dependent thickness but
   still notches at corners, because a single segment has no joins.
1. **Screen-space anchoring** — `Sprite.anchored(Anchor.t, insetX, insetY, t)`
   plus `Frame.with2DOverlay`, resolving against the letterboxed viewport and
   ignoring `Camera2D.at` / `zoom`. This is the highest-value remaining item: it
   is what makes a scrolling 2D game with a fixed HUD expressible at all, and
   its absence is what made a jam entry delete its `ui` hook. The insight is
   that a 2D frame has *two* coordinate systems, and no camera feature
   substitutes for admitting it.
2. **`Camera2D.viewport`** — `Camera2D.toWorld` now maps pointer input through
   letterbox bars, pan, and zoom; exposing the fitted viewport remains useful
   for game-authored layout and diagnostics.
3. **Atlas regions in the typed asset manifest** — `functor import` should emit
   named regions and animation clips for a sprite sheet the way it emits glTF
   clips, so games write `Assets.heroAtlas.walk1` instead of
   `Sprite.region(96, 0, 96, 96)`. Atlas geometry is the one asset-shaped thing
   the branded manifest does not cover. A `Sprite.animate(frames, fps, tts)`
   convenience belongs with it.
4. **Blend modes** — `Sprite.blend(Blend.additive, …)`, so vector and neon 2D
   can glow. `Scene.emissive` already establishes the vocabulary, and this is
   the one respect in which a 3D original beat its 2D port.
5. **`Sprite.textBlock`** — multi-line with explicit line height and per-line
   alignment (left/center/right), for text blocks that want left alignment
   rather than slice 1's centered lines.
6. **Font loading** — `Sprite.textFont(color, size, Asset.Font, string)` and
   proportional metrics. `measure` exists partly as the forward-compatible seam
   for exactly this: authors ask for metrics rather than multiplying by `size`
   themselves, so a proportional font is not a breaking change.

## What should not change

- **`Camera2D.create(width, height)` is the right primitive.** Declaring the
  visible world extent and letting the renderer letterbox meant a 2D port needed
  *zero* projection math. This is better than every ortho-camera API that makes
  you specify left/right/top/bottom, or a size plus an aspect you must keep in
  sync.
- **`Sprite.t` as plain inspectable data** is a genuine differentiator —
  testable, diffable, time-travellable pictures. Every addition to this surface
  must preserve it, which is the constraint that put glyph expansion in lowering.
- **Painter-order `group`** is simple, predictable, and composes.

## Documentation debt

Worth recording because it is independent of the API, and because it was the
single highest-leverage finding in the jam: **the manual does not mention 2D at
all.** Searching the published manual for `Sprite`, `Camera2D`, `create2D`, or
even the bare string `2D` returns nothing. The manual presents cameras as
`Camera3D.lookAt` / `Camera3D.firstPerson`, both perspective, and reads
unambiguously as a 3D-only engine — so a user following the docs concludes
Functor cannot do 2D. Multiple jam entries only learned `Frame.create2D` exists
by reading `examples/platformer` and the `.funi` sources. A complete, well-designed
2D subsystem is invisible to its audience, and a "2D games" manual section
belongs alongside "A complete game".
