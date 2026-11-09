// examples/vr-lab — a VR interaction sandbox: a hub with one station per
// skill, each reset from a selector. Station 1 is the grab table: a ball, a
// stick, a brick and a bottle. Pick any up with either hand (squeeze), push
// it into the wall or the table (it stops there), pass it to the other hand,
// throw it. Station 2 is the inventory: the same props, and slots worn on
// the hips (`Hips`, `Slots`) to put them away in and take them back from.
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
// on the headset). H or a thumbstick click switches where a holding hand is
// drawn: where it is tracked, or where the blocked prop it holds implies.
// `GET /state` shows the station, both hands, the props and the last release.
// Each hold event buzzes its hand (`Pulses`), and a ring at the hand shows it.
//
// Every prop is one body. Held, each step it is swept from where it was
// toward where the hand holds it, turning on the way (`Physics.sweep`, at
// the pose it is drawn at), stops at whatever it hits and slides along it,
// and is declared kinematic there (`Physics.atPose`), so it shoves what it
// sweeps through. Released, it turns dynamic where it last was held and is
// given the velocity and spin estimated from its accepted poses, in the same
// frame. Free, it is drawn and grabbed where the world has it (`Physics.pose`).

let config =
  Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120.0) |> Xr.floorLocked(true))

let settings = (m) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))

type ReleaseKind = | Throw | Drop | Break

/// The last time a hand let go of a prop (world space; `spin` is an axis ×
/// rad/s).
type Release = { prop: Props.Kind, kind: ReleaseKind, at: Input.point3, velocity: Input.point3, spin: Input.point3 }

/// Where a holding hand is drawn: where it is tracked, or at the grip the
/// held prop's accepted pose implies (a hand blocked with its prop).
type HandView = | Tracked | Accepted

/// A prop: in a hand, in a body slot, or free (neither). Free, it is declared
/// at `at` (its spawn, or where it was let go): a constant until the next
/// grab, so the simulation owns its pose. Slotted, it has no body: it rides
/// the hips at its slot's pose, and shoves nothing.
type Prop = {
  kind: Props.Kind,
  held: Option.t<Hold.Held>,
  slot: Option.t<float>,
  at: Input.pose,
  // Read back from the world each step (one step behind).
  pose: Input.pose,
  velocity: Input.point3,
  // Landed in the basket since it was last taken.
  scored: bool
}

// Prop poses and velocities are rig-local.
type Model = {
  station: float,
  xr: Option.t<Input.xr>,
  left: Hold.Hand<Props.Kind>,
  right: Hold.Hand<Props.Kind>,
  // The cycle and thumbstick buttons' levels, for their press edges.
  cycle: bool,
  thumbstick: bool,
  handView: HandView,
  // Bumped on reset: fresh tags respawn the props at rest.
  gen: float,
  props: List<Prop>,
  // New bodies aren't in the world before this frame's physics: don't read them.
  fresh: bool,
  lastRelease: Option.t<Release>,
  score: float,
  // The body frame the slots ride, where they are worn.
  hips: Hips.State,
  // Each hand's haptic edges and last pulse.
  feltLeft: Pulses.Hand,
  feltRight: Pulses.Hand
}

let zero = Hold.zero
// Slower than this, a release is a drop.
let dropSpeed = 0.05
let releaseKind = (v: Input.point3): ReleaseKind => if Quat.length(v) > dropSpeed then Throw else Drop
let tagOf = (gen: float, k: Props.Kind) => Physics.tag($"{Props.name(k)}-{gen}")

let spawned = (k: Props.Kind): Prop =>
  { kind: k, held: Option.None, slot: Option.None, at: Stations.spawn(k), pose: Stations.spawn(k), velocity: zero, scored: false }

/// Jump to a station and reset it: no holds, empty slots, fresh props, and
/// hands that must open before they grab again.
let reset = (m: Model, id: float): Model =>
  { m with
      station: id,
      left: Hold.disarm(m.left),
      right: Hold.disarm(m.right),
      gen: m.gen + 1.0,
      props: Props.all |> List.map(spawned),
      fresh: true,
      lastRelease: Option.None,
      score: 0.0,
      hips: Hips.init,
      feltLeft: Pulses.idle,
      feltRight: Pulses.idle }

let init: Model =
  reset({
    station: 1.0, xr: Option.None, left: Hold.idle, right: Hold.idle, cycle: false, thumbstick: false,
    handView: Tracked, gen: 0.0, props: [], fresh: true, lastRelease: Option.None, score: 0.0, hips: Hips.init,
    feltLeft: Pulses.idle, feltRight: Pulses.idle
  }, 1.0)

// --- input ---

let stationKeys = [
  (Key.Num1, 1.0), (Key.Num2, 2.0), (Key.Num3, 3.0), (Key.Num4, 4.0), (Key.Num5, 5.0),
  (Key.Num6, 6.0), (Key.Num7, 7.0), (Key.Num8, 8.0), (Key.Num9, 9.0),
]

let exists = (id: float) => Stations.all |> List.any((s) => s.id == id)

let buttons = (xr: Option.t<Input.xr>, pressed: (Input.controller) => bool): bool =>
  xr |> Option.map((x) => pressed(x.left) || pressed(x.right)) |> Option.defaultValue(false)

let flipped = (v: HandView): HandView =>
  match v with
  | Tracked => Accepted
  | Accepted => Tracked

let sampledInput = (m: Model, s: Input.snapshot): Model =>
  let chosen =
    stationKeys
      |> List.find((kv) => let (k, id) = kv in exists(id) && (s.pressedKeys |> List.any((p) => p == k)))
      |> Option.map((kv) => let (_, id) = kv in id) in
  let cycle = buttons(s.xr, (c) => c.secondaryPressed || c.menuPressed) in
  let thumbstick = buttons(s.xr, (c) => c.thumbstickPressed) in
  let flip = (s.pressedKeys |> List.any((p) => p == Key.H)) || (thumbstick && not m.thumbstick) in
  let next = { m with xr: s.xr, cycle: cycle, thumbstick: thumbstick, handView: if flip then flipped(m.handView) else m.handView } in
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

