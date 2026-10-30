// hoverbike — walk up to a hoverbike and ride it over a seeded dune course,
// or the flat test stand.
//
//   functor -d examples/hoverbike run native
//   functor -d examples/hoverbike test
//
// You start on foot beside the bike: WASD walks, the mouse looks. Near the
// seat, E gets on; riding, hold E to get off (below 2 m/s). W/S throttle and
// brake, A/D steer, C swaps the chase and rider cameras, R respawns (on foot,
// bike parked), T switches between the dunes and the stand. Gamepad: on foot
// the left stick walks; south button (A / cross) is E. Riding: right trigger
// throttle, left trigger brake, left stick steers, north button (Y /
// triangle) swaps cameras.
//
// With a headset (`bars.fun`): the left stick walks, the right stick snap
// turns, and grabbing a handlebar near the seat gets on. Riding from the
// calibrated eye: squeeze both grips to hold the bars, turn them to steer,
// right trigger throttle, left trigger brake; hold right B to get off, tap it
// to respawn. Left X: vignette strength; left Y: smooth or stepped turning;
// right A: seated or standing; left menu: recalibrate (hold still).
// Getting on and off fades through black (`mount.fun`); walking is `walk.fun`.
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

// The walker: a capsule on the ground, declared only while on foot. Its
// spawns are literals like the bike's, 4 m off its left side, facing it.
let walkerTag = Physics.tag("walker")
let dunesFoot = Vec3.make(104.0, 0.95, -380.0)
let standFoot = Vec3.make(609.0, 0.95, 0.0)
let footYaw = -Math.pi / 2.0
expect Vec3.x(dunesFoot) - Vec3.x(dunesSpawn) == 4.0 && Vec3.z(dunesFoot) == Vec3.z(dunesSpawn)
  && Vec3.x(standFoot) - Vec3.x(standSpawn) == 4.0 && Vec3.z(standFoot) == Vec3.z(standSpawn)
let walkerShape = Physics.capsule(Walk.halfHeight, Walk.radius)
let walkerBody =
  Physics.character(walkerTag, walkerShape, { stepHeight: 0.3, maxSlope: 45deg, snap: 0.3, offset: 0.02 })
    |> Physics.at(dunesFoot)

// Both courses are always declared, side by side; T teleports between them.
let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0),
    (if model.mode == Riding then [bike] else [bike, walkerBody]) |> List.append(Dunes.bodies) |> List.append(Stand.bodies))

type Course =
  | OnDunes
  | OnStand

type Mode =
  | OnFoot
  | Riding

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
  southHeld: bool,
  respawn: bool,
  switchCourse: bool,
  // The headset rider: grips, calibration and comfort.
  vr: Bars.Rider,
  mode: Mode,
  // Getting on or off: the fade, and what happens at its black.
  fade: Mount.Fade,
  walker: Walk.State,
  // The walker's position at the last step, for `draw` to interpolate from.
  footPrev: Physics.position,
  wish: Walk.Wish,
  // The mouse's last position, for mouse look.
  mouse: Option.t<Input.point2>,
  // A mount asked for this step: E (0), or a headset hand's grab (1 left,
  // -1 right).
  board: Option.t<float>,
  // On foot, near the seat and facing it.
  near: bool,
  // The dismount hold (`Mount.hold`), and how long right B has been held
  // (a tap respawns).
  off: float,
  tapB: float,
  // The last dismount found no room beside the bike.
  noRoom: bool,
}

let positionOf = (v: Vec3.t): Physics.position => { x: Vec3.x(v), y: Vec3.y(v), z: Vec3.z(v) }
let spawnOf = (course: Course) =>
  match course with
  | OnDunes => dunesSpawn
  | OnStand => standSpawn
let footOf = (course: Course) =>
  match course with
  | OnDunes => dunesFoot
  | OnStand => standFoot
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
  southHeld: false,
  respawn: false,
  switchCourse: false,
  vr: Bars.rider,
  mode: OnFoot,
  fade: Mount.clear,
  walker: { Walk.standing with yaw: footYaw },
  footPrev: positionOf(dunesFoot),
  wish: Walk.still,
  mouse: Option.None,
  board: Option.None,
  near: false,
  off: -1.0,
  tapB: -1.0,
  noRoom: false,
}

