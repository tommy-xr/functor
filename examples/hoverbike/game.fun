// hoverbike — ride a hoverbike around a flat test stand.
//
//   functor -d examples/hoverbike run native
//   functor -d examples/hoverbike test
//
// W/S throttle and brake, A/D steer, C swaps the chase and rider cameras,
// R respawns. Gamepad: right trigger throttle, left trigger brake, left stick
// steers, north button (Y / triangle) swaps cameras.
//
// The bike is a `Physics.character`, so rapier collides and slides it against
// walls and props. Each step `tick` casts the four hover probes, the pure
// controller in `ride.fun` turns them into a velocity, and `Physics.move`
// carries it out. `draw` interpolates the rig between steps with `alpha`.

let bikeTag = Physics.tag("bike")

// The collider: an upright capsule from 5 cm above the emitter plane to the
// rider's shoulders. Hovering, its bottom rides 1.25 m up, far outside the
// character's ground reach (2 × offset + 5 cm), so `grounded`, `snap` and the
// step-up never engage, and `maxSlope` only matters on contact. `snap` and
// `stepHeight` are 0 so even a bottomed-out bike isn't pulled down or lifted.
let colliderRadius = 0.8
let colliderHalfHeight = 0.1
let colliderLift = 0.95 // capsule centre above the emitter plane
let spawn = Vec3.make(5.0, Ride.hoverHeight + colliderLift, 0.0)

let bike =
  Physics.character(bikeTag, Physics.capsule(colliderHalfHeight, colliderRadius),
                    { stepHeight: 0.0, maxSlope: 50deg, snap: 0.0, offset: 0.02 })
    |> Physics.at(spawn)

let physics = (model) => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [bike, ..Course.bodies])

// Where the bike is: the capsule centre, its heading and lean.
type Pose = { x: float, y: float, z: float, yaw: float, roll: float, pitch: float }

type Model = {
  ride: Ride.State,
  controls: Ride.Controls,
  // The pose at the last step. `draw` interpolates from it to the live one.
  prev: Pose,
  // Where the last step started, to see how much of the move went through.
  last: Physics.position,
  // Each probe's distance to what it hit (`probeRange` on a miss).
  probes: List<float>,
  clearance: float,
  speed: float,
  steps: float,
  // The gauge's quantised readings, refreshed at 10 Hz.
  kmh: float,
  hover: float,
  riderCam: bool,
  swapHeld: bool,
  respawn: bool,
}

let spawnPosition: Physics.position = { x: Vec3.x(spawn), y: Vec3.y(spawn), z: Vec3.z(spawn) }
let poseOf = (p: Physics.position, s: Ride.State): Pose =>
  { x: p.x, y: p.y, z: p.z, yaw: s.yaw, roll: s.roll, pitch: s.pitch }

let init: Model = {
  ride: Ride.rest,
  controls: Ride.idle,
  prev: poseOf(spawnPosition, Ride.rest),
  last: spawnPosition,
  probes: Ride.emitters |> List.map((e) => Ride.hoverHeight),
  clearance: Ride.hoverHeight,
  speed: 0.0,
  steps: 0.0,
  kmh: 0.0,
  hover: Ride.hoverHeight,
  riderCam: false,
  swapHeld: false,
  respawn: false,
}

// ---- Input ---------------------------------------------------------------

let held = (k: Key.t, snap: Input.snapshot) => snap.heldKeys |> List.any((h) => h == k)
let pressed = (k: Key.t, snap: Input.snapshot) => snap.pressedKeys |> List.any((h) => h == k)
let level = (k: Key.t, snap: Input.snapshot) => if held(k, snap) then 1.0 else 0.0
let deadzone = (v: float) => if Math.abs(v) < 0.15 then 0.0 else v

let sampledInput = (m: Model, snap: Input.snapshot) =>
  let pad = (match snap.gamepad with
    | Option.Some(g) => { throttle: g.rightTrigger, brake: g.leftTrigger, steer: -deadzone(g.leftStick.x), swap: g.north }
    | Option.None => { throttle: 0.0, brake: 0.0, steer: 0.0, swap: false }) in
  let swap = pressed(Key.C, snap) || (pad.swap && not m.swapHeld) in
  { m with
    controls: { throttle: Math.max(level(Key.W, snap), pad.throttle),
                brake: Math.max(level(Key.S, snap), pad.brake),
                steer: Math.clamp(-1.0, 1.0, level(Key.A, snap) - level(Key.D, snap) + pad.steer) },
    riderCam: if swap then not m.riderCam else m.riderCam,
    swapHeld: pad.swap,
    respawn: m.respawn || pressed(Key.R, snap) }

// ---- Simulation ----------------------------------------------------------

// A point in the bike's yaw-only frame (+z forward, +x left), from `origin`.
let rigPoint = (origin: Vec3.t, yaw: float, x: float, y: float, z: float) =>
  Vec3.make(Vec3.x(origin) + x * Math.cos(yaw) + z * Math.sin(yaw),
            Vec3.y(origin) + y,
            Vec3.z(origin) - x * Math.sin(yaw) + z * Math.cos(yaw))

// The emitter plane's centre, below the capsule centre.
let rigOrigin = (x: float, y: float, z: float) => Vec3.make(x, y - colliderLift, z)