let handOf = (m: Model, side: Hold.Side): Hold.Hand<Props.Kind> =>
  match side with
  | Hold.Left => m.left
  | Hold.Right => m.right

let withHand = (m: Model, side: Hold.Side, h: Hold.Hand<Props.Kind>): Model =>
  match side with
  | Hold.Left => { m with left: h }
  | Hold.Right => { m with right: h }

let withProp = (p: Prop, m: Model): Model =>
  { m with props: m.props |> List.map((q) => if q.kind == p.kind then p else q) }

let heldBy = (side: Hold.Side, p: Prop): bool =>
  p.held |> Option.map((h) => h.side == side) |> Option.defaultValue(false)

/// The prop a hand holds, if any.
let propIn = (m: Model, side: Hold.Side): Option.t<Prop> => m.props |> List.find((p) => heldBy(side, p))

/// What a hand holds, if anything.
let heldIn = (m: Model, side: Hold.Side): Option.t<Hold.Held> =>
  propIn(m, side) |> Option.bind((p) => p.held)

/// Where a prop is now: held, where it was accepted; free, as read back.
let poseOf = (p: Prop): Input.pose => p.held |> Option.map((h) => h.accepted) |> Option.defaultValue(p.pose)

/// A prop put away this step, and the slot it goes in.
type Stow = { prop: Props.Kind, slot: float }

/// Step every held prop with its hand: the model, and what was let go. A
/// prop let go into the slot `stowed` names goes in it instead.
let holdAll = (m: Model, st: Stations.Station, cast: (Props.Kind, Input.pose, Input.pose) => Option.t<Hold.Hit>, slots: List<Slots.Placed>, stowed: List<Stow>, t: float): (Model, List<Release>) =>
  m.props |> List.fold((acc, p) =>
    let (next, released) = acc in
    match p.held with
    | Option.None => acc
    | Option.Some(h) =>
      let letGo = (pose: Input.pose, kind: ReleaseKind, v: Input.point3, w: Input.point3) =>
        let r = { prop: p.kind, kind: kind, at: Stations.toWorld(st, pose.position), velocity: Stations.dirToWorld(st, v), spin: Stations.dirToWorld(st, w) } in
        (withProp({ p with held: Option.None, at: pose }, next), released |> List.append([r])) in
      (match Hold.step((a, b) => cast(p.kind, a, b), Props.radius(p.kind), h, gripOf(m.xr, h.side), handOf(m, h.side).squeezed, t) with
       | Hold.Holding(held) => (withProp({ p with held: Option.Some(held) }, next), released)
       | Hold.Released(pose, v, w) =>
         (match stowed |> List.find((s) => s.prop == p.kind) with
          | Option.Some(s) => (withProp({ p with held: Option.None, slot: Option.Some(s.slot), pose: slotPose(slots, s.slot, pose), at: pose }, next), released)
          | Option.None => letGo(pose, releaseKind(v), v, w))
       | Hold.Broke(pose) =>
         let (dropped, all) = letGo(pose, Break, zero, zero) in
         (withHand(dropped, h.side, Hold.disarm(handOf(dropped, h.side))), all)),
    (m, []))

/// The prop a hand that holds nothing reaches, and its gap: the nearest in
/// reach (the one it already hovers reaches a little further), held by the
/// other hand (unless it is being put away), free, or in the slot the hand
/// is in.
let reaching = (m: Model, side: Hold.Side, slots: List<Slots.Placed>, stowed: List<Stow>): Option.t<(Props.Kind, float)> =>
  let h = handOf(m, side) in
  match gripOf(m.xr, side) with
  | Option.None => Option.None
  | Option.Some(g) =>
    if Option.isSome(heldIn(m, side)) then Option.None
    else
      let inSlot = Slots.target(slots, g.position) |> Option.map((t) => t.slot.id) in
      m.props
        |> List.filter((p) => not (stowed |> List.any((s) => s.prop == p.kind))
             && (Option.isNone(p.slot) || p.slot == inSlot))
        |> List.map((p) => (p.kind, Props.gap(p.kind, g.position, poseOf(p))))
        |> List.filter((kg) => let (k, gap) = kg in Hold.hovering(h.hover == Option.Some(k), gap))
        |> List.sortBy((kg) => let (_, gap) = kg in gap)
        |> List.head

/// A hand takes a prop: from the other hand where it is, free where the
/// world has it (snapping to an authored grip in reach of it), or out of a
/// slot exactly where it sits.
let take = (m: Model, side: Hold.Side, k: Props.Kind): Model =>
  match gripOf(m.xr, side) with
  | Option.None => m
  | Option.Some(g) =>
    m.props
      |> List.find((p) => p.kind == k)
      |> Option.map((p) =>
        let held =
          (match p.held with
           | Option.Some(h) => Hold.handOff(side, g, h)
           | Option.None => Hold.grab(side, g, p.pose, if Option.isSome(p.slot) then Option.None else Props.grip(k))) in
        withHand(withProp({ p with held: Option.Some(held), slot: Option.None, scored: false }, m), side, { handOf(m, side) with hover: Option.None }))
      |> Option.defaultValue(m)