// Riding, and not mid-fade.
let riding = (m: Model) => m.mode == Riding && not Mount.fading(m.fade)

// ---- Input ---------------------------------------------------------------

let held = (k: Key.t, snap: Input.snapshot) => snap.heldKeys |> List.any((h) => h == k)
let pressed = (k: Key.t, snap: Input.snapshot) => snap.pressedKeys |> List.any((h) => h == k)
let level = (k: Key.t, snap: Input.snapshot) => if held(k, snap) then 1.0 else 0.0
let deadzone = (v: float) => if Math.abs(v) < 0.15 then 0.0 else v

// Keys shared by every input path: respawn and course.
let commands = (snap: Input.snapshot, m: Model) =>
  { m with respawn: m.respawn || pressed(Key.R, snap), switchCourse: m.switchCourse || pressed(Key.T, snap) }

// With a headset: the left stick walks along the look, the right stick snap
// turns, and a fresh squeeze asks to mount (`tick` checks it reaches a grip).
// Right B held gets off; tapped, it respawns, except a press begun riding
// too fast to get off (it counts as held, so letting go to slow down doesn't
// respawn). The right stick click switches course.
let sampledInput = (m: Model, snap: Input.snapshot) =>
  match snap.xr with
  | Option.Some(x) =>
    let (vr, controls) = Bars.read(m.speed < Bars.restSpeed, x, m.vr) in
    let b = vr.buttons.respawn in
    let heading = (match x.head with
      | Option.Some(p) => Bars.headingOf(p.orientation) - vr.calibration.yaw
      | Option.None => 0.0) in
    let grab = if Mount.fresh(m.vr.left, vr.left) then Option.Some(1.0)
      else if Mount.fresh(m.vr.right, vr.right) then Option.Some(-1.0) else Option.None in
    commands(snap, { m with vr: vr, controls: controls,
                            wish: { forward: deadzone(x.left.thumbstick.y), right: deadzone(x.left.thumbstick.x),
                                    heading: heading, turn: -x.right.thumbstick.x },
                            board: grab,
                            off: Mount.hold(riding(m), b && not m.vr.buttons.respawn, b, Bars.sampleDt, m.off),
                            tapB: if b && not m.vr.buttons.respawn && riding(m) && m.speed >= Mount.maxSpeed then Mount.holdTime
                                  else Mount.held(b, Bars.sampleDt, m.tapB),
                            respawn: m.respawn || (not b && m.tapB >= 0.0 && not Mount.long(m.tapB)),
                            switchCourse: m.switchCourse || (vr.buttons.course && not m.vr.buttons.course) })
  | Option.None => commands(snap, flat(m, snap))

// Keyboard and gamepad.
let flat = (m: Model, snap: Input.snapshot) =>
  let pad = (match snap.gamepad with
    | Option.Some(g) => { throttle: g.rightTrigger, brake: g.leftTrigger, steer: -deadzone(g.leftStick.x), swap: g.north,
                          walk: deadzone(g.leftStick.y), side: deadzone(g.leftStick.x), south: g.south }
    | Option.None => { throttle: 0.0, brake: 0.0, steer: 0.0, swap: false, walk: 0.0, side: 0.0, south: false }) in
  let swap = pressed(Key.C, snap) || (pad.swap && not m.swapHeld) in
  // E, or the pad's south button.
  let e = pressed(Key.E, snap) || (pad.south && not m.southHeld) in
  { m with
    wish: { Walk.still with forward: Math.clamp(-1.0, 1.0, level(Key.W, snap) - level(Key.S, snap) + pad.walk),
                            right: Math.clamp(-1.0, 1.0, level(Key.D, snap) - level(Key.A, snap) + pad.side) },
    board: if e then Option.Some(0.0) else Option.None,
    off: Mount.hold(riding(m), e, held(Key.E, snap) || pad.south, Bars.sampleDt, m.off),
    southHeld: pad.south,
    controls: { throttle: Math.max(level(Key.W, snap), pad.throttle),
                brake: Math.max(level(Key.S, snap), pad.brake),
                steer: Math.clamp(-1.0, 1.0, level(Key.A, snap) - level(Key.D, snap) + pad.steer) },
    riderCam: if swap then not m.riderCam else m.riderCam,
    swapHeld: pad.swap,
    vr: if m.vr.present then Bars.absent(m.vr) else m.vr }

