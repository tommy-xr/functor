// dunes.fun — the dune course: a seeded 1 km heightfield with a flat start
// pad, a straight north over one gentle crest, a block on the runout, and a
// soft boundary.
//
// `generate-dunes.mjs` writes the heightmap and `dunesLayout.fun`; this file
// builds the terrain, bodies and scene from them. `Scene.terrain` and
// `Physics.heightfield` take the one `ground` descriptor, so the hover probes
// and `Physics.move` meet the ground that is drawn.

let ground =
  Terrain.heightmap(Asset.texture("dunes.png"), DunesLayout.size, DunesLayout.size,
                    DunesLayout.minHeight, DunesLayout.maxHeight)
  // 4 px fits the Quest vertex budget.
  |> Terrain.maxPixelError(4.0)
  // Dark troughs, rose dune tops, and pale sand from 4 m up, so the crest
  // and the higher dunes read from the saddle.
  |> Terrain.layered(Color.rgb(0.22, 0.13, 0.27), Color.rgb(0.62, 0.34, 0.44),
                     Color.rgb(0.3, 0.2, 0.32), Color.rgb(0.92, 0.66, 0.56), 4.0)

// A low sun from the west, across the lane, so the dune faces shade.
let lights = [
  Light.ambient(Color.rgb(0.24, 0.2, 0.32)),
  Light.directional(Vec3.make(-0.85, -0.35, 0.25), Color.rgb(1.0, 0.75, 0.6), 1.0),
]

// The block on the runout: 8 m wide, standing 3.5 m, sunk 1 m.
type Box = { x: float, y: float, z: float, w: float, h: float, d: float }
let block: Box = { x: DunesLayout.block.x, y: DunesLayout.block.ground + 1.25, z: DunesLayout.block.z,
                   w: 8.0, h: 4.5, d: 3.0 }
// Route posts, 2.2 m tall, standing on the ground.
let posts: List<Box> = DunesLayout.posts |> List.map((p) =>
  { x: p.x, y: p.y + 0.5, z: p.z, w: 0.3, h: 3.4, d: 0.3 })

let fixedBox = (name: string, b: Box) =>
  Physics.fixed(Physics.tag(name), Physics.box(b.w, b.h, b.d)) |> Physics.at(Vec3.make(b.x, b.y, b.z))

// The ground's tag: a dismount lands only on it.
let groundTag = Physics.tag("dunes")

let bodies = [
  Physics.heightfield(groundTag, ground),
  fixedBox("block", block),
  ..(posts |> List.indexedMap((i, p) => fixedBox($"post{i}", p)))
]

// ---- Boundary ------------------------------------------------------------

// Past `softLimit` on either axis, thrust outward cuts out, outward speed
// brakes and a spring pulls the bike back in. No wall to hit and no edge to
// fall off: the terrain runs on 60 m further, and the overshoot stays under
// 25 m. Facing back in, the throttle still works.
let softLimit = 440.0
let boundaryBrake = 4.0 // m/s² against outward speed
let boundaryPull = 0.3 // m/s² per metre past the limit

// Metres past the limit along one axis, signed outward (0 inside).
let past = (p: float) => Math.sign(p) * Math.max(0.0, Math.abs(p) - softLimit)

let recoverAxis = (over: float, dt: float, v: float) =>
  if over == 0.0 then v
  else
    let out = Math.sign(over) in
    v - out * (Math.min(Math.max(0.0, v * out), boundaryBrake * dt) + boundaryPull * Math.abs(over) * dt)

// The controls and velocity at (x, z): unchanged inside the limit. Heading
// `yaw` points along (sin, cos) in x and z.
let limit = (x: float, z: float, yaw: float, c: Ride.Controls): Ride.Controls =>
  if Math.sin(yaw) * Math.sign(past(x)) + Math.cos(yaw) * Math.sign(past(z)) > 0.0 then { c with throttle: 0.0 } else c
let recover = (x: float, z: float, dt: float, s: Ride.State): Ride.State =>
  { s with vx: recoverAxis(past(x), dt, s.vx), vz: recoverAxis(past(z), dt, s.vz) }

