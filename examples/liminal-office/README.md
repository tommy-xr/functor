# `examples/liminal-office`

Two floors of an empty office block, by night or day: carpeted corridors under
drop ceilings and fluorescent panels, open empty offices (103 and 203 have
windows), a sodium-lit stairwell between the floors, a neon entrance on an
empty apron. `B` turns it into a block of four turned copies around a
crossroads. Desktop; Quest is configured (120 Hz, foveation off) but not yet
checked on a headset. Not the web: baked files load with `File.bytes`.

```sh
functor -d examples/liminal-office run native
functor -d examples/liminal-office run native --emulate-xr   # arrows drive the right thumbstick
functor -d examples/liminal-office run vr
functor -d examples/liminal-office test     # layout, stairs, cells, partition, provenance; no GPU
npm run test:liminal-office                 # routes, stairs, wall push, hours, cells, goldens
```

| Input | Action |
| --- | --- |
| mouse (click to capture) | look |
| `W` `A` `S` `D` / arrows | walk, strafe / turn |
| `1` `2` `3` `4` | jump to the entrance, corridor, office, window view |
| `5` `6` `7` `8` | stair foot (looking up), landing (looking down), upstairs corridor, upstairs window |
| `9` `0` | the block: up the street; office 103's window in the copy turned 180° |
| `T` | next hour |
| `N` | midnight ⇄ noon |
| `B` | the office alone ⇄ the block (views 1–8 then jump within the nearest copy) |
| VR left thumbstick | walk, strafe |
| VR right thumbstick | turn, walk |
| VR `X` / `Y` | next hour / midnight ⇄ noon |
| VR `B` | the office alone ⇄ the block |

## One layout, four consumers

`layout.fun` is plain data: the footprint, rooms (rects between wall centre
lines, on level 0 or 1), doors on shared walls, the stair flight, windows in
outside walls, ceiling panels, signs and lamps. `building.fun` derives the
interior from it, and `exterior.fun` the street side:

