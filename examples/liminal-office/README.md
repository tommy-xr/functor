# `examples/liminal-office`

Two floors of an empty office block, by night or day: carpeted corridors under
drop ceilings and fluorescent panels, open empty offices (103 and 203 have
windows), a sodium-lit stairwell between the floors, a neon entrance on an
empty apron. `B` turns it into a block of four turned copies around a
crossroads; `G` into grids of 4, 16 and 64; `H` into four towers of 8 to 16
floors, climbed by a dog-leg stair; `C` stands a multiplex cinema on its
parking lot; `P` a two-level mall round a skylit court, and its parking lot;
`I` puts the block on a 2 km island. Desktop; Quest is configured (120 Hz, foveation off) but not yet
checked on a headset. Not the web: baked files load with `File.bytes`.

```sh
functor -d examples/liminal-office run native
functor -d examples/liminal-office run native --emulate-xr   # arrows drive the right thumbstick
functor -d examples/liminal-office run vr
functor -d examples/liminal-office test     # layout, stairs, cells, partition, provenance, island; no GPU
npm run test:liminal-office                 # routes, stairs, wall push, hours, cells, island, goldens
npm run test:liminal-mall                   # the mall: lot to gallery, shop to service exit, draws, goldens
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
| `G` | the next grid: 4, 16, 64 copies (views 9 and 0 stay in it) |
| `H` | the office alone ⇄ the towers (views 1–0 become the towers', below) |
| `C` | the office alone ⇄ the cinema, from its parking lot (views 1–0 become the cinema's, below) |
| `P` | the office alone ⇄ the mall (views 1–0 become the mall's, below) |
| `K` | at most 4 → 1 → 2 interiors at once |
| `O` | overlook: the eye 20 m up, looking down over the roofs (no interiors draw; a view jump ends it) |
| `I` | the flat ground ⇄ the island (with the block; `G` and `H` leave it) |
| `V` | the island's next biome: grass, snow, desert |
| `L` | flashlight on / off |
| VR left thumbstick | walk, strafe |
| VR right thumbstick | turn, walk |
| VR `X` / `Y` | next hour / midnight ⇄ noon |
| VR `B` | the office alone ⇄ the block |
| VR `A` | the flat ground ⇄ the island |
| VR right trigger | flashlight on / off, held in the right hand |

## One layout, four consumers

The office is one `Layout` (`layout.fun`'s building kit), the plain data in
`officeLayout.fun`: the footprint, storey and level count, rooms (rects
between wall centre lines, on a level), doors on shared walls, the stair
flight, windows in outside walls, ceiling panels, signs and lamps, and the
street side's window bands and lamps. `building.fun` derives the interior
from a layout, `portals.fun` its cells and `exterior.fun` its street side;
`archetypes.fun` derives each archetype once, at load, and every copy draws
those values:

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
- **Street side** (`exterior.fun`): the facade's outer half, the roof and
  the window bands are one `Scene.instanced` cube, one draw. The roof sits
  at the top storey's slab; the facade rising 0.9 m past it is the parapet.
  Each band is a hole through the facade with a dark panel 8 cm back in it,
  not a box a few cm in front of it, which z-fought at a distance. The near
  plane is 0.1 m for the same reason (depth precision goes as 1/near). The
  roof has no collider: nothing reaches it.
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
- **Colliders**: its archetype's walls, floors, ramp and lamp posts as fixed bodies under
  the copy's own tags, each copy one `Physics.memo`.
- **Scene**: one `Scene.memo` per copy, rebuilt when the hour or its
  activation changes.
- **Lights**: every copy has a lobby spill and two street lamps; the frame
  takes the six nearest the eye, which is what fits the 8 forward-light slots
  beside the sun and ambient.

In the block, interiors pop in and out at the radius, through doors and
windows still in view: the fog only starts at 35 m (night) or 70 m (day).
Each town's `reach` (`hours.fun`'s `Reach`, by night and by day) is where its
fog ends and its far plane: 140 m for the office, the block and the grids.

The one overcast day bake holds at every yaw (it has no sun).

## The grids

`G` repeats the block, a 24 m street apart, to 4, 16 and 64 copies. Copy
ids nest (the first 4 of every grid are the block, the first 16 of 64 the
grid of 16), so an id names the same building, memo key and tags in every
town. What scales with the eye's surroundings, not with the town:

- **Index**: each copy is filed under the 53 m tile holding its footprint's
  middle; `Block.near` reads only the tiles around the eye. Activation,
  colliders, the nearest copy and the lights all query it.
- **Activation**: at most `K` interiors (4 by default; `K` applies in the
  block too, where a capped copy shows its street side). A copy qualifies
  within 30 m, or 34 m if it was on. Over `K`, the eye's own building wins,
  then copies already on (nearest first), then the rest (nearest first). A
  copy that is on is never swapped for a nearer one: within 34 m the eye is
  in its collar and may be looking in through its door or a window, which
  only the renderer's portal walk knows. It goes off past 34 m, or when the
  eye walks into another building and all `K` are taken.
- **Shells**: inactive copies are four `Scene.instanced` cubes for the whole
  town: massing up to the roof, parapet, canopy, apron, window bands (10 cm
  proud) and lamp posts lit; neon, lamp shades, and the entrance and
  the layout's windows glowing (the lit rooms' colour by night, neutral
  glass by day). An active copy draws its real
  street side and interior instead. Each copy's instances are built once at
  load; a change of the active set only gathers them (and re-uploads them),
  and a change of hour recolours the templates.
- **Colliders**: copies within 12 m of the player (to 16 m once on). A view
  jump makes its target solid before the teleport lands. The office alone
  and the block keep every copy solid.
- **Lights**: the six nearest the eye, from copies within 60 m (the office
  alone and the block still consider every copy).

## The island

`island.mjs` generates a 2 km island: `island.png`, a 16-bit heightmap of
2049² samples a metre apart (2ⁿ + 1, so every terrain LOD vertex lands on a
sample), and `islandLayout.fun`, the constants the game places it with. A
seabed, a beach, and inland a warped low-frequency mask that blends smooth
plains, rolling hills (gradient-noise fBm) and rocky mountains (ridged
multifractal) up to about 220 m. The town sits on calm plains; a ridge
300–450 m to its north rises inside the day fog, so the street sees a peak.
Trees keep to gentle ground below a 110 m treeline (the grade limit already
keeps them under about 55 m). The block's pad is one flat u16 level (the
four copies, the streets and a 6 m apron), with a 16 m smoothstep ramp
around it. Out to 30 m past the pad, the ground stays within 3 m above it,
so the collars (0–8 m over the street) hold the eye through the activation
radius; `functor test` checks it from the layout's recorded range.

```sh
node examples/liminal-office/island.mjs   # rewrites island.png, islandLayout.fun and every map, byte-identical on rerun
node examples/liminal-office/island.mjs --preset alpine   # A/B: rolling (low hills, ~85 m), mixed (default), alpine (~280 m)
node examples/liminal-office/island.mjs --biome snow      # only that biome's maps, to tune its palette
```

Only the default (`mixed`) is checked in, and the e2e's spots stand on it:
rerun without `--preset` before committing. A run takes about 30 s.

`I` swaps the flat ground for `Scene.terrain` and `Physics.heightfield`,
translated so the pad lands where the ground was: the block stands on it
unchanged. A box under the pad holds the walker while the heightmap decodes.
The terrain draws after the buildings (early-Z rejects it behind them) and
the sea plane after the terrain. The fog ends at the far plane, 500 m by day
and 200 m at night, so the renderer skips terrain past it. No grass, no
shadows. The island's Quest budget is 90 Hz with foveation off; not yet
measured on a headset (`config` still requests 120 Hz).

### Colormaps and biomes

The terrain is unlit: `Terrain.colormap` cross-fades a baked day map and
night map by the hour (`1 − daylight`), with one tiled detail texture near
the eye (`Terrain.colormapDetail`: a 2–3 m tile, gone by 80 m). The
`Terrain.layered` bands show only until the maps stream in. `island.mjs`
paints the maps from what it already computes per sample, 1024² over the
2 km (2 m a texel, one texel per odd sample):

- **Albedo**, from the biome's palette: meadow (broad lusher and drier
  patches, finer mottling), drier on the hills, gravel on moderate slopes,
  scree above the treeline, rock on steep faces, snow from the snow line,
  wet and dry sand at the beach. Dirt rings the pad and follows tracks out
  to the town groves and the south beach; the pad itself is road.
- **Day**: albedo × (ambient + sun · N·L) × horizon occlusion (8
  directions to ~350 m) × a soft blob under every tree and rock. The sun is
  60° up, so slopes facing away still read under the flashlight.
- **Night**: the same under the moon, plus the street lights: exterior.fun's
  lobby spill and sodium lamps on every copy, with the engine's falloff
  `(1 − d²/r²)²`. `islandLayout.fun` lists them; `functor test` checks them
  against game.fun's lights.

`islandLayout.fun` also records the sun, moon and ambient the maps were
baked with; on the island `Hours.islandOutdoorAt` lights the props and
buildings with those, so they match the ground.

| biome | palette | detail |
| --- | --- | --- |
| grass (default) | dry meadow, dirt, sand, grey rock and scree, snow on the crest | meadow grass |
| snow | snow, with rock on slopes and dark shingle at the beach | stomped snow |
| desert | cracked hardpan on the flats, rippled sand on the hills, warm gravel and red rock | rippled sand |

Each pair is 1024² RGB: 0.6–0.7 MB for day, 0.4 MB for night as PNG.
`island-normal.png` (0.6 MB) is shared. On the GPU a map is 5.3 MiB with
mips, so a biome is 10.7 MiB for its pair plus 4 MiB for the normal map
(no mips). 2048² would quarter a texel to 1 m² for 4× the memory (42.7 MiB
for a pair); at 2 m the detail texture already carries the near ground, so
it isn't worth it yet.

The engine takes one detail texture per terrain, so each biome uses its
dominant ground's. Detail textures, CC0, downscaled to 512²:
`detail-grass.jpg` is Poly Haven's
[forrest_ground_01](https://polyhaven.com/a/forrest_ground_01) by Rob Tuytel;
`detail-snow.jpg` is ambientCG's
[Snow006](https://ambientcg.com/view?id=Snow006); `detail-sand.jpg` is
ambientCG's [Ground098](https://ambientcg.com/view?id=Ground098).

### The flashlight

`L` (or the right trigger in VR) toggles a `Light.spot` held below the eye
(in VR, in the right hand). It is the frame's third light, ahead of the
point lights, so it is the colormap terrain's flashlight
(`Terrain.normalMap` with `island-normal.png`); the nearest-lights selector
gives it one of the six point-light slots. The lamps stay point lights for
the buildings and props.

### Rocks and trees

`island.mjs` also places a few props from the seed, stands them on its own
ground and writes them into `islandLayout.fun`: 112 firs in 16 groves (three
near the town) and 38 rocks of 90 boxes. None stands within 90 m of the
pad's edge (the band and its blend), no tree on the beach, nothing on steep
ground. The generator fails unless the caps hold: at most 10 trees within
40 m of any point, 150 trees and 100 rock boxes in all.

`Island.regroup` splits them from the eye, and again once it moves 4 m (so a
tier changes up to 4 m late): firs within 40 m draw as a three-plane card,
the rest within the fog as a two-plane card, and rocks within 150 m. A rock
is two or three lit cubes turned every way and a third buried. Each tier is one `Scene.instanced` node, so
at most five draws, drawn after the buildings and before the terrain. No
shadows. Trunks and rocks within 15 m of the eye get colliders.

The fir cards are night-terrain's (`impostor-fir-{a,b}-{near,far}.glb`, baked
by its `generate-impostors.mjs` from Poly Haven's CC0 [Fir Sapling
Medium](https://polyhaven.com/a/fir_sapling_medium) by Rob Tuytel and Rico
Cilliers). The copies are byte-identical, so git stores each once. Their
silhouettes are cut in geometry, not with `discard`. night-terrain stands its
fetched trunk-and-branch meshes inside the near card. This example keeps to
checked-in files, so the near card stands alone and looks blocky within a
few metres.

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

## The towers

`H` stands four towers round the block's crossroads, one per yaw: 8, 16,
12 and 8 floors (`towerLayout.fun`, derived by `tower.fun`). A 24 m square
plan, 3.3 m storeys, round a core that is the same rect on every floor
(`functor test` pins it): a dog-leg stair and a lift lobby (mustard carpet,
a panel grid, three brushed steel doors). The ground floor is a lobby with
the entrance, two side rooms and a back room; each typical floor a ring of
corridors (four convex strips) with two open offices, each with a window,
and four shut doors, a sodium sconce by every door; the roof the stair's
head room, a skylight slot over the well, and a terrace along the parapet.

- **Stairs**: two flights of 9 risers (0.183 m, 0.28 m treads, 33°) round
  a 0.3 m well, a half-landing at the north wall, a landing at each floor's
  door. A flight's steps have a stepped soffit, never below its own floor,
  so the flight below has 2.8 m of headroom; the ground flight
  stands solid on its floor; the top one ends at a balustrade. A middle
  floor's stairwell room is both the head of the flight below and the foot
  of the one above: walls a storey high, no floor but its landing, no
  ceiling. Each face goes to the stairwell room whose storey holds its
  lowest corner.
- **Cells**: floor `k`'s room `j` is cell `8k + j`; a horizontal portal
  (the stairwell less the landing) joins each floor's stairwell to the next.
  Down the well the frustum stays in the well, so only stairwell cells draw.
  The collar strips rise over the tallest tower's roof terrace, which a
  neighbour's eye can stand on.
- **Collision**: per flight, a smooth ramp through each lane's nosings
  and the half-landing (no step bumps), and lane parapets a storey high
  (stacked, they wall off the well). Each floor's colliders are built once,
  in its bake's space, and moved up per floor; the roof adds the terrace,
  its parapet and the plant room.
- **Bakes**: two, shared by every tower. `tower-typical` is one floor
  between the stairwells of the floors below and above (so light crosses the
  shaft), 512²; its floor's cell models are drawn once per floor, moved up,
  the same `Model.static` values (one GPU copy). `tower-ends` is the ground
  and the roof, each beside its neighbouring stairwell, 1024². With both
  hours that is 10.5 MB of lightmaps.
- **Street side**: the kit's facade, roof (open over the stairwell) and
  entrance canopy, plus the plant room and a planter on the roof and a red
  beacon; a window grid on every facade, one mesh (lit at random by night,
  dark glass by day), with holes where the real windows are. Shells draw the
  same grid.

Views in the towers (in the nearest tower): `1` the lobby, `2` a lift
lobby, `3` a corridor, `4` an office window (floor 3), `5` up the well from
floor 1, `6` down the well from floor 6's landing, `7` up a flight, `8`
floor 8's window; `9` the 8-floor tower's terrace facing the taller ones,
`0` the street between them. `node e2e/liminal-tower.mjs` climbs every tower
ground to roof and checks the well's culling.

## The cinema

`C` stands a 60 × 40 m multiplex on its parking lot (`cinemaLayout.fun`'s
data, `cinemaBuilding.fun`'s interior, `cinema.fun`'s unbaked parts), one
tall level. A 30 × 18 m lobby under a black ceiling and a teal soffit ring
with a pink tube: a concession counter under three glowing menu boards, a
ticket desk, poster light boxes. An arch opens onto the numbered corridor,
3.5 m wide and about 42 m long round one bend (two convex cells): purple
walls under an exposed black ceiling, teal and orange-red soffits stepping
on a wave, orange-red columns edged in pink and cyan neon, posters, a lit
number over each of four doors. Screens 1 and 2 are shut doors; 3 and 4 open
onto 14 × 18.5 m auditoria, 8 m tall: a dark screen by the door, twelve
rows of seats rising away from it, aisles down both sides, house lights
on. Auditorium 4 has a playset marooned by its screen and a staff door
into a white-tiled, fluorescent service corridor, which runs round to a
rear exit.

- **Palette** (`paints.fun`): building.fun's mesh pipeline over the
  cinema's own materials. Every sign, poster, menu, lens, tube and the
  screen is a cell of one glow atlas, so a cell draws one group for all of
  them. Unlit looks (the glow atlas, and the black of the ceilings) take
  no light, so their faces stay out of the bake: each cell draws a baked
  model of its lit faces and a plain one of its unlit ones. A cell draws at
  most 7 groups; `node e2e/liminal-cinema.mjs` counts at most 30 static
  draws from any of its views.
- **Cells**: a cell per room, 8 m tall (over the eye at the top of the
  rake), the kit's collar round the footprint. The rake and seats lie in
  their auditorium's cell.
- **The rake**: 12 rows of 1 m, 0.3 m each (17°), from 4.4 m in from the
  screen to the back wall. The character walks a smooth ramp through the
  rows' nosings, as on the stairs; each row of seats is a box collider, so
  only the aisles are walkable.
- **Seats**: not baked. 216 per auditorium, four tinted boxes each, one
  `Scene.instanced` cube (a draw) per auditorium, `Scene.inCells` it,
  unlit.
- **Bake**: `cinema`, one 1024² lightmap per hour at 12 texels/m, 8.4 MB
  for both. The lights: downlights in the ring, the lobby's ceiling, the
  soffits and the auditoria's ceilings, sconces down the auditoria's walls,
  pink and cyan points by the tubes, the menu's glow, the service
  corridor's panels; by day, daylight through the entrance and the rear
  exit.
- **Street side**: the kit's blank facade (9.5 m), a marquee over the
  entrance with four posters under it, a pink band under the roofline's
  cyan, and an 18 m sign tower with cyan tubes up its corners and pink
  bands across it; a parking lot of painted bays and eight sodium lamps.
  Past 34 m it draws as its shell.

Views in the cinema: `1` the lobby, `2` down the numbered corridor, `3`
auditorium 3 from beside its screen, `4` down its rake from the top row,
`5` up corridor A to the bend, `6` the service corridor, `7` the lobby
toward the entrance, `8` auditorium 4's playset, `9` the parking lot, `0`
the lot's far corner (the shell). `node e2e/liminal-cinema.mjs` walks in
from the lot to the top of auditorium 3's rake and back down.

## The mall

`P` puts a mall and its parking lot on the flat ground (`mallLayout.fun`,
derived by `mallInterior.fun` and `mall.fun`): a two-level concourse 76 ×
22 m, 4.5 m storeys, round a 30 × 22 m atrium court under a space-frame
pyramid skylight 13 m up. 5 m galleries run along the court's long sides
and 4 m bridges across its ends. A food court (fixed tables, drum pendants)
fills the west end; the east end is an anchor store's blank wall with a dark
entrance. Eight shops (8 × 10 m) are open along the north side, four a
level; every other shopfront is a shutter. Behind each open shop, through an
"EMPLOYEES ONLY" door, is a stockroom (steel shelving, cartons, a mop sink),
and behind the stockrooms a 76 m service corridor on each level: 2.2 m wide,
low ceiling, two panels lit in three, a shut door behind every shuttered
bay, and an EXIT at the lower one's east end.

- **Shops**: three fit-outs, empty clothing racks and a counter, a dead
  electronics display (unlit dark screens), and stacked cartons under
  "CLOSING" banners. Each is baked once, in its first bay; every other bay
  with that fit-out draws the same shop and stockroom models moved (the
  towers' trick), so the bake holds three shops whatever the bay count. The
  fit-outs are baked geometry, not unbaked props: they get the bake's
  occlusion and contact shadows for no draws, since they share the cell's
  atlas groups. Bake-only sheets close each home's shopfront and corridor
  door, so a shop bakes in its own light alone and every copy matches.

- **Cells**: the court's void is one box, floor to apex; round it, four
  strips on each level (the lower concourse, the galleries). Open-plan
  neighbours share a portal over their whole face. The balustrades (1.1 m,
  opaque) are geometry inside the galleries. Each open shop is two cells,
  its stockroom's doors offset so the corridor never sees the concourse;
  the corridors are one box a level. 34 cells (the yard outside the exit
  among them) and 2 more collar strips, 118 portals. A walk stops at a
  portal under 6% of the view both ways (`Cells.withMinPortal`), so far
  shops drop out; an unlit dark stand-in 5 cm behind each shop's and
  stockroom's surfaces fills a dropped one's opening (1 draw).
- **Escalators**: a stopped pair, 23 risers at 30°, from the court's floor
  west onto the west bridge, inside the void: cleated treads, balustrade
  panels, solid sides; lane 2 is barred. Collision: a ramp per lane through
  the nosings, boxes along the balustrades.
- **Fountain**: a basin, opaque unlit teal water, a planter island.
- **Bake**: one, `mall`, at 6 texels/m. `bake.mjs` bakes at the kit's 12, so
  `mallInterior.fun` gives the baker the mesh and its lights at half scale
  (light falloff is relative to range, so it lights the same; ambient
  occlusion reaches 2 m) and doubles the baked positions at load. 1024²,
  both hours 8.4 MB, the three shops and the corridors included (59% of the
  atlas). By day, a grid of sky spots under the skylight aims
  down, and the entrance gets the kit's sky and ground patches; no sun, so
  any yaw holds. The skylight's panels are unlit and not baked: sky-blue by
  day, navy by night.
- **Materials**: five groups (floor tiles, ceiling tiles, treads, a lit atlas
  and an unlit one), so a cell draws at most five; from the court a view
  draws at most 80.
- **Street side**: the kit's facade, canopy and neon; the roof open over
  the void, the pyramid's glazing in the window grid's slot (shells draw it
  too); two glazed ridges; the anchor store's box; the lot (asphalt, bay
  lines, wheel stops, 12 lamp posts), instanced with the street side.

Views in the mall: `1` the lot, `2` the court, `3` the south gallery, `4` the
escalators' foot, `5` down into the court from the north gallery, `6` the
food court, `7` into the electronics shop, `8` down the service corridor,
`9` the far lot, `0` the escalators' head. `node e2e/liminal-mall.mjs` walks
from the lot up the escalator and along the gallery, then through a shop,
its stockroom and the service corridor out of another shop and the exit,
and counts the draws (the concourse's from the corridor: none).

## Rebaking

Each bake lives in `baked/<bake>/`: `office`, `tower-typical`, `tower-ends`,
`cinema`, `mall`. Editing `layout.fun`, `building.fun` (which holds the bake jobs and their
lights) or an archetype's `<archetype>Layout.fun` (the cinema's
`paints.fun`, `cinemaLayout.fun` and `cinemaBuilding.fun`; the mall's
`mallLayout.fun` and `mallInterior.fun`) makes its committed bakes stale:

```sh
node examples/liminal-office/bake.mjs           # bake, then write baked/<archetype>/provenance.txt
node examples/liminal-office/bake.mjs --check   # rebake to a temp dir; fail unless byte-identical
```

`baked/office/provenance.txt` records a hash of those three files, the bakes, the
baker's version and its settings; `bake.mjs` refuses to record it if they
change mid-bake. It keeps one `night.mesh.bin`, failing if the day bake's
mesh differs. `provenance.fun` hashes the files again at load: on a mismatch,
or a missing lightmap, the game draws the interior flat-lit with a "stale
bake" banner, and `functor test` fails. The per-cell split derives from the
same hashed files and the baked mesh, so provenance covers it too. The bakes
(12 texels/m, 2 bounces) are byte-identical across runs.