/// Hands hover what they reach, and a fresh squeeze takes it; both hands
/// pressing on one prop, the nearer (`Hold.pick`).
let grabbing = (m: Model, edges: (bool, bool), slots: List<Slots.Placed>, stowed: List<Stow>): Model =>
  let (leftEdge, rightEdge) = edges in
  let l = reaching(m, Hold.Left, slots, stowed) in
  let r = reaching(m, Hold.Right, slots, stowed) in
  let kindOf = (o: Option.t<(Props.Kind, float)>) => o |> Option.map((kg) => let (k, _) = kg in k) in
  let gapOf = (o: Option.t<(Props.Kind, float)>) => o |> Option.map((kg) => let (_, gap) = kg in gap) |> Option.defaultValue(0.0) in
  let contested = leftEdge && rightEdge && Option.isSome(l) && kindOf(l) == kindOf(r) in
  let nearer = Hold.pick(Hold.reacher(Hold.Left, true, true, gapOf(l)), Hold.reacher(Hold.Right, true, true, gapOf(r))) in
  let apply = (acc: Model, side: Hold.Side, edge: bool, o: Option.t<(Props.Kind, float)>) =>
    match kindOf(o) with
    | Option.Some(k) => if edge && (not contested || nearer == Option.Some(side)) then take(acc, side, k) else acc
    | Option.None => acc in
  let hovered = { m with left: { m.left with hover: kindOf(l) }, right: { m.right with hover: kindOf(r) } } in
  apply(apply(hovered, Hold.Right, rightEdge, r), Hold.Left, leftEdge, l)

/// Free props newly in the basket score.
let scoring = (m: Model, st: Stations.Station): Model =>
  let landed = (p: Prop) => Option.isNone(p.held) && not p.scored && Stations.inBasket(Stations.toWorld(st, p.pose.position)) in
  { m with
      score: m.score + (m.props |> List.filter(landed) |> List.length),
      props: m.props |> List.map((p) => if landed(p) then { p with scored: true } else p) }

// --- body slots ---

/// Where a prop sits in slot `id` (rig-local), or `fallback` if no such slot.
let slotPose = (slots: List<Slots.Placed>, id: float, fallback: Input.pose): Input.pose =>
  slots |> List.find((s) => s.slot.id == id) |> Option.map((s) => s.pose) |> Option.defaultValue(fallback)

let occupied = (m: Model, id: float): bool => m.props |> List.any((p) => p.slot == Option.Some(id))

/// The hips follow the head, pinned while either hand is near a slot (where
/// it was last step, so the slot holds still under the reaching hand), and
/// the slotted props ride them.
let wear = (m: Model, dt: float): Model =>
  let before = Slots.placed(m.hips) in
  let reachingFor = (side: Hold.Side) =>
    gripOf(m.xr, side) |> Option.map((g) => Slots.pins(before, g.position)) |> Option.defaultValue(false) in
  let head = m.xr |> Option.bind((x) => x.head) in
  let hips = Hips.step(m.hips, { head: head, pin: reachingFor(Hold.Left) || reachingFor(Hold.Right) }, dt) in
  let now = Slots.placed(hips) in
  { m with hips: hips, props: m.props |> List.map((p: Prop) =>
      match p.slot with
      | Option.Some(id) => { p with pose: slotPose(now, id, p.pose) }
      | Option.None => p) }

/// What the hands put away this step. A hand stores its prop by opening in a
/// slot that fits it and is empty, if it was tracked this step and the last
/// (one coming back from tracking loss already open never stores). Two hands
/// opening in one slot: the nearer its centre wins, the right on a tie
/// (`Hold.pick`); the other's prop is let go as usual.
let stowing = (prev: Model, m: Model, slots: List<Slots.Placed>): List<Stow> =>
  // A hand's claim: what it would store where, as a reacher at its gap to the slot's centre.
  let claim = (side: Hold.Side): List<(Hold.Reacher, Stow)> =>
    let h = handOf(m, side) in
    if not (handOf(prev, side).tracked && h.tracked && not h.squeezed) then []
    else
      let found =
        propIn(m, side) |> Option.bind((p) => gripOf(m.xr, side) |> Option.bind((g) =>
          Slots.target(slots, g.position)
            |> Option.filter((t) => Slots.fits(p.kind, t.slot) && not occupied(m, t.slot.id))
            |> Option.map((t) => (Hold.reacher(side, true, true, Slots.distance(t, g.position)), { prop: p.kind, slot: t.slot.id })))) in
      match found with
      | Option.Some(c) => [c]
      | Option.None => [] in
  let stows = (cs: List<(Hold.Reacher, Stow)>) => cs |> List.map((c) => let (_, s) = c in s) in
  match List.append(claim(Hold.Right), claim(Hold.Left)) with
  | [l, r] =>
    let (lr, ls) = l in
    let (rr, rs) = r in
    if ls.slot != rs.slot then [ls, rs]
    else if Hold.pick(lr, rr) == Option.Some(Hold.Left) then [ls] else [rs]
  | claims => stows(claims)

/// One fixed step, given the world (`cast`) and the props as read back
/// (`read`): the next model, and the props let go (their velocity and spin
/// to give the world).
let step = (m: Model, st: Stations.Station, cast: (Props.Kind, Input.pose, Input.pose) => Option.t<Hold.Hit>, read: (Prop) => Prop, t: float, dt: float): (Model, List<Release>) =>
  let (left, leftEdge) = Hold.stepHand(m.left, Option.isSome(gripOf(m.xr, Hold.Left)), squeezeOf(m.xr, Hold.Left)) in
  let (right, rightEdge) = Hold.stepHand(m.right, Option.isSome(gripOf(m.xr, Hold.Right)), squeezeOf(m.xr, Hold.Right)) in
  let seen = { m with left: left, right: right, props: m.props |> List.map(read), fresh: false } in
  let live = if Stations.worn(st) then wear(seen, dt) else seen in
  let slots = if Stations.worn(st) then Slots.placed(live.hips) else [] in
  // Each prop goes one place per step: into a slot first, else to the other
  // hand, else it is let go. So storing is decided first, and the other hand
  // can't take a prop being put away.
  let stowed = stowing(m, live, slots) in
  // Grabs next, so a hand taking a prop its other hand lets go of in the
  // same step catches it, and a new hold's first step seeds its history. (A
  // hand holding a prop is squeezing, so it has no fresh press for another
  // until it opens and lets go.)
  let (held, released) = holdAll(grabbing(live, (leftEdge, rightEdge), slots, stowed), st, cast, slots, stowed, t) in
  let next = scoring(held, st) in
  ({ next with lastRelease: if List.isEmpty(released) then m.lastRelease else List.last(released) }, released)

