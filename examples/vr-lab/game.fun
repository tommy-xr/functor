// examples/vr-lab — a VR interaction sandbox: a hub with one station per
// skill, each reset from a selector. Station 1 is the grab table: pick the
// ball up with either hand (squeeze), push it into the wall (it stops there),
// throw it into the basket; or take the stick, turn it into the wall, shove
// the ball with it, or throw it spinning.
//
//   functor -d examples/vr-lab run vr
//   functor -d examples/vr-lab run native        # the station, no hands
//   functor -d examples/vr-lab run native --emulate-xr --xr-eye-height 1.3
//                                                 # mouse moves the right hand, Enter squeezes
//   functor -d examples/vr-lab test              # the pure hold/throw logic
//   npm run test:vr-lab                          # injected XR scenarios, headless
//   npm run demo:vr-lab -- grab-throw out.gif    # a spectator video of one
//
// 1-9 jump to a station and reset it (B/Y or a menu button cycles stations
// on the headset). `GET /state` shows the station, both hands, the ball and
// the last release.
//
// The held ball keeps its collider: each step it sweeps from where it was
// toward the hand and stops at whatever it hits (`Hold.sweep`), and is
// declared kinematic there. Released, it turns dynamic and is given the
// velocity estimated from those accepted positions, in the same frame.
//
// The stick is a body too: held, it is swept between poses as it turns
// (`Physics.sweep`, at the poses it is drawn at), stops at the level, and is
// declared kinematic at the accepted pose (`Physics.atPose`), so it shoves
// what it sweeps through. Released, it turns dynamic and is given the
// velocity and spin estimated from its accepted poses, and is drawn where
// the world says it is (`Physics.pose`).

let config =
  Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120.0) |> Xr.floorLocked(true))

let settings = (m) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))

type ReleaseKind = | Throw | Drop | Break

/// The last time a hand let go (world space).
type Release = { kind: ReleaseKind, at: Input.point3, velocity: Input.point3 }

/// The last time a hand let go of the stick (world space; `spin` is an axis
/// × rad/s).
type StickRelease = { kind: ReleaseKind, at: Input.point3, velocity: Input.point3, spin: Input.point3 }

type Model = {
  station: float,
  xr: Option.t<Input.xr>,
  left: Hold.Hand,
  right: Hold.Hand,
  // The cycle buttons' level, for their press edge.
  cycle: bool,
  // Bumped on reset: fresh tags respawn the ball and the stick at rest.
  ballGen: float,
  // Where the free ball was declared (world). Constant while it is free, so
  // the simulation owns its pose.
  ballAt: Input.point3,
  held: Option.t<Hold.Held>,
  // In a hand, or free.
  stick: Option.t<Stick.Held>,
  // Where the free stick was declared (rig-local): its rack, or where it was
  // let go. Constant while it is free, so the simulation owns its pose.
  stickAt: Input.pose,
  // Read back from the world each step (rig-local, one step behind).
  stickPose: Input.pose,
  lastStickRelease: Option.t<StickRelease>,
  // New bodies aren't in the world before this frame's physics: don't read them.
  fresh: bool,
  // Read back from the world each step (one step behind).
  ballPos: Input.point3,
  ballVel: Input.point3,
  lastRelease: Option.t<Release>,
  score: float,
  scored: bool
}

let zero = Hold.zero
// Slower than this, a release is a drop.
let dropSpeed = 0.05
let releaseKind = (v: Input.point3): ReleaseKind => if Quat.length(v) > dropSpeed then Throw else Drop
let ballTag = (gen: float) => Physics.tag($"ball-{gen}")
let stickTag = (gen: float) => Physics.tag($"stick-{gen}")

/// Jump to a station and reset it: no holds, no history, a fresh ball, and
/// hands that must open before they grab again.
let reset = (m: Model, id: float): Model =>
  { m with
      station: id,
      left: Hold.disarm(m.left),
      right: Hold.disarm(m.right),
      ballGen: m.ballGen + 1.0,
      ballAt: Stations.ballSpawn,
      held: Option.None,
      stick: Option.None,
      stickAt: Stations.stickRest,
      stickPose: Stations.stickRest,
      lastStickRelease: Option.None,
      fresh: true,
      ballPos: Stations.ballSpawn,
      ballVel: zero,
      lastRelease: Option.None,
      score: 0.0,
      scored: false }