- **Mesh**: one `Model.staticData` of every cell's faces in cell order: a
  room's inner surfaces, panels, signs and lamps, its half of each door and
  window reveal (split at the wall's centre line; a window's outer half is the
  west yard's), and its share of the stairs, so each face lies in its cell.
- **Bake**: that whole mesh, both floors, is the `night` and `day` bake jobs,
  so light and occlusion cross between rooms and floors. The bake keeps
  triangle order and numbers vertices by first use, and no two faces share a
  vertex, so each cell's vertices are one contiguous run. At load the baked
  mesh is cut into one `Model.static` per cell: a slice of every vertex array
  (UV2 byte for byte) and the cell's triangles rebased onto it. `functor
  test` checks the cut loses and repeats nothing.
- **Cells** (`portals.fun`, not hashed into the bake): one per room, and a
  collar of four street strips around the footprint, 35 m deep (a ring isn't
  convex: the south and north strips run the full width, corners included;
  the apron and the west yard are the south and west strips). Portals on
  doors, on the windows (onto the west strip, so the street sees in), at the
  collar's corners, and at the stair opening: a horizontal portal where the
  stairwell's foot meets its head, the upper floor less the landing. Each
  cell's model is `Scene.inCells` it.
- **Collision**: `Physics.fixed` boxes along every wall (a storey high, so the
  plenum leaves no gap), a trimesh of the floors and landing. Walls are boxes
  because trimesh faces only block from the front: an inward-facing facade
  would let the capsule walk in from outside. The street (a ground box, the
  lamp posts) sits 3 cm lower. The player is a `Physics.character`.

**Culling caveat.** The cells and portals are right (`functor test`), and
`/state.cells.hidden` counts the subtrees the walk hides, but the renderer
currently draws `Model.static` data models whether or not the walk hides them,
and skips their frustum test too. Until that engine fix lands, every room is
submitted every frame.

## The stairwell

Stair B is one room on each level over the same rect. A straight flight of 18
risers (0.183 m) and 0.3 m treads, 31° and 1.35 m clear, rises east along the
south wall to a landing at the east end; its door opens onto the upstairs
turn. A stepped concrete parapet, a metre above each tread, closes the flight
off from the stair opening. Under the visible steps, the character walks a
smooth ramp through their nosings (a trimesh facing up), so it never hops
step to step. The parapets and the solid under the landing are boxes. Four
sodium lamps, bake only, light it amber.

## Lighting

The panels and lamps exist only in the bake jobs: fixture lights submitted
to the frame would take the 8 forward-light slots. The frame's lights (moon
or sun, ambient, the lobby's spill onto the apron, two street lamps) light the
street and facade; all are `Light.baked`, so the lightmapped interior ignores
them. Nothing casts shadows (a shadow pass draws every hidden room), and there
is no glass (translucent subtrees bypass culling). Panels, signs and lamp
lenses are `unlit` groups.

## The block

`block.fun` places the office as an archetype: built once in its own space,
then drawn as four copies at 0/90/180/270° about a crossroads, 24 m streets
between them, every entrance and window facing a street. Each copy is the
same scene value (the same per-cell `Model.static`s and lightmaps) under its
own translation and yaw, so the GPU holds one office. Per copy:

- **Interior**: drawn only within 30 m of the footprint, kept to 34 m so the
  edge doesn't flicker. `Scene.cells` draws everything when the eye is in no
  cell, so the collar reaches 35 m: the radius plus a metre for the eyes.
  `functor test` checks the collar holds every point of that region, corners
  included; the model's `uncovered` counts ticks where an active copy's cells
  missed the eye (the e2e checks it stays 0).
- **Colliders**: `building.fun`'s and the lamp posts' as fixed bodies under
  the copy's own tags, each copy one `Physics.memo`.
- **Scene**: one `Scene.memo` per copy, rebuilt when the hour or its
  activation changes.
- **Lights**: every copy has a lobby spill and two street lamps; the frame
  takes the six nearest the eye, which is what fits the 8 forward-light slots
  beside the sun and ambient.

Interiors pop in and out at the radius, through doors and windows still in
view: the fog only starts at 35 m (night) or 70 m (day).

The one overcast day bake holds at every yaw (it has no sun).

## Day and night

The model holds an `hour`. `hours.fun` maps it to `daylight` (0 at night, 1
by day; dawn 5–8 h, dusk 17–20 h):

- **Interiors switch** bakes at daylight 0.5 (6:30, 18:30); there is no blend.
  Both bakes unwrap the same mesh, so one set of per-cell `Model.static`s
  draws either: the day set is the night set under `Scene.withLightmap(day)`,
  built once. One vertex upload, two resident lightmaps.
- **Outdoors lerps**: sky (fog and clear colour, night → dusk mauve → haze),
  the fog's near end, ambient, moon → sun (colour, intensity, direction), neon
  and lamp glow. The frame's sun and ambient are `Light.baked` at every hour.

The day bake is the night bake plus overcast daylight through the facade's
openings (the windows of 103 and 203, and the entrance); the panels stay on.
Ambient can't model sky light: its occlusion reaches 1 m, so it would light
windowless rooms too. So each opening gets two grids of shadowed spots outside
it, bake only: a patch of sky above its head aimed at the floor inside, and a
patch of sky-lit ground (a quarter of the sky's light per m²) aimed at the ceiling.
The facade's walls shadow them, so daylight reaches only what an opening
connects to. The bake has no sun: every daylight is placed relative to its
opening, so the bake holds at any yaw of the building. The frame's sun
(`Hours.sunDirection`) lights only the outdoors.

## Rebaking

Editing `layout.fun` or `building.fun` (which holds the bake jobs and their
lights) makes the committed bakes stale:

```sh
node examples/liminal-office/bake.mjs           # bake, then write baked/provenance.txt
node examples/liminal-office/bake.mjs --check   # rebake to a temp dir; fail unless byte-identical
```

`baked/provenance.txt` records a hash of those two files, the bakes, the
baker's version and its settings; `bake.mjs` refuses to record it if they
change mid-bake. It keeps one `night.mesh.bin`, failing if the day bake's
mesh differs. `provenance.fun` hashes the files again at load: on a mismatch,
or a missing lightmap, the game draws the interior flat-lit with a "stale
bake" banner, and `functor test` fails. The per-cell split derives from the
same hashed files and the baked mesh, so provenance covers it too. The bakes
(12 texels/m, 2 bounces) are byte-identical across runs.