// --- haptics ---

let feltOf = (m: Model, side: Hold.Side): Pulses.Hand =>
  match side with
  | Hold.Left => m.feltLeft
  | Hold.Right => m.feltRight

let withFelt = (m: Model, side: Hold.Side, h: Pulses.Hand): Model =>
  match side with
  | Hold.Left => { m with feltLeft: h }
  | Hold.Right => { m with feltRight: h }

let other = (side: Hold.Side): Hold.Side =>
  match side with
  | Hold.Left => Hold.Right
  | Hold.Right => Hold.Left

/// The slot a hand is in, and whether it would take the hand's prop if the
/// hand opened now (`stowing`'s test).
let slotIn = (m: Model, side: Hold.Side, slots: List<Slots.Placed>): (Option.t<float>, bool) =>
  match gripOf(m.xr, side) |> Option.bind((g) => Slots.target(slots, g.position)) with
  | Option.None => (Option.None, false)
  | Option.Some(t) =>
    (Option.Some(t.slot.id),
     propIn(m, side) |> Option.map((p) => Slots.fits(p.kind, t.slot) && not occupied(m, t.slot.id)) |> Option.defaultValue(false))

/// What a hand's hold did across a step (`m` before, `next` after): it took a
/// prop (out of a slot or not), or put its prop away, lost it, or let it go.
/// A prop handed to the other hand is that hand's grab.
let holdEvents = (m: Model, next: Model, released: List<Release>, side: Hold.Side): List<Pulses.Event> =>
  let before = propIn(m, side) |> Option.map((p) => p.kind) in
  let after = propIn(next, side) |> Option.map((p) => p.kind) in
  if before == after then []
  else
    match after with
    | Option.Some(k) => if Option.isSome(propOf(m, k).slot) then [Pulses.Unstow] else [Pulses.Grab]
    | Option.None =>
      (match before with
       | Option.None => []
       | Option.Some(k) =>
         let p = propOf(next, k) in
         if Option.isSome(p.slot) then [Pulses.Stow]
         else if heldBy(other(side), p) then []
         else if released |> List.any((r) => r.prop == k && r.kind == Break) then [Pulses.Break]
         else [Pulses.Release])

/// Each hand's pulses for a step: the model with their edges, and the
/// (hand, event) pairs to buzz.
let feel = (m: Model, next: Model, released: List<Release>, slots: List<Slots.Placed>, t: float): (Model, List<(Hold.Side, Pulses.Event)>) =>
  [Hold.Left, Hold.Right] |> List.fold((acc, side) =>
    let (model, fired) = acc in
    let h = feltOf(m, side) in
    let blockedNow =
      heldIn(next, side)
        |> Option.bind((held) => gripOf(next.xr, side) |> Option.map((g) => Pulses.blocked(h.blocked, g, Hold.gripOn(held))))
        |> Option.defaultValue(false) in
    let (slot, takes) = slotIn(next, side, slots) in
    let (felt, events) = Pulses.step(h, holdEvents(m, next, released, side), blockedNow, slot, takes, t) in
    (withFelt(model, side, felt), List.append(fired, events |> List.map((e) => (side, e))))
  , (next, []))

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

/// The props' rig-local sweeps between two poses (their first touch),
/// against the fixed level only: a held prop is kinematic, so it never hits
/// itself (or another prop).
let castOf = (st: Stations.Station) =>
  (k: Props.Kind, from: Input.pose, to: Input.pose) =>
    Physics.sweep(Props.shape(k), posed(st, from), posed(st, to), Physics.SweepFixed)
      |> Option.map((h) => { fraction: h.fraction, normal: Stations.dirToRig(st, Quat.v3(h.nx, h.ny, h.nz)) })

let tick = (m: Model, dt: float, tts: float) =>
  let st = Stations.find(m.station) in
  let read = (p: Prop): Prop =>
    if m.fresh then p
    else
      match Physics.pose(tagOf(m.gen, p.kind)) with
      | Option.Some(q) =>
        let v = Physics.linearVelocity(tagOf(m.gen, p.kind)) in
        { p with pose: unposed(st, q), velocity: Stations.dirToRig(st, Quat.v3(v.x, v.y, v.z)) }
      | Option.None => p in
  let (stepped, released) = step(m, st, castOf(st), read, tts, dt) in
  let (next, pulses) = feel(m, stepped, released, if Stations.worn(st) then Slots.placed(stepped.hips) else [], tts) in
  // Declared dynamic and given their velocities in the same frame: both land
  // on the next step, so none spends a step at rest.
  let throws = released |> List.concatMap((r) =>
    [Physics.setVelocity(tagOf(m.gen, r.prop), Quat.toVec3(r.velocity)),
     Physics.setAngularVelocity(tagOf(m.gen, r.prop), Quat.toVec3(r.spin))]) in
  let buzzes = pulses |> List.map((se) => let (side, e) = se in Pulses.effect(side, e)) in
  (next, Effect.batch(List.append(throws, buzzes)))

let solid = (named: (string, Stations.Box)) =>
  let (name, b) = named in
  Physics.fixed(Physics.tag(name), Physics.box(b.size.x, b.size.y, b.size.z)) |> Physics.at(Quat.toVec3(b.center))