let init: Model =
  reset({
    station: 1.0, xr: Option.None, left: Hold.idle, right: Hold.idle, cycle: false,
    ballGen: 0.0, ballAt: Stations.ballSpawn, held: Option.None, stick: Option.None,
    stickAt: Stations.stickRest, stickPose: Stations.stickRest, lastStickRelease: Option.None, fresh: true,
    ballPos: Stations.ballSpawn, ballVel: zero, lastRelease: Option.None, score: 0.0, scored: false
  }, 1.0)

// --- input ---

let stationKeys = [
  (Key.Num1, 1.0), (Key.Num2, 2.0), (Key.Num3, 3.0), (Key.Num4, 4.0), (Key.Num5, 5.0),
  (Key.Num6, 6.0), (Key.Num7, 7.0), (Key.Num8, 8.0), (Key.Num9, 9.0),
]

let exists = (id: float) => Stations.all |> List.any((s) => s.id == id)

let cyclePressed = (xr: Option.t<Input.xr>): bool =>
  match xr with
  | Option.Some(x) =>
    x.left.secondaryPressed || x.right.secondaryPressed || x.left.menuPressed || x.right.menuPressed
  | Option.None => false

let sampledInput = (m: Model, s: Input.snapshot): Model =>
  let chosen =
    stationKeys
      |> List.find((kv) => let (k, id) = kv in exists(id) && (s.pressedKeys |> List.any((p) => p == k)))
      |> Option.map((kv) => let (_, id) = kv in id) in
  let cycle = cyclePressed(s.xr) in
  let next = { m with xr: s.xr, cycle: cycle } in
  match chosen with
  | Option.Some(id) => reset(next, id)
  | Option.None => if cycle && not m.cycle then reset(next, Stations.after(m.station)) else next

// --- the step ---

let controller = (xr: Option.t<Input.xr>, side: Hold.Side): Option.t<Input.controller> =>
  xr |> Option.map((x) => match side with | Hold.Left => x.left | Hold.Right => x.right)

/// A hand's grip, rig-local, while it is tracked.
let gripOf = (xr: Option.t<Input.xr>, side: Hold.Side): Option.t<Input.pose> =>
  match controller(xr, side) with
  | Option.Some(c) => if c.active then c.grip else Option.None
  | Option.None => Option.None

let squeezeOf = (xr: Option.t<Input.xr>, side: Hold.Side): float =>
  controller(xr, side) |> Option.map((c) => c.squeeze) |> Option.defaultValue(0.0)

let handOf = (m: Model, side: Hold.Side): Hold.Hand =>
  match side with
  | Hold.Left => m.left
  | Hold.Right => m.right

let withHand = (m: Model, side: Hold.Side, h: Hold.Hand): Model =>
  match side with
  | Hold.Left => { m with left: h }
  | Hold.Right => { m with right: h }

let held = (m: Model, st: Stations.Station, outcome: Hold.Outcome): (Model, Option.t<Input.point3>) =>
  match outcome with
  | Hold.Holding(h) => ({ m with held: Option.Some(h) }, Option.None)
  | Hold.Released(at, v) =>
    let release = { kind: releaseKind(v), at: Stations.toWorld(st, at), velocity: Stations.dirToWorld(st, v) } in
    ({ m with held: Option.None, ballAt: release.at, lastRelease: Option.Some(release) }, Option.Some(release.velocity))
  | Hold.Broke(at) =>
    let side = m.held |> Option.map((h) => h.side) |> Option.defaultValue(Hold.Right) in
    let release = { kind: Break, at: Stations.toWorld(st, at), velocity: zero } in
    (withHand({ m with held: Option.None, ballAt: release.at, lastRelease: Option.Some(release) }, side, Hold.disarm(handOf(m, side))),
     Option.Some(zero))

