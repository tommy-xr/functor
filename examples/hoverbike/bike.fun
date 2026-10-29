// bike.fun — the hoverbike's mesh, in its own frame: origin at the centre
// of the emitter plane, +z forward, +x left, metres. About 2.2 m long.
//
// Two parts. The fairing (hull and lift modules) leans with `roll`/`pitch`;
// the saddle, bars, grips and gauge stay rigid in the yaw-only rig frame, so
// the rider's view and hands never tilt with the bodywork.

let graphite = Color.rgb(0.13, 0.13, 0.16)
let panel = Color.rgb(0.22, 0.22, 0.27)
let rubber = Color.rgb(0.05, 0.05, 0.06)
let ion = Color.rgb(0.35, 1.0, 0.9)
let tag = Color.rgb(1.0, 0.4, 0.7)

let box = (w: float, h: float, d: float, x: float, y: float, z: float, c: Color.t) =>
  Scene.cube() |> Scene.scaleXYZ(w, h, d) |> Scene.lit(c) |> Scene.translate(Vec3.make(x, y, z))
let glow = (w: float, h: float, d: float, x: float, y: float, z: float, c: Color.t) =>
  Scene.cube() |> Scene.scaleXYZ(w, h, d) |> Scene.emissive(c) |> Scene.translate(Vec3.make(x, y, z))

// One lift module over its emitter ring.
let liftModule = (e) =>
  Scene.group([
    box(0.38, 0.2, 0.5, 0.0, 0.12, 0.0, panel),
    Scene.cylinder() |> Scene.scaleXYZ(0.32, 0.03, 0.42) |> Scene.emissive(ion)
      |> Scene.translate(Vec3.make(0.0, 0.015, 0.0)),
  ])
    |> Scene.translate(Vec3.make(e.x, 0.0, e.z))

let fairing = Scene.group([
  box(0.8, 0.32, 2.0, 0.0, 0.38, 0.0, graphite),
  box(0.6, 0.2, 0.5, 0.0, 0.0, 0.0, graphite) |> Scene.rotateX(Angle.degrees(18.0))
    |> Scene.translate(Vec3.make(0.0, 0.4, 1.12)),
  box(0.5, 0.15, 1.2, 0.0, 0.6, 0.1, panel),
  glow(0.02, 0.03, 1.8, 0.405, 0.36, 0.0, ion),
  glow(0.02, 0.03, 1.8, -0.405, 0.36, 0.0, ion),
  glow(0.16, 0.02, 0.1, 0.0, 0.68, 0.45, tag),
  glow(0.5, 0.05, 0.02, 0.0, 0.42, -1.0, tag),
  Scene.group(Ride.emitters |> List.map(liftModule)),
])

// The gauge face: a screen showing the speedometer target, facing the rider.
let gauge = (screen: RenderTarget.t) =>
  Scene.group([
    box(0.28, 0.15, 0.04, 0.0, 0.0, 0.0, rubber),
    Scene.quad() |> Scene.screen(screen) |> Scene.scaleXYZ(0.24, 0.12, 1.0)
      |> Scene.rotateY(Angle.degrees(180.0)) |> Scene.translate(Vec3.make(0.0, 0.0, -0.025)),
  ])
    |> Scene.rotateX(Angle.degrees(-25.0))
    |> Scene.translate(Vec3.make(0.0, 1.08, 0.52))

let grip = (x: float) =>
  Scene.cylinder() |> Scene.scaleXYZ(0.05, 0.14, 0.05) |> Scene.lit(rubber)
    |> Scene.rotateZ(Angle.degrees(90.0)) |> Scene.translate(Vec3.make(x, 1.0, 0.62))

let cockpit = (screen: RenderTarget.t) =>
  Scene.group([
    box(0.32, 0.1, 0.6, 0.0, 0.66, -0.4, rubber),
    box(0.06, 0.35, 0.06, 0.0, 0.775, 0.6, panel),
    box(0.5, 0.045, 0.045, 0.0, 1.0, 0.62, panel),
    grip(0.31),
    grip(-0.31),
    gauge(screen),
  ])

// A seated rider for the chase camera (the rider camera sits at its eye).
let rider = Scene.group([
  box(0.4, 0.6, 0.25, 0.0, 0.0, 0.0, Color.rgb(0.2, 0.2, 0.25)) |> Scene.rotateX(Angle.degrees(15.0))
    |> Scene.translate(Vec3.make(0.0, 1.05, -0.3)),
  Scene.sphere() |> Scene.scale(0.14) |> Scene.lit(Color.rgb(0.3, 0.3, 0.36))
    |> Scene.translate(Vec3.make(0.0, 1.48, -0.25)),
  glow(0.2, 0.04, 0.02, 0.0, 1.5, -0.11, ion),
  box(0.08, 0.08, 0.92, 0.0, 0.0, 0.0, Color.rgb(0.2, 0.2, 0.25)) |> Scene.rotateX(Angle.degrees(16.0))
    |> Scene.translate(Vec3.make(0.25, 1.12, 0.18)),
  box(0.08, 0.08, 0.92, 0.0, 0.0, 0.0, Color.rgb(0.2, 0.2, 0.25)) |> Scene.rotateX(Angle.degrees(16.0))
    |> Scene.translate(Vec3.make(-0.25, 1.12, 0.18)),
  box(0.14, 0.14, 0.5, 0.15, 0.76, -0.15, Color.rgb(0.2, 0.2, 0.25)),
  box(0.14, 0.14, 0.5, -0.15, 0.76, -0.15, Color.rgb(0.2, 0.2, 0.25)),
])

// The lift beams from each emitter to what its probe hit. They lean with the
// fairing, so they stay on their emitters.
let beam = (e, distance: float) =>
  Scene.cylinder() |> Scene.scaleXYZ(0.3, distance, 0.4) |> Scene.emissive(ion)
    |> Scene.translate(Vec3.make(e.x, -distance / 2.0, e.z))

let beams = (distances: List<float>) =>
  Scene.group(Ride.emitters |> List.zip(distances) |> List.map((p) =>
    let (e, d) = p in beam(e, d)))
    |> Scene.opacity(0.15)

// The whole bike in its yaw-only rig frame.
let draw = (roll: float, pitch: float, distances: List<float>, withRider: bool, screen: RenderTarget.t) =>
  Scene.group([
    // Lean about a pivot at saddle height, so the fairing banks under the rider.
    Scene.group([fairing, beams(distances)]) |> Scene.translate(Vec3.make(0.0, -0.66, 0.0))
      |> Scene.rotateX(Angle.radians(-pitch)) |> Scene.rotateZ(Angle.radians(-roll))
      |> Scene.translate(Vec3.make(0.0, 0.66, 0.0)),
    cockpit(screen),
    if withRider then rider else Scene.group([]),
  ])
