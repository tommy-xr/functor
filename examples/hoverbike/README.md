# `examples/hoverbike`

A hoverbike on a seeded 1 km dune course: four lift emitters hold it 1.2 m
over the terrain, rapier slides it off the obstacles, and the camera stays
level while the fairing banks. T switches to the flat, walled test stand.

```sh
functor -d examples/hoverbike run native
functor -d examples/hoverbike test       # the controller against analytic ground
npm run test:hoverbike                   # scripted rides over the debug server
node examples/hoverbike/generate-dunes.mjs   # rewrite dunes.png + dunesLayout.fun
```

## Controls

| | Keyboard | Gamepad |
| --- | --- | --- |
| Throttle | W | right trigger |
| Brake | S | left trigger |
| Steer | A / D | left stick |
| Chase / rider camera | C | north button (Y / △) |
| Respawn | R | |
| Dunes / test stand | T | |

## The dune course

Start on the flat pad and head north (the way the bike faces): a straight
over one gentle 5 m crest, a flat runout, then a block across the lane to
pass or stop for. Posts mark the lane, amber over the crest.

- **Generated.** `generate-dunes.mjs` (seed 42) writes the 16-bit
  `dunes.png` and `dunesLayout.fun`, the constants the game places things
  with (pad, crest, block, posts on the ground). Rerunning it (same Node)
  reproduces both byte for byte.
- **Resolution.** 1025² samples over 1 km is ~0.98 m a cell: the finest
  `Physics.heightfield` keeps (1025 per axis), and about the bike's emitter
  footprint (0.9 × 1.5 m). The 4 km terrain's ~4 m cells would facet slopes
  into steps that kick the hover springs.
- **Gentle.** Grades stay under 8.4% (6.8% on the lane). Climbing, the springs ride low by
  `springDamping × climb rate / springRate` (an expect checks this against an
  inclined plane): 0.66 m clearance at top speed on 8%, never near the ground.
- **One descriptor.** `Scene.terrain` and `Physics.heightfield` share
  `Dunes.ground`, so the probes and `Physics.move` meet the ground drawn.
- **Readiness.** After every spawn, riding waits until all four probes see
  ground. On a cold load the heightfield's collider appears only once its PNG
  decodes; until then the bike holds still instead of falling.
- **Boundary.** Past 440 m on either axis, thrust outward cuts out, outward
  speed brakes and a spring pulls the bike back. It stops within 25 m; the
  terrain runs on to 500 m. Facing back in, the throttle still works.
- **Rendering.** Fog closes the view at 420 m, inside a 450 m far plane;
  `Terrain.maxPixelError(4)`, a few dozen instanced posts, no shadows. A low
  side sun and height bands keyed to the dunes (pale sand from 4 m) make the
  gentle relief read.

## How it works

- **`ride.fun`** is the controller, as pure functions. Each emitter is a
  spring/damper on its own probe ray, with gravity fed forward so the bike
  rests exactly at hover height; a miss lifts nothing. It owns the velocity,
  yaw, thrust and drag, braking, and lateral grip. Every constant is a
  top-level `let`. Its expects drive it over analytic flat and inclined
  ground: settling from several drop heights, slope following,
  throttle/coast curves, the turn radius, grip decay, and comfort bounds on
  yaw rate, lateral and vertical acceleration.
- **`game.fun`** makes the bike a `Physics.character`. `tick` casts the four
  probes, steps the controller, and returns `Physics.move`. The capsule rides
  far above the ground, so `snap`, the step-up and `grounded` never engage.
  When a move comes up short against a wall, the bike takes the velocity the
  move achieved and slides on. The capsule is round in plan: wider than the bike, and
  about 0.6 m short of its nose head-on.
- **Rig.** Both cameras are yaw-only, so the horizon stays level. The fairing
  banks into turns and pitches with acceleration; the saddle, bars, grips and
  gauge stay rigid in the yaw-only frame, and the rider camera sits at the eye.
  `draw` takes `alpha` and interpolates the rig from the last step's pose to
  the live one, so motion stays smooth on displays faster than the 60 Hz sim.
- **Speedometer.** `Frame.withUiTarget` paints the gauge into a render target
  shown on the cockpit (`Scene.screen`); its readings refresh at 10 Hz.
- **Quest.** `config` asks for 120 Hz and `settings` turns foveation off. The
  ride has no VR controls yet.

`dunes.fun` is the course, `stand.fun` the test stand, `bike.fun` the mesh.
The goldens (`hoverbike-crest`, `hoverbike-rider` in `golden-scenarios.json`)
replay `ride.script`.
