# `examples/hoverbike`

A hoverbike on a flat, walled test stand: four lift emitters hold it 1.2 m
up, rapier slides it along walls, and the camera stays level while the
fairing banks.

```sh
functor -d examples/hoverbike run native
functor -d examples/hoverbike test       # the controller against analytic flat ground
npm run test:hoverbike                   # a scripted ride over the debug server
```

## Controls

| | Keyboard | Gamepad |
| --- | --- | --- |
| Throttle | W | right trigger |
| Brake | S | left trigger |
| Steer | A / D | left stick |
| Chase / rider camera | C | north button (Y / △) |
| Respawn | R | |

## How it works

- **`ride.fun`** is the controller, as pure functions. Each emitter is a
  spring/damper on its own probe ray, with gravity fed forward so the bike
  rests exactly at hover height; a miss lifts nothing. It owns the velocity,
  yaw, thrust and drag, braking, and lateral grip. Every constant is a
  top-level `let`. Its expects drive it over an analytic flat ground: settling
  from several drop heights, throttle/coast curves, the turn radius, grip
  decay, and comfort bounds on yaw rate, lateral and vertical acceleration.
- **`game.fun`** makes the bike a `Physics.character`. `tick` casts the four
  probes, steps the controller, and returns `Physics.move`. The capsule rides
  far above the ground, so `snap`, the step-up and `grounded` never engage.
  When a move comes up short against a wall, the bike takes the velocity the
  move achieved and slides on.
- **Rig.** Both cameras are yaw-only, so the horizon stays level. The fairing
  banks into turns and pitches with acceleration; the saddle, bars, grips and
  gauge stay rigid in the yaw-only frame, and the rider camera sits at the eye.
  `draw` takes `alpha` and interpolates the rig from the last step's pose to
  the live one, so motion stays smooth on displays faster than the 60 Hz sim.
- **Speedometer.** `Frame.withUiTarget` paints the gauge into a render target
  shown on the cockpit (`Scene.screen`); its readings refresh at 10 Hz.
- **Quest.** `config` asks for 120 Hz and `settings` turns foveation off. The
  ride has no VR controls yet.

`course.fun` is the stand; `bike.fun` the mesh. The goldens
(`hoverbike-chase`, `hoverbike-rider` in `golden-scenarios.json`) replay
`ride.script`.