// Mouse look, on foot.
let mouseMove = (m: Model, x: float, y: float) =>
  let at: Input.point2 = { x: x, y: y } in
  let walker = (match m.mouse with
    | Option.Some(p) => if m.mode == OnFoot && not Mount.fading(m.fade) then Walk.look(x - p.x, y - p.y, m.walker) else m.walker
    | Option.None => m.walker) in
  { m with walker: walker, mouse: Option.Some(at) }

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

// Back to a course's spawn: the bike at rest, the walker on foot beside it,
// both waiting for ground again.
let startOn = (course: Course, m: Model) =>
  let at = positionOf(spawnOf(course)) in
  ({ init with course: course, prev: poseOf(at, Ride.rest), last: at, footPrev: positionOf(footOf(course)),
               riderCam: m.riderCam, swapHeld: m.swapHeld, southHeld: m.southHeld, controls: m.controls, mouse: m.mouse,
               vr: Bars.dismount({ m.vr with vignette: 0.0 }) },
   Effect.batch([Physics.teleport(bikeTag, spawnOf(course)), Physics.teleport(walkerTag, footOf(course))]))

// One step, from where the bike is and what its probes saw. Until the probes
// have all seen ground the ride holds still; on the dunes, the boundary cuts
// outward thrust and pulls the bike back in. Slow, stepped turning steers by
// snaps instead: a snap turns the bike and the last pose together, so the
// drawn rig doesn't sweep between them.
// Unridden, the bike is parked: full brake, so it stays where it stopped.
let parked: Ride.Controls = { throttle: 0.0, brake: 1.0, steer: 0.0 }
let advance = (m: Model, c: Physics.position, probes: List<Ride.Probe>, dt: float): Model =>
  let ready = m.ready || Ride.sensed(probes) in
  let snapping = riding(m) && Bars.snapping(m.vr, m.speed) in
  let controls = if not riding(m) then parked else if snapping then { m.controls with steer: 0.0 } else m.controls in
  let stepped =
    if not ready then m.ride
    else
      match m.course with
      | OnDunes => Ride.step(Dunes.limit(c.x, c.z, m.ride.yaw, controls), probes, dt, collided(m, c, dt)) |> Dunes.recover(c.x, c.z, dt)
      | OnStand => Ride.step(controls, probes, dt, collided(m, c, dt)) in
  // On foot, the walker turns by the snaps instead.
  let (snap, turned) = if m.mode == Riding then Bars.turn(snapping && ready, m.vr.steer, dt, m.vr.snap) else (m.vr.snap, 0.0) in
  let ride = if turned == 0.0 then stepped else Bars.rotate(turned, stepped) in
  let distances = probes |> List.map((p) => if p.hit then p.distance else Ride.probeRange) in
  let clearance = List.sum(distances) / List.length(distances) in
  let speed = Ride.speedOf(ride) in
  let steps = m.steps + 1.0 in
  let refresh = Math.mod(steps, 6.0) == 0.0 in
  let target = Bars.discomfort((speed - m.speed) / dt, ride.yawRate, speed, ride.vy) in
  { m with ready: ready, ride: ride, prev: poseOf(c, { m.ride with yaw: m.ride.yaw + turned }), last: c,
           probes: distances, clearance: clearance, speed: speed, steps: steps,
           vr: { m.vr with snap: snap, vignette: Bars.envelope(target, dt, m.vr.vignette) },
           kmh: if refresh then Math.round(speed * 3.6) else m.kmh,
           hover: if refresh then Math.round(clearance * 10.0) / 10.0 else m.hover }