/// A free ball: hands hover it when in reach, and a fresh squeeze grabs it.
let free = (m: Model, st: Stations.Station, edges: (bool, bool), t: float): Model =>
  let (leftEdge, rightEdge) = edges in
  let ball = Stations.toRig(st, m.ballPos) in
  let gap = (side: Hold.Side) =>
    gripOf(m.xr, side)
      |> Option.map((g) => Quat.length(Quat.sub(g.position, ball)) - Stations.ballRadius)
      |> Option.defaultValue(1000.0) in
  let hover = (h: Hold.Hand, side: Hold.Side) => { h with hover: h.tracked && Hold.hovering(h.hover, gap(side)) } in
  let left = hover(m.left, Hold.Left) in
  let right = hover(m.right, Hold.Right) in
  let scoredNow = not m.scored && Stations.inBasket(m.ballPos) in
  let next = { m with left: left, right: right, score: if scoredNow then m.score + 1.0 else m.score, scored: m.scored || scoredNow } in
  let picked =
    Hold.pick(Hold.reacher(Hold.Left, leftEdge, left.hover, gap(Hold.Left)),
              Hold.reacher(Hold.Right, rightEdge, right.hover, gap(Hold.Right))) in
  match picked with
  | Option.Some(side) =>
    (match gripOf(m.xr, side) with
     | Option.Some(g) =>
       { next with
           held: Option.Some(Hold.grab(side, g, ball, t)),
           left: { left with hover: false },
           right: { right with hover: false },
           scored: false }
     | Option.None => next)
  | Option.None => next

/// A stick let go at `pose` (rig-local) with a velocity and spin: free from
/// there, recorded as the last stick release (world space).
let letGoStick = (m: Model, st: Stations.Station, kind: ReleaseKind, pose: Input.pose, v: Input.point3, w: Input.point3): (Model, Option.t<StickRelease>) =>
  let release = { kind: kind, at: Stations.toWorld(st, pose.position), velocity: Stations.dirToWorld(st, v), spin: Stations.dirToWorld(st, w) } in
  ({ m with stick: Option.None, stickAt: pose, lastStickRelease: Option.Some(release) }, Option.Some(release))

/// The stick: held, it follows its hand; free, a fresh squeeze in reach of
/// it takes it (the right hand first), unless that squeeze just took the
/// ball. Also the release to give the world this step, if any.
let stickStep = (m: Model, st: Stations.Station, cast: (Input.pose, Input.pose) => Option.t<float>, edges: (bool, bool), t: float): (Model, Option.t<StickRelease>) =>
  match m.stick with
  | Option.Some(s) =>
    (match Stick.step(cast, s, gripOf(m.xr, s.side), handOf(m, s.side).squeezed, t) with
     | Stick.Holding(h) => ({ m with stick: Option.Some(h) }, Option.None)
     | Stick.Released(pose, v, w) =>
       letGoStick(m, st, releaseKind(v), pose, v, w)
     | Stick.Broke(pose) =>
       letGoStick(withHand(m, s.side, Hold.disarm(handOf(m, s.side))), st, Break, pose, zero, zero))
  | Option.None =>
    let (leftEdge, rightEdge) = edges in
    let take = (side: Hold.Side, edge: bool): Option.t<Stick.Held> =>
      if not edge || (m.held |> Option.map((h) => h.side == side) |> Option.defaultValue(false)) then Option.None
      else
        gripOf(m.xr, side)
          |> Option.filter((g) => Stick.gap(g.position, m.stickPose) <= Hold.reach)
          |> Option.map((g) => Stick.grab(side, g, m.stickPose, t)) in
    match take(Hold.Right, rightEdge) with
    | Option.Some(h) => ({ m with stick: Option.Some(h) }, Option.None)
    | Option.None => ({ m with stick: take(Hold.Left, leftEdge) }, Option.None)

/// The world one fixed step sees: rig-local sweeps of the ball between two
/// points and of the stick between two poses.
type World = { ball: (Input.point3, Input.point3) => Hold.Hit, stick: (Input.pose, Input.pose) => Option.t<float> }

/// The bodies as read back from the world: the ball (world) and the stick
/// (rig-local).
type Live = { ballPos: Input.point3, ballVel: Input.point3, stickPose: Input.pose }