// Held: kinematic at the accepted pose. Free: dynamic, declared where it was
// let go (a constant until the next grab, so the simulation owns the pose).
let physics = (m: Model) =>
  let st = Stations.find(m.station) in
  let body = (p: Prop) =>
    let (tag, shape) = (tagOf(m.gen, p.kind), Props.shape(p.kind)) in
    match p.held with
    | Option.Some(h) => Physics.kinematic(tag, shape) |> Physics.atPose(posed(st, h.accepted))
    | Option.None => Physics.dynamic(tag, shape) |> Physics.atPose(posed(st, p.at)) |> Props.dynamics(p.kind) in
  Physics.scene(Vec3.make(0.0, -9.81, 0.0), Stations.solids |> List.map(solid) |> List.append(m.props |> List.filter((p) => Option.isNone(p.slot)) |> List.map(body)))

// --- the pure step, with no world ---

let noWorld = (k: Props.Kind, from: Input.pose, to: Input.pose) => Option.None
let rest = (k: Props.Kind) => (Stations.spawn(k)).position

let hand = (p: Input.point3, squeeze: float): Input.controller => {
  active: true, grip: Option.Some({ position: p, orientation: Quat.identity }), aim: Option.None,
  trigger: 0.0, squeeze: squeeze, thumbstick: { x: 0.0, y: 0.0 },
  primaryPressed: false, secondaryPressed: false, thumbstickPressed: false, menuPressed: false
}
let turnedHand = (p: Input.point3, q: Input.quaternion, squeeze: float): Input.controller =>
  { hand(p, squeeze) with grip: Option.Some({ position: p, orientation: q }) }
let away: Input.controller = { hand(zero, 0.0) with active: false, grip: Option.None }
let withHands = (m: Model, l: Input.controller, r: Input.controller): Model =>
  { m with xr: Option.Some({ head: Option.None, left: l, right: r }) }
let both = (m: Model, l: Input.controller, r: Input.controller, t: float) =>
  let (next, _) = step(withHands(m, l, r), Stations.grabTable, noWorld, (p) => p, t, 1.0 / 60.0) in
  next
let run = (m: Model, c: Input.controller, t: float) => both(m, away, c, t)
let propOf = (m: Model, k: Props.Kind): Prop =>
  m.props |> List.find((p) => p.kind == k) |> Option.defaultValue(spawned(k))
let isHeld = (m: Model, k: Props.Kind): bool => Option.isSome(propOf(m, k).held)
let releaseOf = (m: Model, k: Props.Kind, kind: ReleaseKind): bool =>
  m.lastRelease |> Option.map((r) => r.prop == k && r.kind == kind) |> Option.defaultValue(false)

// Reach the ball open, squeeze: it is held.
let grabbed = run(run(init, hand(rest(Props.Ball), 0.0), 0.0), hand(rest(Props.Ball), 1.0), 1.0 / 60.0)
expect isHeld(grabbed, Props.Ball) && grabbed.right.hover == Option.None
// The stick, reached by its near end: squeezed, it is held and the ball isn't.
let stickHandle = Quat.add(rest(Props.Stick), Quat.v3(0.0, 0.03, 0.3))
let tookStick = run(run(init, hand(stickHandle, 0.0), 0.0), hand(stickHandle, 1.0), 1.0 / 60.0)
expect isHeld(tookStick, Props.Stick) && not isHeld(tookStick, Props.Ball)
// Opened, it is let go where it is, still: a drop.
expect (
  let opened = run(tookStick, hand(stickHandle, 0.0), 2.0 / 60.0) in
  not isHeld(opened, Props.Stick) && propOf(opened, Props.Stick).at == Stations.spawn(Props.Stick) && releaseOf(opened, Props.Stick, Drop)
)
// Turned and opened, it is thrown: the release carries the wrist's spin.
expect (
  let swung = List.range(6.0) |> List.fold((m, i) =>
    run(m, turnedHand(stickHandle, Quat.yaw(0.0 - 0.1 * (i + 1.0)), 1.0), (i + 2.0) / 60.0), tookStick) in
  let (_, thrown) = step(withHands(swung, away, turnedHand(stickHandle, Quat.yaw(0.0 - 0.7), 0.0)), Stations.grabTable, noWorld, (p) => p, 8.0 / 60.0, 1.0 / 60.0) in
  match thrown with
  | [r] => r.kind == Throw && r.prop == Props.Stick && Math.abs(r.spin.y + 6.0) < 0.001
  | _ => false
)
// Squeezing out of reach grabs nothing.
expect (
  let above = Quat.add(rest(Props.Ball), Quat.v3(0.0, 0.3, 0.0)) in
  Props.all |> List.all((k) => not isHeld(run(run(init, hand(above, 0.0), 0.0), hand(above, 1.0), 1.0 / 60.0), k))
)
// Hover has hysteresis: 14 cm off the surface hovers only if it already did.
expect (
  let near = Quat.add(rest(Props.Ball), Quat.v3(0.0, Props.ballRadius + 0.1, 0.0)) in
  let mid = Quat.add(rest(Props.Ball), Quat.v3(0.0, Props.ballRadius + 0.14, 0.0)) in
  run(run(init, hand(near, 0.0), 0.0), hand(mid, 0.0), 1.0 / 60.0).right.hover == Option.Some(Props.Ball)
    && run(init, hand(mid, 0.0), 0.0).right.hover == Option.None
)
// A swing forward and release throws the ball forward (world +Z here).
expect (
  let swung = List.range(6.0) |> List.fold((m, i) =>
    run(m, hand(Quat.add(rest(Props.Ball), Quat.v3(0.0, 0.0, -0.05 * (i + 1.0))), 1.0), (i + 2.0) / 60.0), grabbed) in
  let (thrown, released) = step(withHands(swung, away, hand(Quat.add(rest(Props.Ball), Quat.v3(0.0, 0.0, -0.35)), 0.0)), Stations.grabTable, noWorld, (p) => p, 8.0 / 60.0, 1.0 / 60.0) in
  match released with
  | [r] => Math.abs(r.velocity.z - 3.0) < 0.001 && not isHeld(thrown, Props.Ball)
  | _ => false
)
// Pulses: the grab buzzes the right hand once; opening it, a release.
let feltRun = (m: Model, c: Input.controller, t: float) =>
  let (next, released) = step(withHands(m, away, c), Stations.grabTable, noWorld, (p) => p, t, 1.0 / 60.0) in
  feel(withHands(m, away, c), next, released, [], t)