// ---- On foot, getting on and off ------------------------------------------

let down = Vec3.make(0.0, -1.0, 0.0)
let vecOf = (p: Physics.position) => Vec3.make(p.x, p.y, p.z)

// One step on foot: snap turns (headset), then walk, fall and land.
let walk = (m: Model, w: Physics.position, dt: float): (Model, Effect.t) =>
  let sensed = Physics.castExcluding(walkerTag, vecOf(w), down, Walk.senseRange).hit in
  let (snap, turned) = Bars.turn(m.vr.present, m.wish.turn, dt, m.vr.snap) in
  let (walker, v) = Walk.step(Physics.grounded(walkerTag), sensed, m.wish, dt, { m.walker with yaw: m.walker.yaw + turned }) in
  ({ m with walker: walker, footPrev: w, vr: { m.vr with snap: snap } },
   Physics.move(walkerTag, Vec3.make(v.x * dt, v.y * dt, v.z * dt)))

// The seat, and a headset hand's distance from its grip (`hand` 1 left, -1
// right), in the world.
let seatAt = (c: Physics.position, yaw: float) => rigPoint(rigOrigin(c.x, c.y, c.z), yaw, Mount.seat.x, Mount.seat.y, Mount.seat.z)
let toGrip = (m: Model, c: Physics.position, w: Physics.position, hand: float) =>
  let h = if hand > 0.0 then m.vr.left else m.vr.right in
  let g = if hand > 0.0 then Bars.leftGrip else Bars.rightGrip in
  rigPoint(vecOf(w), m.walker.yaw, h.at.x, h.at.y, h.at.z)
    |> Vec3.distance(rigPoint(rigOrigin(c.x, c.y, c.z), m.ride.yaw, g.x, g.y, g.z))

// On foot: get on if asked and eligible, else walk.
let onFoot = (m: Model, c: Physics.position, dt: float): (Model, Effect.t) =>
  let w = Physics.position(walkerTag) in
  let seat = seatAt(c, m.ride.yaw) in
  let near = Mount.near(w.x, w.z, m.walker.yaw + m.wish.heading, Vec3.x(seat), Vec3.z(seat)) in
  let mounting = (match m.board with
    | Option.Some(hand) =>
      if Mount.eligible(near, hand, not m.vr.pending, if hand == 0.0 then 0.0 else toGrip(m, c, w, hand))
      then Option.Some(hand) else Option.None
    | Option.None => Option.None) in
  match mounting with
  | Option.Some(hand) => ({ m with near: near, footPrev: w, fade: Mount.start(Mount.GetOn(hand)) }, Effect.none())
  | Option.None => walk({ m with near: near }, w, dt)

// Where a course's ground is, and whether (x, z) is on the course.
let groundOf = (course: Course) =>
  match course with
  | OnDunes => Dunes.groundTag
  | OnStand => Stand.floorTag
let onCourse = (course: Course, x: float, z: float) =>
  match course with
  | OnDunes => Math.abs(x) < Dunes.softLimit && Math.abs(z) < Dunes.softLimit
  | OnStand => Stand.inside(x, z)

// Room for the walker at a spot in the bike's frame: the course's ground
// straight below, the capsule clear of everything (props, walls, the bike) as
// it comes down onto it, and nothing between it and the bike. Answers the
// capsule centre, 3 cm over the ground.
let room = (course: Course, c: Physics.position, yaw: float) => (spot: Input.point3): Option.t<Input.point3> =>
  let p = rigPoint(vecOf(c), yaw, spot.x, 0.0, spot.z) in
  let x = Vec3.x(p) in
  let z = Vec3.z(p) in
  let ground = Physics.castExcluding(bikeTag, Vec3.make(x, c.y, z), down, 6.0) in
  if not (ground.hit && ground.tag == groundOf(course) && onCourse(course, x, z)) then Option.None
  else
    let y = ground.y + Walk.centreHeight + 0.03 in
    let swept = Physics.shapeCast(walkerShape, Vec3.make(x, y + 1.0, z), down, 1.0) in
    let out = Vec3.make(x - c.x, 0.0, z - c.z) in
    let gap = Physics.castExcluding(bikeTag, Vec3.make(c.x, ground.y + 1.2, c.z), out, Vec3.length(out)) in
    if swept.hit || gap.hit then Option.None else Option.Some({ x: x, y: y, z: z })