/// One fixed step, given the world and the bodies as read back: the next
/// model, a velocity to set on the ball, and the stick's release.
let step = (m: Model, st: Stations.Station, world: World, live: Live, t: float): (Model, Option.t<Input.point3>, Option.t<StickRelease>) =>
  let (left, leftEdge) = Hold.stepHand(m.left, Option.isSome(gripOf(m.xr, Hold.Left)), squeezeOf(m.xr, Hold.Left)) in
  let (right, rightEdge) = Hold.stepHand(m.right, Option.isSome(gripOf(m.xr, Hold.Right)), squeezeOf(m.xr, Hold.Right)) in
  let next = { m with left: left, right: right, ballPos: live.ballPos, ballVel: live.ballVel, stickPose: live.stickPose, fresh: false } in
  // (A hand holding either item is squeezing, so it has no fresh press for
  // the other until it opens and lets go.)
  let (ball, kick) =
    match m.held with
    | Option.Some(h) => held(next, st, Hold.step(world.ball, h, gripOf(m.xr, h.side), handOf(next, h.side), t))
    | Option.None => (free(next, st, (leftEdge, rightEdge), t), Option.None) in
  let (stepped, thrown) = stickStep(ball, st, world.stick, (leftEdge, rightEdge), t) in
  (stepped, kick, thrown)

/// A rig-local pose in the world, as it is drawn (`Scene.atPose`), swept and
/// declared (`Physics.atPose`).
let posed = (st: Stations.Station, p: Input.pose) => Camera3D.mapTrackedPose(Stations.rig(st), p)

/// `posed`'s inverse: a world pose (`Physics.pose`) back in the rig, where
/// the pose's forward is its -Z.
let unposed = (st: Stations.Station, p: Camera3D.mappedPose): Input.pose =>
  let forward = Stations.dirToRig(st, p.forward) in
  let up = Stations.dirToRig(st, p.up) in
  { position: Stations.toRig(st, p.position),
    orientation: Quat.fromBasis(Quat.cross(forward, up), up, Quat.scale(0.0 - 1.0, forward)) }

expect (
  let st = { Stations.grabTable with yaw: 0.7 } in
  let p = { position: Quat.v3(0.2, 1.1, -0.4), orientation: Quat.mul(Quat.yaw(2.0), Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), 0.9)) } in
  let back = unposed(st, posed(st, p)) in
  Hold.near3(back.position, p.position) && Quat.angle(back.orientation, p.orientation) < 0.0001
)

let stickShape = Physics.box(Stick.thickness, Stick.thickness, Stick.length)

/// The world behind `Hold.sweep` and `Stick.move`, swept against the fixed
/// level only: a held item is kinematic or bodiless, so it never hits itself
/// (or the other one).
let worldOf = (st: Stations.Station): World => {
  ball: (from: Input.point3, to: Input.point3) =>
    let at = (p: Input.point3) => posed(st, { position: p, orientation: Quat.identity }) in
    (match Physics.sweep(Physics.sphere(Stations.ballRadius), at(from), at(to), Physics.SweepFixed) with
     | Option.Some(h) =>
       { hit: true, distance: h.fraction * Quat.length(Quat.sub(to, from)), normal: Stations.dirToRig(st, Quat.v3(h.nx, h.ny, h.nz)) }
     | Option.None => { hit: false, distance: 0.0, normal: zero }),
  stick: (from: Input.pose, to: Input.pose) =>
    Physics.sweep(stickShape, posed(st, from), posed(st, to), Physics.SweepFixed) |> Option.map((h) => h.fraction)
}

let tick = (m: Model, dt: float, tts: float) =>
  let st = Stations.find(m.station) in
  let tag = ballTag(m.ballGen) in
  let sTag = stickTag(m.ballGen) in
  let live =
    if m.fresh then { ballPos: m.ballAt, ballVel: zero, stickPose: m.stickAt }
    else
      let p = Physics.position(tag) in
      let v = Physics.linearVelocity(tag) in
      { ballPos: Quat.v3(p.x, p.y, p.z), ballVel: Quat.v3(v.x, v.y, v.z),
        stickPose: Physics.pose(sTag) |> Option.map((p) => unposed(st, p)) |> Option.defaultValue(m.stickAt) } in
  let (next, kick, thrown) = step(m, st, worldOf(st), live, tts) in
  // Declared dynamic and given their velocities in the same frame: both land
  // on the next step, so neither spends a step at rest.
  let ballKick = kick |> Option.map((v) => [Physics.setVelocity(tag, Quat.toVec3(v))]) |> Option.defaultValue([]) in
  let stickThrow =
    thrown
      |> Option.map((r) => [Physics.setVelocity(sTag, Quat.toVec3(r.velocity)), Physics.setAngularVelocity(sTag, Quat.toVec3(r.spin))])
      |> Option.defaultValue([]) in
  (next, Effect.batch(ballKick |> List.append(stickThrow)))