// One hover probe, cast straight down from 0.3 m above its emitter (inside
// the bike's own capsule, which it ignores) so a dipped emitter still hits.
let probeFrom = 0.3
let probe = (c: Physics.position, yaw: float, e): Ride.Probe =>
  let hit = Physics.castExcluding(bikeTag, rigPoint(rigOrigin(c.x, c.y, c.z), yaw, e.x, probeFrom, e.z),
                                  Vec3.make(0.0, -1.0, 0.0), Ride.probeRange + probeFrom) in
  { hit: hit.hit, distance: hit.distance - probeFrom }

// The velocity the last move actually achieved. Where a wall or prop cut the
// move short, that replaces the controller's velocity, so the bike slides on
// along the wall instead of pushing into it. Free moves keep the controller's
// own value, untouched by float noise in the positions.
let collided = (m: Model, c: Physics.position, dt: float): Ride.State =>
  let ax = (c.x - m.last.x) / dt in
  let ay = (c.y - m.last.y) / dt in
  let az = (c.z - m.last.z) / dt in
  let s = m.ride in
  let off = Math.abs(ax - s.vx) + Math.abs(ay - s.vy) + Math.abs(az - s.vz) in
  if m.steps > 0.0 && off > 0.05 then { s with vx: ax, vy: ay, vz: az } else s

let respawned = (m: Model) =>
  ({ init with riderCam: m.riderCam, swapHeld: m.swapHeld, controls: m.controls },
   Physics.teleport(bikeTag, spawn))

let tick = (m: Model, dt: float, tts) =>
  if dt <= 0.0 then (m, Effect.none())
  else if m.respawn then respawned(m)
  else
    let c = Physics.position(bikeTag) in
    let probes = Ride.emitters |> List.map((e) => probe(c, m.ride.yaw, e)) in
    let ride = Ride.step(m.controls, probes, dt, collided(m, c, dt)) in
    let distances = probes |> List.map((p) => if p.hit then p.distance else Ride.probeRange) in
    let clearance = List.sum(distances) / List.length(distances) in
    let speed = Math.sqrt(ride.vx * ride.vx + ride.vz * ride.vz) in
    let steps = m.steps + 1.0 in
    let refresh = Math.mod(steps, 6.0) == 0.0 in
    ({ m with ride: ride, prev: poseOf(c, m.ride), last: c, probes: distances, clearance: clearance,
              speed: speed, steps: steps,
              kmh: if refresh then Math.round(speed * 3.6) else m.kmh,
              hover: if refresh then Math.round(clearance * 10.0) / 10.0 else m.hover },
     Physics.move(bikeTag, Vec3.make(ride.vx * dt, ride.vy * dt, ride.vz * dt)))

// ---- Drawing -------------------------------------------------------------

let sky = Color.rgb(0.1, 0.06, 0.16)
let ion = Color.rgb(0.35, 1.0, 0.9)
let speedo = RenderTarget.named("speedo") |> RenderTarget.sized(128.0, 64.0)

let gaugeView = (m: Model) =>
  Ui.column([
    Ui.row([Ui.textColor(ion, $"{m.kmh}"), Ui.text(" km/h")]),
    Ui.text($"hover {Text.fixed(m.hover, 1.0)} m"),
  ])

// `alpha` of the way from the last step's pose to the live one.
let between = (a: Pose, b: Pose, alpha: float): Pose =>
  { x: a.x |> Math.lerp(b.x, alpha), y: a.y |> Math.lerp(b.y, alpha), z: a.z |> Math.lerp(b.z, alpha),
    yaw: a.yaw |> Math.lerp(b.yaw, alpha), roll: a.roll |> Math.lerp(b.roll, alpha),
    pitch: a.pitch |> Math.lerp(b.pitch, alpha) }

// Both cameras are yaw-only: the horizon stays level however the bike leans.
// The rider camera sits at the eye, 0.75 m above the saddle.
let camera = (riderCam: bool, origin: Vec3.t, yaw: float) =>
  if riderCam then
    Camera3D.firstPerson(rigPoint(origin, yaw, 0.0, 1.46, -0.35), Angle.radians(yaw), 0deg, 75deg)
  else Camera3D.lookAt(rigPoint(origin, yaw, 0.0, 3.0, -7.5), rigPoint(origin, yaw, 0.0, 1.2, 3.0))

let lights = [
  Light.ambient(Color.rgb(0.32, 0.28, 0.4)),
  Light.directional(Vec3.make(0.3, -0.6, -0.7), Color.rgb(1.0, 0.75, 0.65), 0.9),
]

let draw = (m: Model, tts, alpha: float) =>
  let p = between(m.prev, poseOf(Physics.position(bikeTag), m.ride), alpha) in
  let origin = rigOrigin(p.x, p.y, p.z) in
  let rig = Bike.draw(p.roll, p.pitch, m.probes, not m.riderCam, speedo)
    |> Scene.rotateY(Angle.radians(p.yaw)) |> Scene.translate(origin) in
  Frame.createLit(camera(m.riderCam, origin, p.yaw), Scene.group([Course.scene, rig]), lights)
    |> Frame.withClearColor(sky)
    |> Frame.withFog(Fog.linear(60.0, 260.0, sky))
    |> Frame.withUiTarget(speedo, gaugeView(m))

let ui = (m: Model) =>
  Ui.column([gaugeView(m), Ui.text(if m.riderCam then "rider cam (C)" else "chase cam (C)")])
    |> Ui.panel(Ui.bottomRight())

// Quest: 120 Hz, no foveation. The handlebars come in a later change.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120.0))
let settings = (m: Model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))
