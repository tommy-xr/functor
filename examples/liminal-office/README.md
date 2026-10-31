# `examples/liminal-office`

One floor of an empty office block at night: a carpeted corridor under a drop
ceiling and fluorescent panels, open empty offices, a neon entrance on an empty
apron. Desktop; Quest is configured (120 Hz, foveation off) but not yet checked
on a headset. Not the web: baked files load with `File.bytes`.

```sh
functor -d examples/liminal-office run native
functor -d examples/liminal-office run native --emulate-xr   # arrows drive the right thumbstick
functor -d examples/liminal-office run vr
functor -d examples/liminal-office test     # layout, cells, provenance; no GPU
npm run test:liminal-office                 # walk the route, wall push, culling, goldens
```

| Input | Action |
| --- | --- |
| mouse (click to capture) | look |
| `W` `A` `S` `D` / arrows | walk, strafe / turn |
| `1` `2` `3` | jump to the entrance, corridor, office view |
| VR left thumbstick | walk, strafe |
| VR right thumbstick | turn, walk |

## One layout, four consumers

`layout.fun` is plain data: the footprint, rooms (rects between wall centre
lines), doors on shared walls, ceiling panels and signs. `building.fun` derives
the interior from it, and `exterior.fun` the street side:

- **Mesh**: each room's inner surfaces, panels and signs, then each door's
  reveal, as one `Model.staticData`, in contiguous parts.
- **Bake**: that whole mesh is the `night` bake job, so light and occlusion
  cross between rooms. The baked mesh keeps triangle order, so it splits back
  into one `Model.static` per part (all vertices, the part's triangle range).
- **Cells**: one per room, portals on the doors. A room's model is
  `Scene.inCells` its cell; a door reveal is in both of its rooms. The apron is
  a cell too, so culling works from the entrance. Past the fog's far end
  (140 m) the interior isn't drawn at all.
- **Collision**: `Physics.fixed` boxes along every wall, a trimesh of the
  floors. Walls are boxes because trimesh faces only block from the front: an
  inward-facing facade would let the capsule walk in from outside. The street
  (a ground box, the lamp posts) sits 3 cm lower. The player is a
  `Physics.character`.

## Lighting

The panels exist only in the bake job: fixture lights submitted to the frame
would take the 8 forward-light slots. The frame's lights (moon, ambient, the
lobby's spill onto the apron, two sodium lamps) light the street and facade;
all are `Light.baked`, so the lightmapped interior ignores them. Nothing casts
shadows (a shadow pass draws every hidden room), and there is no glass
(translucent subtrees bypass culling). Panels and signs are `unlit` groups.

## Rebaking

Editing `layout.fun` or `building.fun` (which holds the bake job and its
lights) makes the committed bake stale:

```sh
node examples/liminal-office/bake.mjs           # bake, then write baked/provenance.txt
node examples/liminal-office/bake.mjs --check   # rebake to a temp dir; fail unless byte-identical
```

`baked/provenance.txt` records a hash of those two files, the baker's version
and its settings; `bake.mjs` refuses to record it if they change mid-bake. `provenance.fun` hashes the files again at load: on a
mismatch the game draws the interior flat-lit with a "stale bake" banner, and
`functor test` fails. The bake (12 texels/m, 2 bounces) takes about 19 s on a
debug CLI and is byte-identical across runs.