let solid = (named: (string, Stations.Box)) =>
  let (name, b) = named in
  Physics.fixed(Physics.tag(name), Physics.box(b.size.x, b.size.y, b.size.z)) |> Physics.at(Quat.toVec3(b.center))

// Held: kinematic at the accepted pose. Free: dynamic, declared where it was
// let go (a constant until the next grab, so the simulation owns the pose).
let physics = (m: Model) =>
  let st = Stations.find(m.station) in
  let tag = ballTag(m.ballGen) in
  let stick =
    match m.stick with
    | Option.Some(s) => Physics.kinematic(stickTag(m.ballGen), stickShape) |> Physics.atPose(posed(st, s.accepted))
    | Option.None =>
      Physics.dynamic(stickTag(m.ballGen), stickShape)
        |> Physics.atPose(posed(st, m.stickAt))
        |> Physics.friction(0.6)
        |> Physics.restitution(0.2)
        |> Physics.linearDamping(0.1)
        |> Physics.angularDamping(0.5)
        |> Physics.continuous in
  let shape = Physics.sphere(Stations.ballRadius) in
  let ball =
    match m.held with
    | Option.Some(h) => Physics.kinematic(tag, shape) |> Physics.at(Quat.toVec3(Stations.toWorld(st, h.accepted)))
    | Option.None =>
      Physics.dynamic(tag, shape)
        |> Physics.at(Quat.toVec3(m.ballAt))
        |> Physics.restitution(0.45)
        |> Physics.friction(0.6)
        |> Physics.linearDamping(0.1)
        |> Physics.angularDamping(1.5)
        |> Physics.continuous in
  Physics.scene(Vec3.make(0.0, -9.81, 0.0), Stations.solids |> List.map(solid) |> List.append([ball, stick]))

// --- the pure step, with no world ---

let noWorld: World = {
  ball: (from: Input.point3, to: Input.point3) => { hit: false, distance: 0.0, normal: zero },
  stick: (from: Input.pose, to: Input.pose) => Option.None
}
let ballRig = Stations.toRig(Stations.grabTable, Stations.ballSpawn)

let hand = (p: Input.point3, squeeze: float): Input.controller => {
  active: true, grip: Option.Some({ position: p, orientation: Quat.identity }), aim: Option.None,
  trigger: 0.0, squeeze: squeeze, thumbstick: { x: 0.0, y: 0.0 },
  primaryPressed: false, secondaryPressed: false, thumbstickPressed: false, menuPressed: false
}
let away: Input.controller = { hand(zero, 0.0) with active: false, grip: Option.None }
let withRight = (m: Model, c: Input.controller): Model =>
  { m with xr: Option.Some({ head: Option.None, left: away, right: c }) }
let still = (m: Model): Live => { ballPos: m.ballPos, ballVel: zero, stickPose: m.stickPose }
let run = (m: Model, c: Input.controller, t: float) =>
  let (next, _, _) = step(withRight(m, c), Stations.grabTable, noWorld, still(m), t) in
  next

