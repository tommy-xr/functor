# `examples/liminal-office`

One floor of an empty office block at night: a carpeted corridor under a drop
ceiling and fluorescent panels, open empty offices, a neon entrance on an empty
apron. Desktop and Quest; not the web (baked files load with `File.bytes`).

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
everything else from it:

- **Mesh**: each room's inner surfaces, door reveals, panels and signs, as one
  `Model.staticData`. Triangles are ordered room by room.
- **Bake**: that whole mesh is the `night` bake job, so light and occlusion
  cross between rooms. The baked mesh keeps triangle order, so it splits back
  into one `Model.static` per room (all vertices, the room's triangle range).
- **Cells**: one per room, portals on the doors. Each room's model is
  `Scene.inCells` its cell. The apron is a cell too, so culling works from the
  entrance; further out (more than 60 m) the interior isn't drawn at all.
- **Collision**: `Physics.fixed` boxes along every wall, a trimesh of the
  floors, a ground box. Walls are boxes because trimesh faces only block from
  the front: an inward-facing facade would let the capsule walk in from outside.
  The player is a `Physics.character`.

## Lighting

The panels exist only in the bake job: fixture lights submitted to the frame
would take the 8 forward-light slots. The frame's lights (moon, ambient, the
lobby's spill onto the apron, two sodium lamps) light the street and facade;
all are `Light.baked`, so the lightmapped interior ignores them. Nothing casts
shadows (a shadow pass draws every hidden room), and there is no glass
(translucent subtrees bypass culling). Panels and signs are `unlit` groups.

## Rebaking

Editing `layout.fun` or `building.fun` makes the committed bake stale:

```sh
node examples/liminal-office/bake.mjs           # bake, then write baked/provenance.txt
node examples/liminal-office/bake.mjs --check   # rebake to a temp dir; fail unless byte-identical
```

`baked/provenance.txt` records a hash of those two files, the baker's version
and its settings. `provenance.fun` hashes the files again at load: on a
mismatch the game draws the interior flat-lit with a "stale bake" banner, and
`functor test` fails. The bake (12 texels/m, 2 bounces) takes about 19 s on a
debug CLI and is byte-identical across runs.