// Riding: holding to get off, slow enough, starts the fade if there is room.
// Releasing the grips never does (they brake, `bars.fun`).
let onBike = (m: Model, c: Physics.position): Model =>
  if not Mount.wantsOff(m.off, m.speed) then { m with noRoom: m.noRoom && m.off >= 0.0 }
  else
    match Mount.landing(room(m.course, c, m.ride.yaw)) with
    | Option.Some(p) => { m with noRoom: false, fade: Mount.start(Mount.GetOff(p.x, p.y, p.z, Mount.toward(p.x, p.z, c.x, c.z))) }
    | Option.None => { m with noRoom: true }

// At the fade's black: on the bike, its eye at the seat; or on foot at the
// landing, facing the bike.
let arrive = (m: Model): (Model, Effect.t) =>
  match m.fade.action with
  | Mount.GetOn(hand) => ({ m with mode: Riding, near: false, off: -1.0, vr: Bars.mountOn(hand, m.vr) }, Effect.none())
  | Mount.GetOff(x, y, z, yaw) =>
    ({ m with mode: OnFoot, walker: { Walk.standing with yaw: yaw, ready: true }, footPrev: { x: x, y: y, z: z },
              vr: Bars.dismount(m.vr) },
     Physics.teleport(walkerTag, Vec3.make(x, y, z)))

// Riding, releasing the grips never gets off; holding at speed waits; with no
// ground beside the bike (`functor test` has no world), it stays on.
expect (
  let riding = { init with mode: Riding } in
  let c = positionOf(dunesSpawn) in
  let released = onBike({ riding with controls: { throttle: 0.0, brake: Bars.gripLossBrake, steer: 0.0 } }, c) in
  let fast = onBike({ riding with off: 0.6, speed: 3.0 }, c) in
  let nowhere = onBike({ riding with off: 0.6, speed: 1.0 }, c) in
  not Mount.fading(released.fade) && not Mount.fading(fast.fade) && not fast.noRoom
    && not Mount.fading(nowhere.fade) && nowhere.noRoom && nowhere.mode == Riding
)
// At the black: getting on rides, the grabbing hand latched; getting off
// stands the walker at the landing, facing as asked, hands let go.
expect (
  let (on, _) = arrive({ init with fade: Mount.start(Mount.GetOn(-1.0)) }) in
  let (off, _) = arrive({ on with fade: Mount.start(Mount.GetOff(1.0, 2.0, 3.0, 0.5)) }) in
  on.mode == Riding && on.vr.mounted && on.vr.right.latched && not on.vr.left.latched
    && off.mode == OnFoot && off.walker.yaw == 0.5 && off.footPrev.x == 1.0 && off.footPrev.z == 3.0
    && not off.vr.mounted && not off.vr.right.latched
)

let tick = (m: Model, dt: float, tts) =>
  if dt <= 0.0 then (m, Effect.none())
  else if m.switchCourse then startOn(if m.course == OnDunes then OnStand else OnDunes, m)
  else if m.respawn then startOn(m.course, m)
  else
    let c = Physics.position(bikeTag) in
    let rode = advance(m, c, Ride.emitters |> List.map((e) => probe(c, m.ride.yaw, e)), dt) in
    let (fade, black) = Mount.step(dt, m.fade) in
    let stepped = { rode with fade: fade } in
    let (next, moved) =
      if black then arrive(stepped)
      else if Mount.fading(fade) then (stepped, Effect.none())
      else
        match m.mode with
        | OnFoot => onFoot(stepped, c, dt)
        | Riding => (onBike(stepped, c), Effect.none()) in
    // Waiting for ground, the ride is at rest, so this moves nothing.
    ({ next with board: Option.None },
     Effect.batch([Physics.move(bikeTag, Vec3.make(next.ride.vx * dt, next.ride.vy * dt, next.ride.vz * dt)), moved]))