// Reach the ball open, squeeze: it is held.
let grabbed = run(run(init, hand(ballRig, 0.0), 0.0), hand(ballRig, 1.0), 1.0 / 60.0)
expect Option.isSome(grabbed.held) && not grabbed.right.hover
// The stick, reached by its near end: squeezed, it is held and the ball isn't.
let stickHandle = Quat.add(Stations.stickRest.position, Quat.v3(0.0, 0.03, 0.3))
let tookStick = run(run(init, hand(stickHandle, 0.0), 0.0), hand(stickHandle, 1.0), 1.0 / 60.0)
expect Option.isSome(tookStick.stick) && Option.isNone(tookStick.held)
// Opened, it is let go where it is, still: a drop.
expect (
  let opened = run(tookStick, hand(stickHandle, 0.0), 2.0 / 60.0) in
  Option.isNone(opened.stick) && opened.stickAt == Stations.stickRest
    && (opened.lastStickRelease |> Option.map((r) => r.kind == Drop) |> Option.defaultValue(false))
)
// Turned and opened, it is thrown: the release carries the wrist's spin.
expect (
  let turnedTo = (a: float, squeeze: float) =>
    { hand(stickHandle, squeeze) with grip: Option.Some({ position: stickHandle, orientation: Quat.yaw(a) }) } in
  let swung = List.range(6.0) |> List.fold((m, i) =>
    run(m, turnedTo(0.0 - 0.1 * (i + 1.0), 1.0), (i + 2.0) / 60.0), tookStick) in
  let (_, _, thrown) = step(withRight(swung, turnedTo(0.0 - 0.7, 0.0)), Stations.grabTable, noWorld, still(swung), 8.0 / 60.0) in
  match thrown with
  | Option.Some(r) => r.kind == Throw && Math.abs(r.spin.y + 6.0) < 0.001
  | Option.None => false
)
// Squeezing out of reach grabs nothing.
expect Option.isNone(run(run(init, hand(Quat.add(ballRig, Quat.v3(0.0, 0.3, 0.0)), 0.0), 0.0), hand(Quat.add(ballRig, Quat.v3(0.0, 0.3, 0.0)), 1.0), 1.0 / 60.0).held)
// Hover has hysteresis: 14 cm off the surface hovers only if it already did.
expect (
  let near = Quat.add(ballRig, Quat.v3(0.0, Stations.ballRadius + 0.1, 0.0)) in
  let mid = Quat.add(ballRig, Quat.v3(0.0, Stations.ballRadius + 0.14, 0.0)) in
  run(run(init, hand(near, 0.0), 0.0), hand(mid, 0.0), 1.0 / 60.0).right.hover
    && not run(init, hand(mid, 0.0), 0.0).right.hover
)
// A swing forward and release throws the ball forward (world +Z here).
expect (
  let swung = List.range(6.0) |> List.fold((m, i) =>
    run(m, hand(Quat.add(ballRig, Quat.v3(0.0, 0.0, -0.05 * (i + 1.0))), 1.0), (i + 2.0) / 60.0), grabbed) in
  let (thrown, kick, _) = step(withRight(swung, hand(Quat.add(ballRig, Quat.v3(0.0, 0.0, -0.35)), 0.0)), Stations.grabTable, noWorld, still(swung), 8.0 / 60.0) in
  match kick with
  | Option.Some(v) => Math.abs(v.z - 3.0) < 0.001 && Option.isNone(thrown.held)
  | Option.None => false
)
// Resetting the station drops the hold and its history, respawns the ball,
// and the still-squeezing hand cannot grab again until it opens.
expect (
  let r = reset(grabbed, 1.0) in
  Option.isNone(r.held) && r.ballGen == grabbed.ballGen + 1.0 && r.fresh && not r.right.armed
    && Option.isNone(run(r, hand(ballRig, 1.0), 2.0 / 60.0).held)
)

// --- drawing ---

let box = (b: Stations.Box, c: Color.t): Scene.t =>
  Scene.cube() |> Scene.scaleXYZ(b.size.x, b.size.y, b.size.z) |> Scene.translate(Quat.toVec3(b.center)) |> Scene.lit(c)

let pad = (at: Input.point3, size: float, c: Color.t): Scene.t =>
  Scene.cylinder() |> Scene.scaleXYZ(size, 0.01, size) |> Scene.translate(Quat.toVec3(Quat.add(at, Quat.v3(0.0, 0.005, 0.0)))) |> Scene.lit(c)

let lab =
  Scene.group([
    Scene.plane() |> Scene.scale(30.0) |> Scene.lit(Color.rgb(0.2, 0.21, 0.24)),
    pad(zero, 1.4, Color.rgb(0.3, 0.32, 0.4)),
    box(Stations.table, Color.rgb(0.55, 0.4, 0.26)),
    box(Stations.wall, Color.rgb(0.62, 0.64, 0.68)),
    Scene.group(Stations.basket |> List.map((b) => box(b, Color.rgb(0.95, 0.5, 0.15)))),
    box({ center: Quat.add(Stations.boardAt, Quat.v3(0.0, 0.0, 0.05)), size: Quat.v3(0.6, 0.45, 0.04) }, Color.rgb(0.05, 0.05, 0.07)),
  ])

let board = (score: float) =>
  Digits.number(score, Color.rgb(1.0, 0.75, 0.2), Color.rgb(0.05, 0.04, 0.03))
    |> Scene.scale(0.09)
    |> Scene.rotateY(180deg)
    |> Scene.translate(Quat.toVec3(Stations.boardAt))