expect (
  let (reached, none) = feltRun(init, hand(rest(Props.Ball), 0.0), 0.0) in
  let (took, grab) = feltRun(reached, hand(rest(Props.Ball), 1.0), 1.0 / 60.0) in
  let (_, held) = feltRun(took, hand(rest(Props.Ball), 1.0), 2.0 / 60.0) in
  let (_, release) = feltRun(took, hand(rest(Props.Ball), 0.0), 2.0 / 60.0) in
  none == [] && grab == [(Hold.Right, Pulses.Grab)] && held == [] && release == [(Hold.Right, Pulses.Release)]
)
// Pushed 5 cm into the table it is blocked: one buzz, none the next step.
expect (
  let (reached, _) = feltRun(init, hand(rest(Props.Ball), 0.0), 0.0) in
  let (took, _) = feltRun(reached, hand(rest(Props.Ball), 1.0), 1.0 / 60.0) in
  let table = (k: Props.Kind, from: Input.pose, to: Input.pose) =>
    if to.position.y < from.position.y then Option.Some({ fraction: 0.0, normal: Quat.v3(0.0, 1.0, 0.0) }) else Option.None in
  let into = withHands(took, away, hand(Quat.add(rest(Props.Ball), Quat.v3(0.0, -0.05, 0.0)), 1.0)) in
  let (pressed, r1) = step(into, Stations.grabTable, table, (p) => p, 2.0 / 60.0, 1.0 / 60.0) in
  let (felt, first) = feel(into, pressed, r1, [], 2.0 / 60.0) in
  let (still, r2) = step(felt, Stations.grabTable, table, (p) => p, 3.0 / 60.0, 1.0 / 60.0) in
  let (_, second) = feel(felt, still, r2, [], 3.0 / 60.0) in
  first == [(Hold.Right, Pulses.Blocked)] && second == []
)

// Resetting the station drops the hold, respawns the props, and the
// still-squeezing hand cannot grab again until it opens.
expect (
  let r = reset(grabbed, 1.0) in
  not isHeld(r, Props.Ball) && r.gen == grabbed.gen + 1.0 && r.fresh && not r.right.armed
    && not isHeld(run(r, hand(rest(Props.Ball), 1.0), 2.0 / 60.0), Props.Ball)
)

// The brick, resting turned, taken at a corner with the hand turned another
// way: it stays exactly where it was, and lifted, keeps that turn.
let brickCorner = Quat.add(rest(Props.Brick), Quat.v3(0.05, 0.04, 0.09))
let wrist = Quat.mul(Quat.yaw(1.1), Quat.axisAngle(Quat.v3(0.0, 0.0, 1.0), 0.6))
let tookBrick = run(run(init, turnedHand(brickCorner, wrist, 0.0), 0.0), turnedHand(brickCorner, wrist, 1.0), 1.0 / 60.0)
expect (
  let lifted = run(tookBrick, turnedHand(Quat.add(brickCorner, Quat.v3(0.0, 0.2, 0.0)), wrist, 1.0), 2.0 / 60.0) in
  let spawn = Stations.spawn(Props.Brick) in
  let accepted = (m: Model) => propOf(m, Props.Brick).held |> Option.map((h) => h.accepted) |> Option.defaultValue(Hold.origin) in
  Hold.near(accepted(tookBrick), spawn) && Hold.near(accepted(lifted), { spawn with position: Quat.add(spawn.position, Quat.v3(0.0, 0.2, 0.0)) })
)
// Handed to the left hand mid-hold: it doesn't move, the right holds nothing,
// and it follows the left.
expect (
  let lefty = Quat.add(rest(Props.Brick), Quat.v3(-0.06, 0.0, -0.05)) in
  let right = turnedHand(brickCorner, wrist, 1.0) in
  let offered = both(tookBrick, hand(lefty, 0.0), right, 2.0 / 60.0) in
  let given = both(offered, hand(lefty, 1.0), right, 3.0 / 60.0) in
  let moved = both(given, hand(Quat.add(lefty, Quat.v3(0.0, 0.1, 0.0)), 1.0), turnedHand(brickCorner, wrist, 0.0), 4.0 / 60.0) in
  let spawn = Stations.spawn(Props.Brick) in
  let h = heldIn(given, Hold.Left) in
  (h |> Option.map((a) => Hold.near(a.accepted, spawn)) |> Option.defaultValue(false)) && Option.isNone(heldIn(given, Hold.Right))
    && (heldIn(moved, Hold.Left) |> Option.map((b) => Hold.near3(b.accepted.position, Quat.add(spawn.position, Quat.v3(0.0, 0.1, 0.0)))) |> Option.defaultValue(false))
)
// Handed over in the step the right hand opens: the left catches it, and
// nothing is let go.
expect (
  let lefty = Quat.add(rest(Props.Brick), Quat.v3(-0.06, 0.0, -0.05)) in
  let offered = both(tookBrick, hand(lefty, 0.0), turnedHand(brickCorner, wrist, 1.0), 2.0 / 60.0) in
  let caught = both(offered, hand(lefty, 1.0), turnedHand(brickCorner, wrist, 0.0), 3.0 / 60.0) in
  (heldIn(caught, Hold.Left) |> Option.map((h) => Hold.near(h.accepted, Stations.spawn(Props.Brick))) |> Option.defaultValue(false))
    && Option.isNone(caught.lastRelease)
)
// The bottle taken 2 cm off its neck by a hand pointing up: it snaps, so the
// neck is in the hand. Taken by its base, it doesn't.
let fist = Quat.axisAngle(Quat.v3(1.0, 0.0, 0.0), Math.pi / 2.0)
let neck = Quat.add(rest(Props.Bottle), Quat.v3(0.02, Props.neckAt, 0.0))
let bottleBase = Quat.add(rest(Props.Bottle), Quat.v3(0.05, 0.03 - (Props.extent(Props.Bottle)).y / 2.0, 0.0))
let takeAt = (q: Input.point3) => run(run(init, turnedHand(q, fist, 0.0), 0.0), turnedHand(q, fist, 1.0), 1.0 / 60.0)
expect (
  let snapped = run(takeAt(neck), turnedHand(neck, fist, 1.0), 2.0 / 60.0) in
  propOf(snapped, Props.Bottle).held
    |> Option.map((h) => Hold.near(Hold.gripOn(h), { position: neck, orientation: fist })
      && Hold.near3(Hold.compose(h.accepted, Props.grip(Props.Bottle) |> Option.defaultValue(Hold.origin)).position, neck))
    |> Option.defaultValue(false)
)
expect (
  let based = run(takeAt(bottleBase), turnedHand(bottleBase, fist, 1.0), 2.0 / 60.0) in
  propOf(based, Props.Bottle).held |> Option.map((h) => Hold.near(h.accepted, Stations.spawn(Props.Bottle))) |> Option.defaultValue(false)
)