// Readiness, driven without a world.
let missed = Ride.emitters |> List.map((e) => { hit: false, distance: 0.0 })
let onGround = Ride.flatProbes(Ride.hoverHeight)
let waitFor = (steps: float, probes: List<Ride.Probe>, m: Model) =>
  List.range(steps) |> List.fold((acc, i) => advance(acc, acc.last, probes, Ride.dt60), m)
// Two seconds without ground: still waiting, and the bike hasn't moved or
// begun to fall, whatever the controls say.
expect (
  let m = waitFor(120.0, missed, { init with controls: { throttle: 1.0, brake: 0.0, steer: 1.0 } }) in
  not m.ready && m.ride == Ride.rest
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

// With a headset, a third line says what the rider must do next.
let prompt = (m: Model) =>
  let r = m.vr in
  if r.pending then (if m.speed < Bars.restSpeed then "hold still" else "stop to calibrate")
  else if not (r.left.latched && r.right.latched) then "grab grips"
  else if not r.armed then "let go of throttle"
  else if r.stepped then "stepped turns"
  else "smooth turns"

// Riding, a line for getting off, or the headset's next step.
let notice = (m: Model) =>
  if m.mode != Riding then Option.None
  else if Mount.tooFast(m.off, m.speed) then Option.Some("slow down")
  else if m.noRoom then Option.Some("no room to get off")
  else if m.vr.present then Option.Some(prompt(m))
  else Option.None

let gaugeView = (m: Model) =>
  Ui.column([
    Ui.row([Ui.textColor(Bike.ion, $"{m.kmh}"), Ui.text("km/h")]),
    Ui.text($"hover {Text.fixed(m.hover, 1.0)} m"),
  ] |> List.append(match notice(m) with
    | Option.Some(text) => [Ui.textColor(Bike.tag, text)]
    | Option.None => []))

// On foot near the seat, a card over it says how to get on, turned to the
// walker.
let card = RenderTarget.named("card") |> RenderTarget.sized(128.0, 24.0)
let cardText = (m: Model) =>
  if not m.vr.present then "E to ride" else if m.vr.pending then "hold still" else "grab the bars"
let showsCard = (m: Model) => m.mode == OnFoot && m.near && not Mount.fading(m.fade)
let withCard = (m: Model, frame) =>
  if showsCard(m) then frame |> Frame.withUiTarget(card, Ui.text(cardText(m))) else frame
let cardAt = (origin: Vec3.t, yaw: float, w: Vec3.t) =>
  let at = rigPoint(origin, yaw, 0.0, 1.3, Mount.seat.z) in
  Scene.quad() |> Scene.screen(card) |> Scene.scaleXYZ(0.6, 0.12, 1.0)
    |> Scene.rotateY(Angle.radians(Mount.toward(Vec3.x(at), Vec3.z(at), Vec3.x(w), Vec3.z(w))))
    |> Scene.translate(at)

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

// The headset camera: the rig-local origin placed so the calibrated head
// lands on the eye, turned by the calibration's heading. Yaw-only, so the
// horizon stays level; tracking composes the head on top. The headset owns
// the field of view; 96°, a Quest 3 eye's vertical, frames the desktop view
// like it.
let headsetCamera = (r: Bars.Rider, origin: Vec3.t, yaw: float) =>
  let o = Bars.rigOrigin(r) in
  Camera3D.firstPerson(rigPoint(origin, yaw, o.x, o.y, o.z), Angle.radians(yaw - r.calibration.yaw), 0deg, 96deg)

// The camera maps a rig-local pose where `Bars.toBike` says it is.
expect (
  let r = { Bars.rider with calibration: { x: 0.1, y: -0.08, z: 0.2, yaw: 0.6 }, standing: true, mounted: true } in
  let origin = Vec3.make(3.0, 1.0, -2.0) in
  let p: Input.point3 = { x: 0.3, y: -0.4, z: -0.5 } in
  let mapped = Camera3D.mapTrackedPose(headsetCamera(r, origin, 0.7), Bars.pose(p)) in
  let b = Bars.toBike(r.calibration, Bars.eyeOf(r), p) in
  let w = rigPoint(origin, 0.7, b.x, b.y, b.z) in
  Math.abs(mapped.position.x - Vec3.x(w)) < 0.0001 && Math.abs(mapped.position.y - Vec3.y(w)) < 0.0001
    && Math.abs(mapped.position.z - Vec3.z(w)) < 0.0001
)

// Seat-eye placement: once on, the calibrated head (where it was, the way it
// faced) lands on the seat eye, looking along the bike's heading; on foot,
// on the walker's eye, looking along the walker's.
let lands = (r: Bars.Rider, eye: Input.point3, yaw: float) =>
  let c = r.calibration in
  let head: Input.pose = { position: { x: c.x, y: c.y, z: c.z }, orientation: Bars.yawed(c.yaw) } in
  let origin = Vec3.make(3.0, 1.0, -2.0) in
  let mapped = Camera3D.mapTrackedPose(headsetCamera(r, origin, yaw), head) in
  let w = rigPoint(origin, yaw, eye.x, eye.y, eye.z) in
  Math.abs(mapped.position.x - Vec3.x(w)) < 0.0001 && Math.abs(mapped.position.y - Vec3.y(w)) < 0.0001
    && Math.abs(mapped.position.z - Vec3.z(w)) < 0.0001
    && Math.abs(mapped.forward.x - Math.sin(yaw)) < 0.0001 && Math.abs(mapped.forward.z - Math.cos(yaw)) < 0.0001
expect (
  let foot = { Bars.rider with calibration: { x: 0.1, y: -0.08, z: 0.2, yaw: 0.6 } } in
  lands(foot, Walk.eye, -1.2) && lands(Bars.mountOn(1.0, foot), Bars.seatedEye, 0.7)
)

// Each tracked hand, where it really is: lit ion once latched. Sampled each
// fixed step, so at 120 Hz the markers move in 60 Hz steps.
let hands = (r: Bars.Rider, origin: Vec3.t, yaw: float) =>
  Scene.group([r.left, r.right] |> List.filter((h) => h.tracked) |> List.map((h) =>
    Scene.sphere() |> Scene.scale(0.035) |> Scene.emissive(if h.latched then Bike.ion else Bike.tag)
      |> Scene.translate(rigPoint(origin, yaw, h.at.x, h.at.y, h.at.z))))

// The comfort layer over the frame: the vignette, its clear radius the
// aperture, and the stepped turn's fade. Absent when both are clear.
let vignette = Asset.texture("vignette.png")
let black = Color.rgb(0.0, 0.0, 0.0)
// With a headset the layer also carries getting on and off's fade.
let withComfort = (m: Model, frame) =>
  let r = m.vr in
  let closed = Bars.closure(r) in
  let fade = Math.max(Bars.fade(r.snap), Bars.fadeAt(m.fade.t)) in
  if not r.present || (closed < 0.01 && fade <= 0.0) then frame
  else
    let size = 2.0 * Bars.aperture(closed) / Bars.vignetteInner in
    frame |> Frame.with2D(Camera2D.create(Bars.layerWidth, Bars.layerHeight),
      Sprite.group([
        if closed < 0.01 then Sprite.blank() else Sprite.image(size, size, vignette) |> Sprite.fade(Bars.opacity(closed)),
        if fade <= 0.0 then Sprite.blank() else Sprite.rectangle(black, 4.0, 4.0) |> Sprite.fade(fade),
      ]))

let lights = [
  Light.ambient(Color.rgb(0.32, 0.28, 0.4)),
  Light.directional(Vec3.make(0.3, -0.6, -0.7), Color.rgb(1.0, 0.75, 0.65), 0.9),
]

// On a desktop display the 2D layer letterboxes to its own aspect, so getting
// on and off fades behind a black veil in the world instead: a quad 20 cm
// ahead of the eye, facing it, wide enough for any view.
let veil = (level: float, eye: Vec3.t, look: Vec3.t) =>
  if level <= 0.0 then Scene.group([])
  else
    let f = Vec3.normalize(look) in
    Scene.quad() |> Scene.color(black) |> Scene.scale(2.0)
      |> Scene.rotateX(Angle.radians(-Math.asin(Vec3.y(f))))
      |> Scene.rotateY(Angle.radians(Math.atan2(Vec3.x(f), Vec3.z(f))))
      |> Scene.translate(Vec3.add(eye, Vec3.scale(0.2, f)))
      |> Scene.opacity(level)

// On foot the walker's capsule centre, `alpha` of the way from the last step.
let walkerAt = (m: Model, alpha: float) =>
  vecOf(m.footPrev) |> Vec3.lerp(vecOf(Physics.position(walkerTag)), alpha)

let draw = (m: Model, tts, alpha: float) =>
  let p = between(m.prev, poseOf(Physics.position(bikeTag), m.ride), alpha) in
  let origin = rigOrigin(p.x, p.y, p.z) in
  let headset = m.vr.present in
  let mounted = m.mode == Riding in
  let rig = Bike.draw(p.roll, p.pitch, m.probes, mounted && not m.riderCam && not headset, speedo)
    |> Scene.rotateY(Angle.radians(p.yaw)) |> Scene.translate(origin) in
  // The frame the view and the hands are in: the bike's, or the walker's.
  let (frameAt, frameYaw) = if mounted then (origin, p.yaw) else (walkerAt(m, alpha), m.walker.yaw) in
  let footEye = rigPoint(frameAt, frameYaw, Walk.eye.x, Walk.eye.y, Walk.eye.z) in
  let pitch = m.walker.pitch in
  let view =
    if headset then headsetCamera(m.vr, frameAt, frameYaw)
    else if mounted then camera(m.riderCam, origin, p.yaw)
    else Camera3D.firstPerson(footEye, Angle.radians(frameYaw), Angle.radians(pitch), 75deg) in
  // The desktop view's eye and look, for the veil.
  let (eye, look) =
    if not mounted then
      (footEye, Vec3.make(Math.sin(frameYaw) * Math.cos(pitch), Math.sin(pitch), Math.cos(frameYaw) * Math.cos(pitch)))
    else if m.riderCam then (rigPoint(origin, p.yaw, 0.0, 1.46, -0.35), Vec3.make(Math.sin(p.yaw), 0.0, Math.cos(p.yaw)))
    else
      let e = rigPoint(origin, p.yaw, 0.0, 3.0, -7.5) in
      (e, rigPoint(origin, p.yaw, 0.0, 1.2, 3.0) |> Vec3.sub(e)) in
  let extras = (if headset then [hands(m.vr, frameAt, frameYaw)] else [veil(Bars.fadeAt(m.fade.t), eye, look)])
    |> List.append(if showsCard(m) then [cardAt(origin, p.yaw, frameAt)] else []) in
  let world = (course) => Scene.group([course, rig] |> List.append(extras)) in
  (match m.course with
   | OnDunes =>
     Frame.createLit(view |> Camera3D.clip(0.1, 450.0), world(Dunes.scene), Dunes.lights)
       |> Frame.withClearColor(haze)
       |> Frame.withFog(Fog.linear(120.0, 420.0, haze))
   | OnStand =>
     Frame.createLit(view, world(Stand.scene), lights)
       |> Frame.withClearColor(sky)
       |> Frame.withFog(Fog.linear(60.0, 260.0, sky)))
    |> Frame.withUiTarget(speedo, gaugeView(m))
    |> withCard(m)
    |> withComfort(m)

let ui = (m: Model) =>
  Ui.column([
    gaugeView(m),
    Ui.text(if m.vr.present then "headset" else if m.mode == OnFoot then "on foot (E rides)"
            else if m.riderCam then "rider cam (C)" else "chase cam (C)"),
    Ui.text(if m.course == OnDunes then "dunes (T)" else "stand (T)"),
  ])
    |> Ui.panel(Ui.bottomRight())

// Quest: 120 Hz, no foveation.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120.0))
let settings = (m: Model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))
