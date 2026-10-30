// hoverbike — ride a hoverbike over a seeded dune course, or the flat test
// stand.
//
//   functor -d examples/hoverbike run native
//   functor -d examples/hoverbike test
//
// W/S throttle and brake, A/D steer, C swaps the chase and rider cameras,
// R respawns, T switches between the dunes and the stand. Gamepad: right
// trigger throttle, left trigger brake, left stick steers, north button
// (Y / triangle) swaps cameras.
//
// The bike is a `Physics.character`, so rapier collides and slides it against
// the ground, walls and props. Each step `tick` casts the four hover probes,
// the pure controller in `ride.fun` turns them into a velocity, and
// `Physics.move` carries it out. `draw` interpolates the rig between steps
// with `alpha`. After every spawn, riding waits until all four probes see
// ground: on a cold load the dunes' collider appears only once its heightmap
// decodes, and until then the bike would fall.

let bikeTag = Physics.tag("bike")

// The collider: an upright capsule from 5 cm above the emitter plane to the
// rider's shoulders. Hovering, its bottom rides 1.25 m up, far outside the
// character's ground reach (2 × offset + 5 cm), so `grounded`, `snap` and the
// step-up never engage, and `maxSlope` only matters on contact. `snap` and
// `stepHeight` are 0 so even a bottomed-out bike isn't pulled down or lifted.
// Round in plan, it is wider than the bike (1.6 m) and shorter (the nose
// overhangs it by about 0.6 m head-on); a heading-aligned collider can come later.
let colliderRadius = 0.8
let colliderHalfHeight = 0.1
let colliderLift = 0.95 // capsule centre above the emitter plane
// Literals, not derived from the tunables or the layout: the declaration is
// the body's spawn, and changing it on a hot reload would teleport the bike.
let dunesSpawn = Vec3.make(100.0, 2.15, -380.0)
let standSpawn = Vec3.make(605.0, 2.15, 0.0)
// Both put the emitter plane at hover height over flat ground at 0: the
// dunes' start pad and the stand's floor.
expect Vec3.x(dunesSpawn) == DunesLayout.pad.x && Vec3.z(dunesSpawn) == DunesLayout.pad.z
  && Math.abs(Vec3.y(dunesSpawn) - (Ride.hoverHeight + colliderLift)) < 0.000001
expect Vec3.x(standSpawn) == Stand.originX + 5.0 && Vec3.y(standSpawn) == Vec3.y(dunesSpawn)

let bike =
  Physics.character(bikeTag, Physics.capsule(colliderHalfHeight, colliderRadius),
                    { stepHeight: 0.0, maxSlope: 50deg, snap: 0.0, offset: 0.02 })
    |> Physics.at(dunesSpawn)

// Both courses are always declared, side by side; T teleports between them.
let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0), [bike] |> List.append(Dunes.bodies) |> List.append(Stand.bodies))

type Course =
  | OnDunes
  | OnStand

// Where the bike is: the capsule centre, its heading and lean.
type Pose = { x: float, y: float, z: float, yaw: float, roll: float, pitch: float }

type Model = {
  course: Course,
  // Whether all four probes have seen ground since the last spawn.
  ready: bool,
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
  switchCourse: bool,
}

let positionOf = (v: Vec3.t): Physics.position => { x: Vec3.x(v), y: Vec3.y(v), z: Vec3.z(v) }
let spawnOf = (course: Course) =>
  match course with
  | OnDunes => dunesSpawn
  | OnStand => standSpawn
let poseOf = (p: Physics.position, s: Ride.State): Pose =>
  { x: p.x, y: p.y, z: p.z, yaw: s.yaw, roll: s.roll, pitch: s.pitch }

let init: Model = {
  course: OnDunes,
  ready: false,
  ride: Ride.rest,
  controls: Ride.idle,
  prev: poseOf(positionOf(dunesSpawn), Ride.rest),
  last: positionOf(dunesSpawn),
  probes: Ride.emitters |> List.map((e) => Ride.hoverHeight),
  clearance: Ride.hoverHeight,
  speed: 0.0,
  steps: 0.0,
  kmh: 0.0,
  hover: Ride.hoverHeight,
  riderCam: false,
  swapHeld: false,
  respawn: false,
  switchCourse: false,
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
    respawn: m.respawn || pressed(Key.R, snap),
    switchCourse: m.switchCourse || pressed(Key.T, snap) }

