# `examples/hoverbike`

A hoverbike on a seeded 1 km dune course: four lift emitters hold it 1.2 m
over the terrain, rapier slides it off the obstacles, and the camera stays
level while the fairing banks. T switches to the flat, walled test stand.

```sh
functor -d examples/hoverbike run native
functor -d examples/hoverbike test       # the controller against analytic ground
npm run test:hoverbike                   # scripted rides over the debug server
npm run test:hoverbike-xr                # the handlebars, with injected XR samples
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

With a headset:

| | Quest Touch |
| --- | --- |
| Hold the bars | squeeze both grips (at the grips) |
| Steer | turn the bars |
| Throttle / brake | right / left trigger |
| Vignette strength (off, low, medium, high) | left X |
| Smooth / stepped turning | left Y |
| Seated / standing | right A |
| Recalibrate (hold still) | left menu |
| Respawn / dunes or test stand | right B / right stick click |

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
- **Quest.** `config` asks for 120 Hz and `settings` turns foveation off.

## Riding with a headset

`bars.fun` holds the rules, as pure functions with expects; `e2e/hoverbike-xr.mjs`
drives them with injected XR samples.

- **Calibration.** The camera sits at the calibrated eye: once the head and
  the bike have held still for half a second, the head's position and heading
  are taken as the eye, seated (default) or standing. Saddle and grips stay
  rigid in the yaw-only frame, so the horizon is level and latched hands
  don't drift from the controllers.
- **Grips.** A fresh squeeze within 15 cm of a grip latches that hand. Power
  needs both, and a throttle released since grabbing. Releasing a grip or
  losing its tracking cuts the throttle and brakes at 40%; only a fresh grab
  brings power back.
- **Steering.** The grip-to-grip vector's turn from where the bars armed,
  with a 3° dead zone, full lock at 20° and a moving average. Grip twist is
  ignored.
- **Vignette.** A feathered annulus (`vignette.png`, from
  `generate-vignette.mjs`) drawn with `Frame.with2D`. It closes quickly and
  opens slowly with acceleration, yaw rate, lateral acceleration and vertical
  speed over crests, and is absent when clear. On Quest it is drawn the same
  in both eyes, centred on each eye buffer rather than each eye's (asymmetric)
  view axis; whether that fuses comfortably is a check in the headset.
- **Turning.** Smooth, at the controller's bounded yaw rate, by default.
  Stepped turning snaps 30° below 2 m/s (bike and rig together, behind a
  short fade), and is smooth above it.

`dunes.fun` is the course, `stand.fun` the test stand, `bike.fun` the mesh.
The goldens (`hoverbike-crest`, `hoverbike-rider` in `golden-scenarios.json`)
replay `ride.script`.