// --- drawing ---

/// Where a hand is drawn: tracked, or (`Accepted`, holding) at the grip its
/// prop's accepted pose implies. Steering always uses the tracked pose.
let drawnGrip = (view: HandView, grip: Input.pose, held: Option.t<Hold.Held>): Input.pose =>
  match view with
  | Tracked => grip
  | Accepted => held |> Option.map(Hold.gripOn) |> Option.defaultValue(grip)

// A held ball pushed 10 cm into a wall: drawn tracked, or at the wall.
expect (
  let blocked = { side: Hold.Right, offset: Hold.origin, accepted: Hold.p(-0.55, 1.0, -0.5), history: [] } in
  let tracked = Hold.p(-0.65, 1.0, -0.5) in
  Hold.near(drawnGrip(Tracked, tracked, Option.Some(blocked)), tracked)
    && Hold.near(drawnGrip(Accepted, tracked, Option.Some(blocked)), blocked.accepted)
    && Hold.near(drawnGrip(Accepted, tracked, Option.None), tracked)
)
// H switches it, and back; so does a thumbstick click, once per press.
expect (
  let keys = (pressed: List<Key.t>, xr: Option.t<Input.xr>): Input.snapshot =>
    { heldKeys: [], pressedKeys: pressed, releasedKeys: [],
      mouse: { x: 0.0, y: 0.0, surfaceWidth: 1.0, surfaceHeight: 1.0,
               buttons: { left: false, right: false, middle: false },
               pressed: { left: false, right: false, middle: false },
               released: { left: false, right: false, middle: false } },
      xr: xr, gamepad: Option.None, touch: Option.None } in
  let clicked = Option.Some({ head: Option.None, left: away, right: { hand(zero, 0.0) with thumbstickPressed: true } }) in
  let once = sampledInput(init, keys([Key.H], Option.None)) in
  let held = sampledInput(sampledInput(init, keys([], clicked)), keys([], clicked)) in
  init.handView == Tracked && once.handView == Accepted && sampledInput(once, keys([Key.H], Option.None)).handView == Tracked
    && held.handView == Accepted
)

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
    box(Stations.bench, Color.rgb(0.55, 0.4, 0.26)),
    Scene.group(Stations.basket |> List.map((b) => box(b, Color.rgb(0.95, 0.5, 0.15)))),
    box({ center: Quat.add(Stations.boardAt, Quat.v3(0.0, 0.0, 0.05)), size: Quat.v3(0.6, 0.45, 0.04) }, Color.rgb(0.05, 0.05, 0.07)),
  ])

let board = (score: float) =>
  Digits.number(score, Color.rgb(1.0, 0.75, 0.2), Color.rgb(0.05, 0.04, 0.03))
    |> Scene.scale(0.09)
    |> Scene.rotateY(180deg)
    |> Scene.translate(Quat.toVec3(Stations.boardAt))

// Held, drawn at the pose it is swept at; slotted, at its slot; free, where
// the world has it.
let propScene = (m: Model, st: Stations.Station, p: Prop) =>
  let look =
    if Option.isSome(p.held) then Props.Taken
    else if m.left.hover == Option.Some(p.kind) || m.right.hover == Option.Some(p.kind) then Props.Hovered
    else Props.Resting in
  let pose =
    match p.held with
    | Option.Some(h) => posed(st, h.accepted)
    | Option.None =>
      if Option.isSome(p.slot) then posed(st, p.pose)
      else Physics.pose(tagOf(m.gen, p.kind)) |> Option.defaultValue(posed(st, p.at)) in
  Props.scene(p.kind, look) |> Scene.atPose(pose)

/// What the holding hands in a slot offer it.
let offered = (m: Model, s: Slots.Placed): List<Props.Kind> =>
  [Hold.Left, Hold.Right] |> List.concatMap((side) =>
    let offer =
      gripOf(m.xr, side)
        |> Option.filter((g) => Slots.target([s], g.position) |> Option.isSome)
        |> Option.bind((g) => propIn(m, side))
        |> Option.map((p) => p.kind) in
    match offer with
    | Option.Some(k) => [k]
    | Option.None => [])