// ---- Simulation ----------------------------------------------------------

// A point in the bike's yaw-only frame (+z forward, +x left), from `origin`.
let rigPoint = (origin: Vec3.t, yaw: float, x: float, y: float, z: float) =>
  Vec3.make(Vec3.x(origin) + x * Math.cos(yaw) + z * Math.sin(yaw),
            Vec3.y(origin) + y,
            Vec3.z(origin) - x * Math.sin(yaw) + z * Math.cos(yaw))

// The emitter plane's centre, below the capsule centre.
let rigOrigin = (x: float, y: float, z: float) => Vec3.make(x, y - colliderLift, z)

// One hover probe, cast straight down from 0.3 m above its emitter (ignoring
// the bike's own capsule) so a dipped emitter still hits.
let probeFrom = 0.3
let probe = (c: Physics.position, yaw: float, e): Ride.Probe =>
  let hit = Physics.castExcluding(bikeTag, rigPoint(rigOrigin(c.x, c.y, c.z), yaw, e.x, probeFrom, e.z),
                                  Vec3.make(0.0, -1.0, 0.0), Ride.probeRange + probeFrom) in
  { hit: hit.hit, distance: hit.distance - probeFrom }

// The velocity the last move actually achieved. Where a wall or prop cut the
// move short, that replaces the controller's velocity, so the bike slides on
// along the wall instead of pushing into it. Free moves keep the controller's
// own value, untouched by float noise in the positions, and so does anything
// faster than commanded (a teleport, a hot-reloaded declaration): a collision
// only ever shortens a move.
let collided = (m: Model, c: Physics.position, dt: float): Ride.State =>
  let ax = (c.x - m.last.x) / dt in
  let ay = (c.y - m.last.y) / dt in
  let az = (c.z - m.last.z) / dt in
  let s = m.ride in
  let off = Math.abs(ax - s.vx) + Math.abs(ay - s.vy) + Math.abs(az - s.vz) in
  let achieved = Math.sqrt(ax * ax + ay * ay + az * az) in
  let commanded = Math.sqrt(s.vx * s.vx + s.vy * s.vy + s.vz * s.vz) in
  if m.steps > 0.0 && off > 0.05 && achieved <= commanded + 0.05 then { s with vx: ax, vy: ay, vz: az } else s

// Back to a course's spawn, at rest and waiting for ground again.
let startOn = (course: Course, m: Model) =>
  let at = positionOf(spawnOf(course)) in
  ({ init with course: course, prev: poseOf(at, Ride.rest), last: at,
               riderCam: m.riderCam, swapHeld: m.swapHeld, controls: m.controls },
   Physics.teleport(bikeTag, spawnOf(course)))

// One step, from where the bike is and what its probes saw. Until the probes
// have all seen ground the ride holds still; on the dunes, the boundary cuts
// the throttle and steers the bike back in.
let advance = (m: Model, c: Physics.position, probes: List<Ride.Probe>, dt: float): Model =>
  let ready = m.ready || Ride.sensed(probes) in
  let ride =
    if not ready then m.ride
    else
      match m.course with
      | OnDunes => Ride.step(Dunes.limit(c.x, c.z, m.controls), probes, dt, collided(m, c, dt)) |> Dunes.recover(c.x, c.z, dt)
      | OnStand => Ride.step(m.controls, probes, dt, collided(m, c, dt)) in
  let distances = probes |> List.map((p) => if p.hit then p.distance else Ride.probeRange) in
  let clearance = List.sum(distances) / List.length(distances) in
  let speed = Ride.speedOf(ride) in
  let steps = m.steps + 1.0 in
  let refresh = Math.mod(steps, 6.0) == 0.0 in
  { m with ready: ready, ride: ride, prev: poseOf(c, m.ride), last: c, probes: distances, clearance: clearance,
           speed: speed, steps: steps,
           kmh: if refresh then Math.round(speed * 3.6) else m.kmh,
           hover: if refresh then Math.round(clearance * 10.0) / 10.0 else m.hover }

