# `examples/sniper-scope`

A magnified rifle scope on a flat 200 m range. Desktop first-person for now;
tracked-rifle VR controls come later.

```sh
functor -d examples/sniper-scope run native
functor -d examples/sniper-scope test       # inline expects, no GPU
npm run test:sniper-scope                   # scripted hit / miss / obstructed shots, headless
```

| Input | Action |
| --- | --- |
| mouse | look (slower while shouldered, scaled by the zoom) |
| right mouse (hold) | shoulder the rifle |
| wheel / `Z` | zoom preset: 12° or 6° vertical field of view |
| left mouse | fire (shouldered only; 1 s bolt cycle) |
| `Space` (hold) | hold breath: steadies the sway for up to 4 s |

`Key` has no Shift variant yet, so breath is on `Space`. `Z` also changes the
zoom because input scripts cannot scroll.

## How the scope works

- **Image.** A second camera at the same eye, with the preset FOV, renders
  into a 512² `RenderTarget` with `Frame.withRenderTarget`. The pass is
  declared only while the rifle is fully shouldered; at the hip the lens is
  dark glass. The scope frame has no shadows, grass or translucency.
- **Reticle.** A `Frame.with2D` pass inside the target frame, in the
  target's own pixels, so it stays on the glass.
- **Lens.** `Scene.cylinder` has solid caps, so the tube is built from
  `Mesh.fromPolygons` rings: open eyepiece, hollow bore, closed objective.
  The lens is a `Scene.quad` with `Scene.screen`, recessed inside the tube
  and turned 180° to face the eye. Seen down the tube, the walls clip the
  quad to a circle. The corners stay hidden while
  `√2 · lensR / lensZ ≤ tubeR / eyeZ`.
- **Clip planes.** Both cameras set `Camera3D.clip`. The default far plane
  is 100 m and would cut off the 200 m plate.

## Ballistics

A hitscan `Physics.raycast` from the muzzle. The bore starts 5 cm below the
line of sight and crosses it at 100 m. It lands
`sightHeight · (d / 100 − 1)` from the crosshair:

| Range | Impact |
| --- | --- |
| 50 m | 2.5 cm low |
| 100 m | on |
| 200 m | 5 cm high |

A shot rings a plate (scoring its distance), misses into the backstop or the
sky, or is blocked by the ground or a berm short of the backstop. Impact
markers keep the last eight strikes.

## Sway and breath

A figure-eight of two sines, ±2 mrad wide, applied only as far as the rifle
is shouldered. Holding breath cuts it to 20%. Running out of breath (4 s)
forces a 1.5 s recovery with 160% sway and no hold.

## Range

Plates are 1.2 × 0.6 m: 50 m on the shooter's left, 100 m on the right and
200 m on the centre line, so the long sight line stays clear. Each plate's
frame has a tall cyan-capped post on the left and a short post on the right,
so a mirrored scope image shows up next to the main view.

## Goldens

`golden-scenarios.json` renders three scenarios: the hip view at a fixed
time, and the shouldered view at 12° and at 6°. `--fixed-time` pins the
model at `init`, which is the hip view. The shouldered states come from
`shoulder.script` at fixed frames, using the scripted fixed-step path.