// The hips' belt, a band with a notch at the front, a spine up to the neck,
// and the head's heading, an arrow above it: what the slots follow, seen
// from outside.
let belt =
  Scene.group([
    Scene.cylinder() |> Scene.scaleXYZ(0.36, 0.03, 0.26) |> Scene.lit(Color.rgb(0.75, 0.7, 0.55)) |> Scene.opacity(0.35),
    Scene.cube() |> Scene.scaleXYZ(0.03, 0.04, 0.05) |> Scene.translate(Vec3.make(0.0, 0.0, 0.14)) |> Scene.emissive(Color.rgb(1.0, 0.85, 0.4)),
  ])
let gaze =
  Scene.group([
    Scene.cylinder() |> Scene.scaleXYZ(0.12, 0.015, 0.12) |> Scene.lit(Color.rgb(0.85, 0.85, 0.9)),
    Scene.cube() |> Scene.scaleXYZ(0.025, 0.02, 0.1) |> Scene.translate(Vec3.make(0.0, 0.0, 0.08)) |> Scene.emissive(Color.rgb(0.95, 0.95, 1.0)),
  ])

let spine = (root: Input.point3) =>
  let length = root.y / Hips.pelvisRatio - root.y in
  Scene.cylinder() |> Scene.scaleXYZ(0.05, length, 0.05) |> Scene.translate(Vec3.make(0.0, length / 2.0, 0.0))
    |> Scene.lit(Color.rgb(0.75, 0.7, 0.55)) |> Scene.opacity(0.3)

/// The slots (each lit by what is offered to it), the belt, spine and gaze,
/// where they are worn and the head is tracked.
let bodyScene = (m: Model, st: Stations.Station): Scene.t =>
  match (if Stations.worn(st) then m.hips.head else Option.None) with
  | Option.None => Scene.group([])
  | Option.Some(head) =>
    let slots = Slots.placed(m.hips) in
    let up = Quat.v3(0.0, 0.22, 0.0) in
    Scene.group([
      Scene.group(slots |> List.map((s) =>
        Slots.scene(s.slot, Slots.look(s.slot, occupied(m, s.slot.id), offered(m, s))) |> Scene.atPose(posed(st, s.pose)))),
      belt |> Scene.atPose(posed(st, Hips.frame(m.hips))),
      spine(m.hips.root) |> Scene.atPose(posed(st, { position: m.hips.root, orientation: Quat.identity })),
      gaze |> Scene.atPose(posed(st, { position: Quat.add(head.position, up), orientation: Quat.yaw(Hips.headingOr(m.hips.yaw, head.orientation)) })),
    ])

// A controller: a grip ball (where it grabs from) and a stub pointing along
// its forward, bright while squeezed.
let handModel = (squeezed: bool, c: Color.t) =>
  let paint = (s: Scene.t) => if squeezed then s |> Scene.emissive(c) else s |> Scene.lit(c) in
  Scene.group([
    Scene.sphere() |> Scene.scale(0.028) |> paint,
    Scene.cube() |> Scene.scaleXYZ(0.04, 0.045, 0.13) |> Scene.translate(Vec3.make(0.0, -0.015, -0.07)) |> paint,
    Scene.cube() |> Scene.scaleXYZ(0.012, 0.012, 0.06) |> Scene.translate(Vec3.make(0.0, 0.0, 0.03)) |> Scene.lit(Color.rgb(0.9, 0.9, 0.95)),
  ])

/// A hand where `drawnGrip` puts it, and (drawn elsewhere by over
/// `Hold.ghostDistance`) a translucent ghost where it really is.
let handScene = (m: Model, st: Stations.Station, side: Hold.Side, c: Color.t): (Scene.t, Scene.t) =>
  match gripOf(m.xr, side) with
  | Option.None => (Scene.group([]), Scene.group([]))
  | Option.Some(g) =>
    let h = handOf(m, side) in
    let drawn = drawnGrip(m.handView, g, heldIn(m, side)) in
    (handModel(h.squeezed, c) |> Scene.atPose(posed(st, drawn)),
     if Hold.ghosted(g, drawn) then handModel(false, c) |> Scene.atPose(posed(st, g)) |> Scene.opacity(0.4) else Scene.group([]))

/// Each hand's last pulse, made visible where the hand is tracked.
let cues = (m: Model, st: Stations.Station, t: float): Scene.t =>
  Scene.group([Hold.Left, Hold.Right] |> List.concatMap((side) =>
    let shown =
      gripOf(m.xr, side) |> Option.bind((g) => Pulses.cue(feltOf(m, side), t)
        |> Option.map((s) => s |> Scene.atPose(posed(st, { position: g.position, orientation: Quat.identity })))) in
    match shown with
    | Option.Some(s) => [s]
    | Option.None => []))

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

// Children 3 (the props, in `Props.all` order) to 7 are what the e2e checks
// read from `/scene`: the left and right hands, then their ghosts. 8 is
// what the body wears; 9 the haptic cues.
let draw = (m: Model, tts) =>
  let st = Stations.find(m.station) in
  let (left, leftGhost) = handScene(m, st, Hold.Left, Color.rgb(0.2, 0.85, 1.0)) in
  let (right, rightGhost) = handScene(m, st, Hold.Right, Color.rgb(1.0, 0.3, 0.7)) in
  Frame.createLit(
    camera(m, st),
    Scene.group([
      lab,
      pad(st.feet, 0.8, Color.rgb(0.2, 0.6, 0.65)),
      board(m.score),
      Scene.group(m.props |> List.map((p) => propScene(m, st, p))),
      left,
      right,
      leftGhost,
      rightGhost,
      bodyScene(m, st),
      cues(m, st, tts),
    ]),
    lights)
    |> Frame.withClearColor(Color.rgb(0.07, 0.08, 0.11))