let tick = (m: Model, dt: float, tts) =>
  if dt <= 0.0 then (m, Effect.none())
  else if m.switchCourse then startOn(if m.course == OnDunes then OnStand else OnDunes, m)
  else if m.respawn then startOn(m.course, m)
  else
    let c = Physics.position(bikeTag) in
    let next = advance(m, c, Ride.emitters |> List.map((e) => probe(c, m.ride.yaw, e)), dt) in
    (next,
     if next.ready then Physics.move(bikeTag, Vec3.make(next.ride.vx * dt, next.ride.vy * dt, next.ride.vz * dt))
     else Effect.none())

// Readiness, driven without a world.
let missed = Ride.emitters |> List.map((e) => { hit: false, distance: 0.0 })
let onGround = Ride.flatProbes(Ride.hoverHeight)
let waitFor = (steps: float, probes: List<Ride.Probe>, m: Model) =>
  List.range(steps) |> List.fold((acc, i) => advance(acc, acc.last, probes, Ride.dt60), m)
// Two seconds without ground: still waiting, and the bike hasn't moved or
// begun to fall, whatever the controls say.
expect (
  let m = waitFor(120.0, missed, { init with controls: { throttle: 1.0, brake: 0.0, steer: 1.0 } }) in
  not m.ready && m.ride == Ride.rest && m.last == init.last
)
// The first step that senses ground starts the ride. It stays started: a
// miss after that is a real drop, and the bike falls.
expect (
  let m = waitFor(1.0, onGround, waitFor(30.0, missed, init)) in
  let dropped = waitFor(1.0, missed, m) in
  m.ready && dropped.ready && dropped.ride.vy < 0.0
)
// A respawn or course switch waits again, at the new spawn.
expect (
  let (m, _) = startOn(OnStand, waitFor(1.0, onGround, init)) in
  not m.ready && m.course == OnStand && m.last == positionOf(standSpawn) && m.prev.x == Vec3.x(standSpawn)
)

// ---- Drawing -------------------------------------------------------------

let sky = Color.rgb(0.1, 0.06, 0.16)
// Dusk haze over the dunes: the fog bounds the view at 420 m, inside the far
// plane, so the terrain beyond it is never drawn.
let haze = Color.rgb(0.3, 0.17, 0.3)
let speedo = RenderTarget.named("speedo") |> RenderTarget.sized(128.0, 64.0)

let gaugeView = (m: Model) =>
  Ui.column([
    Ui.row([Ui.textColor(Bike.ion, $"{m.kmh}"), Ui.text("km/h")]),
    Ui.text($"hover {Text.fixed(m.hover, 1.0)} m"),
  ])

// `alpha` of the way from the last step's pose to the live one.
let between = (a: Pose, b: Pose, alpha: float): Pose =>
  { x: a.x |> Math.lerp(b.x, alpha), y: a.y |> Math.lerp(b.y, alpha), z: a.z |> Math.lerp(b.z, alpha),
    yaw: a.yaw |> Math.lerp(b.yaw, alpha), roll: a.roll |> Math.lerp(b.roll, alpha),
    pitch: a.pitch |> Math.lerp(b.pitch, alpha) }
expect (
  let a = poseOf(positionOf(dunesSpawn), Ride.rest) in
  let b = { a with z: a.z + 2.0, yaw: 1.0, roll: 0.2 } in
  between(a, b, 0.0) == a && between(a, b, 0.5) == { a with z: a.z + 1.0, yaw: 0.5, roll: 0.1 }
)

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
  let view = camera(m.riderCam, origin, p.yaw) in
  (match m.course with
   | OnDunes =>
     Frame.createLit(view |> Camera3D.clip(0.1, 450.0), Scene.group([Dunes.scene, rig]), lights)
       |> Frame.withClearColor(haze)
       |> Frame.withFog(Fog.linear(120.0, 420.0, haze))
   | OnStand =>
     Frame.createLit(view, Scene.group([Stand.scene, rig]), lights)
       |> Frame.withClearColor(sky)
       |> Frame.withFog(Fog.linear(60.0, 260.0, sky)))
    |> Frame.withUiTarget(speedo, gaugeView(m))

let ui = (m: Model) =>
  Ui.column([
    gaugeView(m),
    Ui.text(if m.riderCam then "rider cam (C)" else "chase cam (C)"),
    Ui.text(if m.course == OnDunes then "dunes (T)" else "stand (T)"),
  ])
    |> Ui.panel(Ui.bottomRight())

// Quest: 120 Hz, no foveation. The handlebars come in a later change.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120.0))
let settings = (m: Model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))