let hovered = (m: Model) => m.left.hover || m.right.hover

// A plain box with a grip band at its near end. Held, drawn at the pose it is
// swept at; free, where the world has it.
let stickScene = (m: Model, st: Stations.Station) =>
  let pose =
    match m.stick with
    | Option.Some(s) => posed(st, s.accepted)
    | Option.None => Physics.pose(stickTag(m.ballGen)) |> Option.defaultValue(posed(st, m.stickAt)) in
  let wood = if Option.isSome(m.stick) then Color.rgb(0.35, 0.8, 0.75) else Color.rgb(0.25, 0.6, 0.58) in
  Scene.group([
    Scene.cube() |> Scene.scaleXYZ(Stick.thickness, Stick.thickness, Stick.length) |> Scene.lit(wood),
    Scene.cube()
      |> Scene.scaleXYZ(Stick.thickness * 1.3, Stick.thickness * 1.3, 0.12)
      |> Scene.translate(Vec3.make(0.0, 0.0, 0.07 - Stick.length / 2.0))
      |> Scene.lit(Color.rgb(0.12, 0.12, 0.14)),
  ]) |> Scene.atPose(pose)

let ballScene = (m: Model) =>
  let shape = Scene.sphere() |> Scene.scale(Stations.ballRadius) in
  let look =
    if Option.isSome(m.held) then shape |> Scene.lit(Color.rgb(1.0, 0.55, 0.25))
    else if hovered(m) then shape |> Scene.emissive(Color.rgb(1.0, 0.95, 0.45))
    else shape |> Scene.lit(Color.rgb(0.95, 0.3, 0.2)) in
  look |> Physics.transformed(ballTag(m.ballGen))

// A controller: a grip ball (where it grabs from) and a stub pointing along
// its forward, bright while squeezed.
let handScene = (st: Stations.Station, grip: Option.t<Input.pose>, h: Hold.Hand, c: Color.t) =>
  match grip with
  | Option.None => Scene.group([])
  | Option.Some(g) =>
    let paint = (s: Scene.t) => if h.squeezed then s |> Scene.emissive(c) else s |> Scene.lit(c) in
    Scene.group([
      Scene.sphere() |> Scene.scale(0.028) |> paint,
      Scene.cube() |> Scene.scaleXYZ(0.04, 0.045, 0.13) |> Scene.translate(Vec3.make(0.0, -0.015, -0.07)) |> paint,
      Scene.cube() |> Scene.scaleXYZ(0.012, 0.012, 0.06) |> Scene.translate(Vec3.make(0.0, 0.0, 0.03)) |> Scene.lit(Color.rgb(0.9, 0.9, 0.95)),
    ]) |> Scene.atPose(Camera3D.mapTrackedPose(Stations.rig(st), g))

let lights = [
  Light.ambient(Color.rgb(0.38, 0.38, 0.44)),
  Light.directional(Vec3.make(-0.35, -1.0, 0.45), Color.rgb(1.0, 0.96, 0.9), 0.85) |> Light.castShadows,
]

// The headset draws the eye at the rig plus the tracked head; without one,
// stand at eye height and look down at the station.
let camera = (m: Model, st: Stations.Station) =>
  match m.xr with
  | Option.Some(_) => Stations.rig(st)
  | Option.None =>
    Camera3D.firstPerson(Quat.toVec3(Quat.add(st.feet, Quat.v3(0.0, 1.6, 0.0))), Angle.radians(st.yaw), -24deg, 70deg)
      |> Camera3D.clip(0.05, 60.0)

let draw = (m: Model, tts) =>
  let st = Stations.find(m.station) in
  Frame.createLit(
    camera(m, st),
    Scene.group([
      lab,
      pad(st.feet, 0.8, Color.rgb(0.2, 0.6, 0.65)),
      board(m.score),
      ballScene(m),
      stickScene(m, st),
      handScene(st, gripOf(m.xr, Hold.Left), m.left, Color.rgb(0.2, 0.85, 1.0)),
      handScene(st, gripOf(m.xr, Hold.Right), m.right, Color.rgb(1.0, 0.3, 0.7)),
    ]),
    lights)
    |> Frame.withClearColor(Color.rgb(0.07, 0.08, 0.11))