// Riding along x over flat ground from x0 at heading yaw and speed v,
// newest first.
type Run = { t: float, x: float, s: Ride.State }
let runFrom = (seconds: float, throttle: float, x0: float, yaw: float, v: float) =>
  List.range(Math.round(seconds * 60.0)) |> List.fold((acc, i) =>
    let r = acc |> List.head |> Option.defaultValue({ t: 0.0, x: x0, s: { Ride.rest with yaw: yaw, vx: v * Math.sin(yaw) } }) in
    let c = limit(r.x, 0.0, r.s.yaw, { throttle: throttle, brake: 0.0, steer: 0.0 }) in
    let s = Ride.step(c, Ride.hoverHeight, Ride.flatProbes(Ride.hoverHeight), Ride.dt60, r.s) |> recover(r.x, 0.0, Ride.dt60) in
    [{ t: (i + 1.0) * Ride.dt60, x: r.x + s.vx * Ride.dt60, s: s }, ..acc], [])
let furthest = (run: List<Run>) => run |> List.map((r) => r.x) |> List.maximum |> Option.defaultValue(0.0)
let endsInside = (run: List<Run>) => run |> List.head |> Option.map((r) => r.x <= softLimit) |> Option.defaultValue(false)
// Out across the limit at top speed, heading +x.
let runOut = (seconds: float, throttle: float) => runFrom(seconds, throttle, softLimit, Math.pi / 2.0, Ride.topSpeed)

// Throttle held: the bike stops under 25 m out and stays near the limit.
expect (
  let run = runOut(30.0, 1.0) in
  furthest(run) < softLimit + 25.0 && (run |> List.head |> Option.map((r) => r.x < softLimit + 1.0) |> Option.defaultValue(false))
)
// Throttle released: it comes back inside within 8 s.
expect (
  let run = runOut(8.0, 0.0) in
  furthest(run) < softLimit + 25.0 && endsInside(run)
)
// Stopped outside but facing back in, the throttle drives it home.
expect endsInside(runFrom(3.0, 1.0, softLimit + 15.0, 0.0 - Math.pi / 2.0, 0.0))
// Inside the limit nothing changes.
expect (
  let s = { Ride.rest with vx: 20.0, vz: -5.0 } in
  recover(softLimit - 1.0, 0.0 - softLimit + 1.0, Ride.dt60, s) == s
    && limit(0.0, 0.0, 0.0, { throttle: 1.0, brake: 0.0, steer: 0.0 }).throttle == 1.0
)
// Past it on z, only z recovers, and only outward thrust is cut.
expect (
  let s = recover(10.0, 0.0 - softLimit - 5.0, Ride.dt60, { Ride.rest with vx: 3.0, vz: -10.0 }) in
  let full = { throttle: 1.0, brake: 0.0, steer: 0.0 } in
  s.vx == 3.0 && s.vz > -10.0
    && limit(10.0, 0.0 - softLimit - 5.0, Math.pi, full).throttle == 0.0
    && limit(10.0, 0.0 - softLimit - 5.0, 0.0, full).throttle == 1.0
)

// ---- Scene ---------------------------------------------------------------

let ion = Bike.ion
let amber = Color.rgb(1.0, 0.62, 0.3)

let solid = (c: Color.t, b: Box) =>
  Scene.cube() |> Scene.scaleXYZ(b.w, b.h, b.d) |> Scene.lit(c) |> Scene.translate(Vec3.make(b.x, b.y, b.z))

// Posts glow cyan along the route, amber within 60 m of the crest.
let postGlow = (crest: bool) =>
  Scene.cube() |> Scene.emissive(if crest then amber else ion) |> Scene.instanced(
    posts |> List.filter((p) => (Math.abs(p.z - DunesLayout.crestZ) < 60.0) == crest)
      |> List.map((p) => Instance.at(Vec3.make(p.x, p.y, p.z)) |> Instance.scaleXYZ(p.w, p.h, p.d)))

let scene = Scene.group([
  Scene.terrain(ground),
  // The start pad: a darker disc on the flat.
  Scene.cylinder() |> Scene.scaleXYZ(30.0, 0.04, 30.0) |> Scene.lit(Color.rgb(0.2, 0.14, 0.24))
    |> Scene.translate(Vec3.make(DunesLayout.pad.x, 0.0, DunesLayout.pad.z)),
  solid(Color.rgb(0.75, 0.3, 0.55), block),
  Scene.cube() |> Scene.scaleXYZ(block.w + 0.02, 0.08, block.d + 0.02) |> Scene.emissive(ion)
    |> Scene.translate(Vec3.make(block.x, block.y + block.h / 2.0, block.z)),
  postGlow(false),
  postGlow(true),
])
