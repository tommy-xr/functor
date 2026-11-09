// examples/vr-lab — a VR interaction sandbox: a hub with one station per
// skill, each reset from a selector. Station 1 is the grab table: a ball, a
// stick, a brick and a bottle. Pick any up with either hand (squeeze), push
// it into the wall or the table (it stops there), pass it to the other hand,
// throw it. Station 2 is the inventory: the same props, and slots worn on
// the hips (`Hips`, `Slots`) to put them away in and take them back from.
// Station 3 is the armory: a pistol (`Gun`) on a bench, fired at targets down
// range, and holstered at the thigh; and a rifle (`Longarm`) in a rack beside
// it, aimed with a second hand on its foregrip, semi or full-auto (A/X while
// holding it, or F). Both are loaded (12+1, 30+1; `Ammo`) with a readout on
// each; the thumbstick pushed down on a gun's hand (or M) drops its magazine,
// a prop like any other, and spare magazines stand on the bench.
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
  // The rifle: a second hand on its foregrip, aiming it; or (`fore`) held by
  // the foregrip alone, its trigger out of reach.
  support: Option.t<Longarm.Support>,
  fore: bool,
  slot: Option.t<float>,
  at: Input.pose,
  // Read back from the world each step (one step behind).
  pose: Input.pose,
  velocity: Input.point3,
  // Landed in the basket since it was last taken.
  scored: bool,
  // A magazine's rounds (none for the other props).
  mag: Option.t<Ammo.Mag>
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
  // Each hand's haptic edges and last pulse, and this step's pulses.
  feltLeft: Pulses.Hand,
  feltRight: Pulses.Hand,
  pulses: List<(Hold.Side, Pulses.Event)>,
  // The pistol's and the rifle's trigger, mode, kick, shots and hits.
  gun: Gun.State,
  rifle: Gun.State,
  // Each hand's A/X level (left, right), for its press edges.
  buttonA: (bool, bool),
  // Each hand's thumbstick held down (the magazine release), for its edges.
  stick: (bool, bool),
  // The next ejected magazine's id.
  nextMag: float
}

let zero = Hold.zero
// Slower than this, a release is a drop.
let dropSpeed = 0.05
let releaseKind = (v: Input.point3): ReleaseKind => if Quat.length(v) > dropSpeed then Throw else Drop
let tagOf = (gen: float, k: Props.Kind) => Physics.tag($"{Props.name(k)}-{gen}")

// The rifle starts in its rack, the spare magazines in their cradles.
let spawned = (k: Props.Kind): Prop =>
  { kind: k, held: Option.None, support: Option.None, fore: false, slot: Stations.startSlot(k),
    at: Stations.spawn(k), pose: Stations.spawn(k), velocity: zero, scored: false,
    mag: match k with
      | Props.Mag(fits, _) => Option.Some(Ammo.full(fits))
      | _ => Option.None }

/// Jump to a station and reset it: no holds, empty slots, fresh props, the
/// guns loaded and the spares full (until magazines go back in, the reload),
/// and hands that must open before they grab again.
let reset = (m: Model, id: float): Model =>
  { m with
      station: id,
      left: Hold.disarm(m.left),
      right: Hold.disarm(m.right),
      gen: m.gen + 1.0,
      props: Stations.props(Stations.find(id)) |> List.map(spawned),
      fresh: true,
      lastRelease: Option.None,
      score: 0.0,
      hips: Hips.init,
      feltLeft: Pulses.idle,
      feltRight: Pulses.idle,
      pulses: [],
      gun: Gun.stocked(Gun.pistol),
      rifle: Gun.stocked(Longarm.weapon),
      nextMag: List.length(Stations.spares) + 1.0 }

let init: Model =
  reset({
    station: 1.0, xr: Option.None, left: Hold.idle, right: Hold.idle, cycle: false, thumbstick: false,
    handView: Tracked, gen: 0.0, props: [], fresh: true, lastRelease: Option.None, score: 0.0, hips: Hips.init,
    feltLeft: Pulses.idle, feltRight: Pulses.idle, pulses: [], gun: Gun.idle, rifle: Gun.idle, buttonA: (false, false),
    stick: (false, false), nextMag: 1.0
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
  // The selector: a fresh A/X press on the hand at the rifle's trigger, or F.
  let a = (side: Hold.Side) => controller(s.xr, side) |> Option.map((c) => c.primaryPressed) |> Option.defaultValue(false) in
  let (wasLeft, wasRight) = m.buttonA in
  let pressedOn = (side: Hold.Side, was: bool) => a(side) && not was && triggerHand(m, Props.Rifle) == Option.Some(side) in
  let select = (s.pressedKeys |> List.any((p) => p == Key.F)) || pressedOn(Hold.Left, wasLeft) || pressedOn(Hold.Right, wasRight) in
  // The magazine release: a fresh thumbstick push down on the hand at a
  // gun's trigger, or M for every gun held.
  let (stickLeft, stickRight) = m.stick in
  let down = (side: Hold.Side, was: bool) => controller(s.xr, side) |> Option.map((c) => stickDown(was, c.thumbstick.y)) |> Option.defaultValue(false) in
  let (downLeft, downRight) = (down(Hold.Left, stickLeft), down(Hold.Right, stickRight)) in
  let released = (k: Props.Kind) =>
    match triggerHand(m, k) with
    | Option.Some(side) =>
      (s.pressedKeys |> List.any((p) => p == Key.M))
        || (match side with | Hold.Left => downLeft && not stickLeft | Hold.Right => downRight && not stickRight)
    | Option.None => false in
  let next = [Props.Pistol, Props.Rifle] |> List.fold((acc, k) => if released(k) then ejectMag(acc, k) else acc,
    { m with xr: s.xr, cycle: cycle, thumbstick: thumbstick, handView: if flip then flipped(m.handView) else m.handView,
             buttonA: (a(Hold.Left), a(Hold.Right)), rifle: if select then Gun.select(Longarm.weapon, m.rifle) else m.rifle,
             stick: (downLeft, downRight) }) in
  match chosen with
  | Option.Some(id) => reset(next, id)
  | Option.None => if cycle && not m.cycle then reset(next, Stations.after(m.station)) else next

/// A thumbstick held down: pushed to -0.7, let back past -0.3.
let stickDown = (was: bool, y: float): bool => if was then y < 0.0 - 0.3 else y <= 0.0 - 0.7

expect stickDown(false, -0.7) && not stickDown(false, -0.6) && stickDown(true, -0.4) && not stickDown(true, -0.3)

/// A gun's magazine dropped out: a new magazine prop with its rounds, free
/// below the well (`Gun.dropOf`), the chambered round left in the gun. A gun
/// with none, unchanged.
let ejectMag = (m: Model, k: Props.Kind): Model =>
  let g = gunOf(m, k) in
  let (ammo, out) = Ammo.eject(g.ammo) in
  match out with
  | Option.None => m
  | Option.Some(mag) =>
    let at = Hold.compose(poseOf(propOf(m, k)), Gun.dropOf(weaponOf(k))) in
    let dropped = { spawned(Props.Mag(mag.kind, m.nextMag)) with at: at, pose: at, mag: Option.Some(mag) } in
    { withGun(m, k, { g with ammo: ammo }) with props: m.props |> List.append([dropped]), nextMag: m.nextMag + 1.0 }

/// The station's unspent rounds: in every magazine (free, held, slotted or
/// in a well) and chamber. With the shots fired (`Gun.State.fired`), always
/// what the station was reset with.
let unspent = (m: Model): float =>
  (m.props |> List.map((p) => Ammo.roundsIn(p.mag)) |> List.sum) + Ammo.count(m.gun.ammo) + Ammo.count(m.rifle.ammo)

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

let triggerOf = (xr: Option.t<Input.xr>, side: Hold.Side): float =>
  controller(xr, side) |> Option.map((c) => c.trigger) |> Option.defaultValue(0.0)

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

/// A prop by kind (spawned at rest if this station has none).
let propOf = (m: Model, k: Props.Kind): Prop =>
  m.props |> List.find((p) => p.kind == k) |> Option.defaultValue(spawned(k))

/// What a hand holds, if anything.
let heldIn = (m: Model, side: Hold.Side): Option.t<Hold.Held> =>
  propIn(m, side) |> Option.bind((p) => p.held)

/// Where a prop is now: held, where it was accepted; free, as read back.
let poseOf = (p: Prop): Input.pose => p.held |> Option.map((h) => h.accepted) |> Option.defaultValue(p.pose)

let supportOf = (p: Prop): Option.t<Hold.Side> => p.support |> Option.map((s) => s.side)

/// What a hand is on: the prop it holds, or the rifle whose foregrip it supports.
let inHand = (m: Model, side: Hold.Side): Option.t<Props.Kind> =>
  match propIn(m, side) with
  | Option.Some(p) => Option.Some(p.kind)
  | Option.None => m.props |> List.find((p) => supportOf(p) == Option.Some(side)) |> Option.map((p) => p.kind)

/// The hand on a weapon's trigger: its holder, unless that holds the rifle by
/// the foregrip.
let triggerHand = (m: Model, k: Props.Kind): Option.t<Hold.Side> =>
  let p = propOf(m, k) in
  if p.fore then Option.None else p.held |> Option.map((h) => h.side)

/// The grip a hand steers its prop by: where it is tracked, or (a supported
/// rifle's primary hand) turned to the two-hand aim.
let steeredGrip = (m: Model, side: Hold.Side): Option.t<Input.pose> =>
  gripOf(m.xr, side) |> Option.map((g) =>
    propIn(m, side) |> Option.bind((p) => p.support) |> Option.map((s) => Longarm.steer(g, s)) |> Option.defaultValue(g))

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
        (withProp({ p with held: Option.None, support: Option.None, fore: false, at: pose }, next), released |> List.append([r])) in
      // A supported rifle's primary hand opening hands it to the foregrip
      // hand, where it is.
      let handedOn = p.support |> Option.bind((s) => gripOf(m.xr, s.side) |> Option.map((g) => Hold.handOff(s.side, g, h))) in
      (match Hold.step((a, b) => cast(p.kind, a, b), Props.radius(p.kind), h, steeredGrip(m, h.side), handOf(m, h.side).squeezed, t) with
       | Hold.Holding(held) => (withProp({ p with held: Option.Some(held) }, next), released)
       | Hold.Released(pose, v, w) =>
         (match stowed |> List.find((s) => s.prop == p.kind) with
          | Option.Some(s) => (withProp({ p with held: Option.None, support: Option.None, fore: false, slot: Option.Some(s.slot), pose: slotPose(slots, s.slot, pose), at: pose }, next), released)
          | Option.None =>
            (match handedOn with
             | Option.Some(fore) => (withProp({ p with held: Option.Some(fore), support: Option.None, fore: true }, next), released)
             | Option.None => letGo(pose, releaseKind(v), v, w)))
       | Hold.Broke(pose) =>
         let (dropped, all) = letGo(pose, Break, zero, zero) in
         (withHand(dropped, h.side, Hold.disarm(handOf(dropped, h.side))), all)),
    (m, []))

/// The prop a hand that holds nothing reaches, and its gap: the nearest in
/// reach (the one it already hovers reaches a little further), held by the
/// other hand (unless it is being put away), free, or in the slot the hand
/// is in. A rifle the other hand holds by its pistol grip is reached only at
/// its foregrip; one in two hands, not at all.
let reaching = (m: Model, side: Hold.Side, slots: List<Slots.Placed>, stowed: List<Stow>): Option.t<(Props.Kind, float)> =>
  let h = handOf(m, side) in
  match gripOf(m.xr, side) with
  | Option.None => Option.None
  | Option.Some(g) =>
    if Option.isSome(inHand(m, side)) then Option.None
    else
      let inSlot = Slots.target(slots, g.position) |> Option.map((t) => t.slot.id) in
      m.props
        |> List.filter((p) => not (stowed |> List.any((s) => s.prop == p.kind))
             && (Option.isNone(p.slot) || p.slot == inSlot) && Option.isNone(p.support)
             // A foregrip the primary hand's axis would break off at once isn't taken.
             && (not foregripOnly(p) || (p.held |> Option.bind((h) => gripOf(m.xr, h.side)) |> Option.map((pg) => Longarm.holds(Longarm.alongOf(pg, g.position))) |> Option.defaultValue(false))))
        |> List.map((p) =>
             if foregripOnly(p) then (p.kind, Longarm.foreGap(poseOf(p), g.position), Longarm.foreZone)
             else (p.kind, Props.gap(p.kind, g.position, poseOf(p)), Props.reach(p.kind)))
        // A required grip's zone is exact: no hysteresis past it.
        |> List.filter((kgr) => let (k, gap, range) = kgr in Hold.hovering(range, h.hover == Option.Some(k) && not Props.gripRequired(k), gap))
        |> List.map((kgr) => let (k, gap, _) = kgr in (k, gap))
        |> List.sortBy((kg) => let (_, gap) = kg in gap)
        |> List.head

/// A rifle held by its pistol grip: the other hand can only take its foregrip.
let foregripOnly = (p: Prop): bool => Option.isSome(p.held) && not p.fore && p.kind == Props.Rifle

/// A hand takes a prop: from the other hand where it is, free where the
/// world has it (snapping to an authored grip in reach of it), or out of a
/// slot exactly where it sits. A prop whose grip is required (the guns) is
/// taken where it is however it is taken, then settles into its grip. A
/// rifle the other hand holds by its pistol grip is taken by the foregrip, as
/// its support; one held by the foregrip, by the pistol grip, its holder
/// staying on as the support.
let take = (m: Model, side: Hold.Side, k: Props.Kind): Model =>
  match gripOf(m.xr, side) with
  | Option.None => m
  | Option.Some(g) =>
    let taking = (p: Prop) => withHand(withProp(p, m), side, { handOf(m, side) with hover: Option.None }) in
    m.props
      |> List.find((p) => p.kind == k)
      |> Option.bind((p) =>
        if foregripOnly(p) then
          p.held |> Option.bind((h) => gripOf(m.xr, h.side)) |> Option.map((primary) =>
            taking({ p with support: Option.Some(Longarm.support(side, primary, g.position)) }))
        else Option.Some(
        // Retaken, the foregrip hand stays on as the support if the new primary's axis holds it.
        let stays = if p.fore then p.held |> Option.bind((h) => gripOf(m.xr, h.side) |> Option.filter((q) => Longarm.holds(Longarm.alongOf(g, q.position)))
          |> Option.map((q) => Longarm.support(h.side, g, q.position))) else Option.None in
        let required = if Props.gripRequired(k) then Props.grip(k) else Option.None in
        let taken =
          (match p.held with
           | Option.Some(h) => Hold.handOff(side, g, h)
           | Option.None => Hold.grab(side, g, p.pose, if Option.isSome(p.slot) || Option.isSome(required) then Option.None else Props.grip(k))) in
        let held =
          (match required with
           | Option.Some(a) => Hold.settleInto(a, taken)
           | Option.None => taken) in
        taking({ p with held: Option.Some(held), support: stays, fore: false, slot: Option.None, scored: false })))
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
  let landed = (p: Prop) => Option.isNone(p.held) && Option.isNone(p.slot) && not p.scored && Stations.inBasket(Stations.toWorld(st, p.pose.position)) in
  { m with
      score: m.score + (m.props |> List.filter(landed) |> List.length),
      props: m.props |> List.map((p) => if landed(p) then { p with scored: true } else p) }

// --- body slots ---

/// Where a prop sits in slot `id` (rig-local), or `fallback` if no such slot.
let slotPose = (slots: List<Slots.Placed>, id: float, fallback: Input.pose): Input.pose =>
  slots |> List.find((s) => s.slot.id == id) |> Option.map((s) => s.pose) |> Option.defaultValue(fallback)

/// The rifle rack and the magazine cradles, where the armory has them.
let racks = (st: Stations.Station): List<Slots.Placed> =>
  if st.id == Stations.armory.id then [{ slot: Slots.rack, pose: Stations.rifleRack }, ..Stations.cradles] else []

let occupied = (m: Model, id: float): bool => m.props |> List.any((p) => p.slot == Option.Some(id))

/// Whether slot `s` takes a prop of kind `k` now: it fits, and is empty.
let opens = (m: Model, k: Props.Kind, s: Slots.Placed): bool => Slots.fits(k, s.slot) && not occupied(m, s.slot.id)

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
/// slot (or the rifle rack) that fits it and is empty, if it was tracked this step and the last
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
        // A rifle still in its other hand isn't put away: it is handed on.
        propIn(m, side) |> Option.filter((p) => p.support |> Option.map((s) => not handOf(m, s.side).squeezed) |> Option.defaultValue(true))
          |> Option.bind((p) => gripOf(m.xr, side) |> Option.bind((g) =>
          Slots.target(slots, g.position)
            |> Option.filter((t) => opens(m, p.kind, t))
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

/// The world as the step reaches into it, rig-local: a prop's sweep between
/// two poses (its first touch), what a shot from a weapon at a point along a
/// direction strikes within `Gun.range` (past the weapon's own body), and
/// whether a segment touches the level (the muzzle check).
type World = {
  sweep: (Props.Kind, Input.pose, Input.pose) => Option.t<Hold.Hit>,
  ray: (Props.Kind, Input.point3, Input.point3) => Option.t<Gun.Impact>,
  blocked: (Input.point3, Input.point3) => bool
}

// --- the guns ---

let gunOf = (m: Model, k: Props.Kind): Gun.State => if k == Props.Rifle then m.rifle else m.gun
let withGun = (m: Model, k: Props.Kind, g: Gun.State): Model => if k == Props.Rifle then { m with rifle: g } else { m with gun: g }
let weaponOf = (k: Props.Kind): Gun.Weapon => Props.weapon(k) |> Option.defaultValue(Gun.pistol)

/// A gun's step: its trigger, read from the hand on it (`triggerHand`), and
/// the shot a pull fires: from the muzzle of the pose the hold accepted
/// (never the tracked hand's), along its barrel, unless nothing is
/// chambered (a dry click) or the muzzle check finds the level there (a
/// click). A target hit scores. The model, and the pulses: a click in the
/// trigger hand, a shot in every hand on the gun. A station without the gun
/// skips it.
let shooting = (m: Model, world: World, k: Props.Kind, t: float, dt: float): (Model, List<(Hold.Side, Pulses.Event)>) =>
  match m.props |> List.find((q) => q.kind == k) with
  | Option.None => (m, [])
  | Option.Some(p) =>
  let holder = triggerHand(m, k) in
  let tracked = holder |> Option.map((s) => Option.isSome(gripOf(m.xr, s))) |> Option.defaultValue(false) in
  let value = holder |> Option.map((s) => triggerOf(m.xr, s)) |> Option.defaultValue(0.0) in
  let (g, pulled) = Gun.step(gunOf(m, k), holder, tracked, value, dt) in
  match (if pulled then p.held else Option.None) with
  | Option.None => (withGun(m, k, g), [])
  | Option.Some(h) =>
    let b = Gun.boreOf(weaponOf(k), h.accepted) in
    let (from, to) = Gun.checked(b) in
    if not Gun.canFire(g) then (withGun(m, k, Gun.dryPull(g, t)), [(h.side, Pulses.Click)])
    else if not world.blocked(from, to) then
      let impact = world.ray(k, b.muzzle, b.dir) in
      let onTarget = impact |> Option.map((i) => i.target) |> Option.defaultValue(false) in
      let hands = [h.side] |> List.append(supportOf(p) |> Option.map((s) => [s]) |> Option.defaultValue([])) in
      ({ withGun(m, k, Gun.fire(g, { t: t, from: b.muzzle, dir: b.dir, impact: impact })) with score: if onTarget then m.score + 1.0 else m.score },
       hands |> List.map((s) => (s, Pulses.Fire)))
    else (withGun(m, k, Gun.refuse(g)), [(h.side, Pulses.Click)])

/// Each rifle's support hand this step, before the holds: still on the
/// foregrip, aiming it, or let go or broken off (too far off the primary
/// hand's axis, or past the foregrip's ends), back to one hand. The model,
/// the props now in one hand (to settle back into the grip once this step
/// has held them where the aim had them), and the support hands' pulses.
/// While the primary hand is lost the rifle holds still and the support can
/// only let go (opened or lost), leaving the hold as it was.
let supporting = (m: Model): (Model, List<Props.Kind>, List<(Hold.Side, Pulses.Event)>) =>
  m.props |> List.fold((acc, p: Prop) =>
    let (next, alone, felt) = acc in
    match p.support with
    | Option.None => acc
    | Option.Some(s) =>
      p.held |> Option.map((h) =>
      let primary = gripOf(m.xr, h.side) in
      let at = gripOf(m.xr, s.side) |> Option.map((q) => q.position) in
      let squeezed = handOf(m, s.side).squeezed in
      let single = { p with held: Option.Some(primary |> Option.map((g) => Longarm.alone(h, s, g)) |> Option.defaultValue(h)), support: Option.None } in
      let grasp =
        match primary with
        | Option.Some(g) => Longarm.grasp(s, g, at, squeezed)
        | Option.None => if Option.isSome(at) && squeezed then Longarm.Supporting(s) else Longarm.LetGo in
      (match grasp with
       | Longarm.Supporting(s2) => (withProp({ p with support: Option.Some(s2) }, next), alone, felt)
       | Longarm.LetGo => (withProp(single, next), alone |> List.append([p.kind]), felt |> List.append([(s.side, Pulses.Release)]))
       | Longarm.Broke =>
         (withHand(withProp(single, next), s.side, Hold.disarm(handOf(next, s.side))), alone |> List.append([p.kind]), felt |> List.append([(s.side, Pulses.Break)]))))
        |> Option.defaultValue(acc),
    (m, [], []))

/// One fixed step, given the world and the props as read back (`read`): the
/// next model (with the step's `pulses`), and the props let go (their
/// velocity and spin to give the world).
let step = (m: Model, st: Stations.Station, world: World, read: (Prop) => Prop, t: float, dt: float): (Model, List<Release>) =>
  let (left, leftEdge) = Hold.stepHand(m.left, Option.isSome(gripOf(m.xr, Hold.Left)), squeezeOf(m.xr, Hold.Left)) in
  let (right, rightEdge) = Hold.stepHand(m.right, Option.isSome(gripOf(m.xr, Hold.Right)), squeezeOf(m.xr, Hold.Right)) in
  let seen = { m with left: left, right: right, props: m.props |> List.map(read), fresh: false } in
  let live = if Stations.worn(st) then wear(seen, dt) else seen in
  let slots = (if Stations.worn(st) then Slots.placed(live.hips) else []) |> List.append(racks(st)) in
  // The rifles' support hands first: they aim (or let go of) what the holds
  // then move, and a rifle one has let go of can be put away.
  let (supported, alone, steadied) = supporting(live) in
  // Each prop goes one place per step: into a slot first, else to the other
  // hand, else it is let go. So storing is decided next, and the other hand
  // can't take a prop being put away.
  let stowed = stowing(m, supported, slots) in
  // Grabs next, so a hand taking a prop its other hand lets go of in the
  // same step catches it, and a new hold's first step seeds its history. (A
  // hand holding a prop is squeezing, so it has no fresh press for another
  // until it opens and lets go.)
  let (moved, released) = holdAll(grabbing(supported, (leftEdge, rightEdge), slots, stowed), st, world.sweep, slots, stowed, t) in
  let held = { moved with props: moved.props |> List.map((p) =>
    if alone |> List.any((k) => k == p.kind) then { p with held: p.held |> Option.map((h) => Hold.settleInto(Longarm.grip, h)) } else p) } in
  // Then the guns, from the poses their holds just accepted.
  let (pistolShot, pistolFired) = shooting(scoring(held, st), world, Props.Pistol, t, dt) in
  let (shot, rifleFired) = shooting(pistolShot, world, Props.Rifle, t, dt) in
  let (next, pulses) = feel(m, shot, released, slots, List.flatten([pistolFired, rifleFired, steadied]), t) in
  ({ next with lastRelease: if List.isEmpty(released) then m.lastRelease else List.last(released), pulses: pulses }, released)

// --- haptics ---

let feltOf = (m: Model, side: Hold.Side): Pulses.Hand =>
  match side with
  | Hold.Left => m.feltLeft
  | Hold.Right => m.feltRight

let other = (side: Hold.Side): Hold.Side =>
  match side with
  | Hold.Left => Hold.Right
  | Hold.Right => Hold.Left

/// What a hand's hold did across a step (`m` before; what it is on in
/// `next`): it took a prop (out of a slot or not; or a rifle's foregrip), or
/// put its prop away, lost it, or let it go. A prop handed to the other hand
/// is that hand's grab; a rifle whose other hand stays on it, no event. A
/// support hand's let-go or break is `supporting`'s, so a hand that was one
/// has no events here.
let holdEvents = (m: Model, next: Model, released: List<Release>, side: Hold.Side): List<Pulses.Event> =>
  let was = inHand(m, side) in
  let now = inHand(next, side) in
  if was == now || (m.props |> List.any((p) => supportOf(p) == Option.Some(side))) then []
  else
    match now with
    | Option.Some(k) => if Option.isSome(propOf(m, k).slot) then [Pulses.Unstow] else [Pulses.Grab]
    | Option.None =>
      (match was with
       | Option.None => []
       | Option.Some(k) =>
         let p = propOf(next, k) in
         if Option.isSome(p.slot) then [Pulses.Stow]
         else if heldBy(other(side), p) then []
         else if released |> List.any((r) => r.prop == k && r.kind == Break) then [Pulses.Break]
         else [Pulses.Release])

/// One hand's pulses for a step (its guns' and its support's, from `fired`,
/// then its hold's events), and its new edges: blocked (its prop held off by the level), and
/// the slot it holds its prop in (an empty hand is in none, as far as pulses
/// go), and whether that slot takes the prop. Untracked, a hold survives, and
/// so do its edges: a hand coming back still pressed or still in the slot
/// doesn't buzz again.
let feelHand = (m: Model, next: Model, released: List<Release>, slots: List<Slots.Placed>, fired: List<(Hold.Side, Pulses.Event)>, side: Hold.Side, t: float): (Pulses.Hand, List<Pulses.Event>) =>
  let h = feltOf(m, side) in
  let holding = propIn(next, side) in
  let shots = fired |> List.filter((se) => let (s, _) = se in s == side) |> List.map((se) => let (_, e) = se in e) in
  let held = List.append(shots, holdEvents(m, next, released, side)) in
  match gripOf(next.xr, side) with
  | Option.None =>
    let kept = Option.isSome(holding) in
    Pulses.step(h, held, kept && h.blocked, if kept then h.slot else Option.None, false, t)
  | Option.Some(tracked) =>
    let g = steeredGrip(next, side) |> Option.defaultValue(tracked) in
    let blockedNow =
      holding |> Option.bind((p) => p.held) |> Option.map((on) => Pulses.blocked(h.blocked, g, Hold.gripOn(on)))
        |> Option.defaultValue(false) in
    let slot = if List.isEmpty(slots) || Option.isNone(holding) then Option.None else Slots.target(slots, g.position) in
    let takes = slot |> Option.bind((s) => holding |> Option.map((p) => opens(next, p.kind, s))) |> Option.defaultValue(false) in
    Pulses.step(h, held, blockedNow, slot |> Option.map((s) => s.slot.id), takes, t)

/// Both hands' pulses for a step: the model with their edges, and the
/// (hand, event) pairs to buzz.
let feel = (m: Model, next: Model, released: List<Release>, slots: List<Slots.Placed>, fired: List<(Hold.Side, Pulses.Event)>, t: float): (Model, List<(Hold.Side, Pulses.Event)>) =>
  let (left, l) = feelHand(m, next, released, slots, fired, Hold.Left, t) in
  let (right, r) = feelHand(m, next, released, slots, fired, Hold.Right, t) in
  ({ next with feltLeft: left, feltRight: right },
   List.append(l |> List.map((e) => (Hold.Left, e)), r |> List.map((e) => (Hold.Right, e))))

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

// Each fixed body's tag and name, to name what a shot struck.
let solidTags = Stations.solids |> List.map((s) => let (name, _) = s in (Physics.tag(name), name))
// The muzzle check sweeps a 2 mm ball along it.
let muzzleProbe = Physics.sphere(0.002)

/// The world at a station, rig-local: the props' sweeps, a shot's ray (past
/// its gun's own body), and the muzzle check against the level only.
let worldOf = (st: Stations.Station, gen: float): World => {
  sweep: castOf(st),
  ray: (k: Props.Kind, from: Input.point3, dir: Input.point3) =>
    let hit = Physics.castExcluding(tagOf(gen, k), Quat.toVec3(Stations.toWorld(st, from)), Quat.toVec3(Stations.dirToWorld(st, dir)), Gun.range) in
    if not hit.hit then Option.None
    else
      let name = solidTags |> List.find((tn) => let (tag, _) = tn in tag == hit.tag) |> Option.map((tn) => let (_, n) = tn in n) |> Option.defaultValue("prop") in
      Option.Some({ point: Stations.toRig(st, Quat.v3(hit.x, hit.y, hit.z)), tag: name, target: Stations.isTarget(name) }),
  blocked: (from: Input.point3, to: Input.point3) =>
    Option.isSome(Physics.sweep(muzzleProbe, posed(st, Hold.at(from, Quat.identity)), posed(st, Hold.at(to, Quat.identity)), Physics.SweepFixed))
}

let tick = (m: Model, dt: float, tts: float) =>
  let st = Stations.find(m.station) in
  let read = (p: Prop): Prop =>
    // A slotted prop has no body to read.
    if m.fresh || Option.isSome(p.slot) then p
    else
      match Physics.pose(tagOf(m.gen, p.kind)) with
      | Option.Some(q) =>
        let v = Physics.linearVelocity(tagOf(m.gen, p.kind)) in
        { p with pose: unposed(st, q), velocity: Stations.dirToRig(st, Quat.v3(v.x, v.y, v.z)) }
      | Option.None => p in
  let (next, released) = step(m, st, worldOf(st, m.gen), read, tts, dt) in
  // Declared dynamic and given their velocities in the same frame: both land
  // on the next step, so none spends a step at rest.
  let throws = released |> List.concatMap((r) =>
    [Physics.setVelocity(tagOf(m.gen, r.prop), Quat.toVec3(r.velocity)),
     Physics.setAngularVelocity(tagOf(m.gen, r.prop), Quat.toVec3(r.spin))]) in
  let buzzes = next.pulses |> List.map((se) => let (side, e) = se in Pulses.effect(side, e)) in
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

let noWorld: World = {
  sweep: (k: Props.Kind, from: Input.pose, to: Input.pose) => Option.None,
  ray: (k: Props.Kind, from: Input.point3, dir: Input.point3) => Option.None,
  blocked: (from: Input.point3, to: Input.point3) => false
}
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
  let (next, _) = step(withHands(m, away, c), Stations.grabTable, noWorld, (p) => p, t, 1.0 / 60.0) in
  (next, next.pulses)
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
  let w = { noWorld with sweep: table } in
  let (pressed, _) = step(into, Stations.grabTable, w, (p) => p, 2.0 / 60.0, 1.0 / 60.0) in
  let (still, _) = step(pressed, Stations.grabTable, w, (p) => p, 3.0 / 60.0, 1.0 / 60.0) in
  let (lost, _) = step(withHands(still, away, away), Stations.grabTable, w, (p) => p, 4.0 / 60.0, 1.0 / 60.0) in
  let (back, _) = step(withHands(lost, away, hand(Quat.add(rest(Props.Ball), Quat.v3(0.0, -0.05, 0.0)), 1.0)), Stations.grabTable, w, (p) => p, 5.0 / 60.0, 1.0 / 60.0) in
  // Lost for a step and back still pressed: no second buzz.
  pressed.pulses == [(Hold.Right, Pulses.Blocked)] && still.pulses == [] && lost.pulses == [] && back.pulses == []
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

/// Where a hand is drawn (`drawnGrip`); a rifle's support hand, in
/// `Accepted`, on the foregrip of the rifle as the hold accepted it.
let drawnHand = (m: Model, side: Hold.Side, g: Input.pose): Input.pose =>
  let onFore =
    m.props |> List.find((p) => supportOf(p) == Option.Some(side)) |> Option.bind((p) =>
      p.held |> Option.bind((h) => p.support |> Option.bind((s) => gripOf(m.xr, h.side) |> Option.map((primary) =>
        Longarm.supportOn(h.accepted, Hold.compose(Longarm.steer(primary, s), h.offset), g))))) in
  match onFore with
  | Option.Some(on) => if m.handView == Accepted then on else g
  | Option.None => drawnGrip(m.handView, g, heldIn(m, side))

// A held ball pushed 10 cm into a wall: drawn tracked, or at the wall.
expect (
  let blocked = { side: Hold.Right, offset: Hold.origin, accepted: Hold.p(-0.55, 1.0, -0.5), history: [], snapTo: Option.None } in
  let tracked = Hold.p(-0.65, 1.0, -0.5) in
  Hold.near(drawnGrip(Tracked, tracked, Option.Some(blocked)), tracked)
    && Hold.near(drawnGrip(Accepted, tracked, Option.Some(blocked)), blocked.accepted)
    && Hold.near(drawnGrip(Accepted, tracked, Option.None), tracked)
)
/// A sampled input with only keys pressed and XR, for checks.
let snapshotOf = (pressed: List<Key.t>, xr: Option.t<Input.xr>): Input.snapshot =>
  { heldKeys: [], pressedKeys: pressed, releasedKeys: [],
    mouse: { x: 0.0, y: 0.0, surfaceWidth: 1.0, surfaceHeight: 1.0,
             buttons: { left: false, right: false, middle: false },
             pressed: { left: false, right: false, middle: false },
             released: { left: false, right: false, middle: false } },
    xr: xr, gamepad: Option.None, touch: Option.None }

// H switches it, and back; so does a thumbstick click, once per press.
expect (
  let keys = snapshotOf in
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

// A disc facing the player at an armory target (local -Z, `Scene.atPose`).
let disc = (r: float, z: float, c: Color.t) =>
  Scene.cylinder() |> Scene.scaleXYZ(2.0 * r, 0.002, 2.0 * r) |> Scene.rotateX(90deg) |> Scene.translate(Vec3.make(0.0, 0.0, z)) |> Scene.lit(c)

let target =
  let red = Color.rgb(0.85, 0.15, 0.12) in
  Scene.group([
    Scene.cube() |> Scene.scaleXYZ(Stations.targetSize, Stations.targetSize, 0.04) |> Scene.lit(Color.rgb(0.92, 0.9, 0.85)),
    disc(0.12, -0.021, red),
    disc(0.08, -0.022, Color.rgb(0.95, 0.95, 0.92)),
    disc(0.04, -0.023, red),
  ])

/// A rig-local point at the armory, as `Scene.atPose` places a subtree.
let atRange = (p: Input.point3) => posed(Stations.armory, Hold.at(p, Quat.identity))

let lab =
  Scene.group([
    Scene.plane() |> Scene.scale(30.0) |> Scene.lit(Color.rgb(0.2, 0.21, 0.24)),
    pad(zero, 1.4, Color.rgb(0.3, 0.32, 0.4)),
    box(Stations.table, Color.rgb(0.55, 0.4, 0.26)),
    box(Stations.wall, Color.rgb(0.62, 0.64, 0.68)),
    box(Stations.bench, Color.rgb(0.55, 0.4, 0.26)),
    Scene.group(Stations.basket |> List.map((b) => box(b, Color.rgb(0.95, 0.5, 0.15)))),
    box({ center: Quat.add(Stations.boardAt, Quat.v3(0.0, 0.0, 0.05)), size: Quat.v3(0.6, 0.45, 0.04) }, Color.rgb(0.05, 0.05, 0.07)),
    box(Stations.armoryBench, Color.rgb(0.4, 0.42, 0.46)),
    box(Stations.armoryWall, Color.rgb(0.62, 0.64, 0.68)),
    box(Stations.backstop, Color.rgb(0.42, 0.33, 0.25)),
    Scene.group(Stations.targetsAt |> List.map((nc) => let (_, c) = nc in target |> Scene.atPose(atRange(c)))),
    box(Stations.atArmory(Quat.add(Stations.rangeBoardAt, Quat.v3(0.0, 0.0, -0.01)), Quat.v3(0.6, 0.45, 0.02)), Color.rgb(0.05, 0.05, 0.07)),
    Scene.group(Stations.rackPosts |> List.map((b) => box(b, Color.rgb(0.3, 0.24, 0.17)))),
  ])

/// A score at `at` (world), facing a station turned `yaw`.
let board = (score: float, at: Input.point3, yaw: float) =>
  Digits.number(score, Color.rgb(1.0, 0.75, 0.2), Color.rgb(0.05, 0.04, 0.03))
    |> Scene.scale(0.09)
    |> Scene.rotateY(Angle.radians(yaw + Math.pi))
    |> Scene.translate(Quat.toVec3(at))

// --- the ammo's look ---

let readoutOn = Color.rgb(1.0, 0.75, 0.2)
let readoutDry = Color.rgb(1.0, 0.2, 0.15)
let readoutOff = Color.rgb(0.05, 0.04, 0.03)
/// How long a readout shows red after a dry pull.
let drySeconds = 0.3

/// A readout's glyphs, digits 1 wide, centred: the magazine's rounds (two
/// dashes with none in), a plus, the chambered round (1 or 0); red after a
/// dry pull. `(rounds or -1, chambered, dry)`.
let readoutGlyphs = (shown: (float, bool, bool)): Scene.t =>
  let (rounds, chambered, dry) = shown in
  let on = if dry then readoutDry else readoutOn in
  let dash = (x: float) => Digits.digit(Digits.dash, on, readoutOff) |> Scene.translate(Vec3.make(x, 0.0, 0.0)) in
  let bar = (w: float, h: float) => Scene.cube() |> Scene.scaleXYZ(w, h, 0.12) |> Scene.emissive(on) in
  Scene.group([
    if rounds < 0.0 then Scene.group([dash(-0.8), dash(0.8)]) else Digits.number(rounds, on, readoutOff),
    Scene.group([bar(0.7, 0.14), bar(0.14, 0.7)]) |> Scene.translate(Vec3.make(2.15, 0.0, 0.0)),
    Digits.digit(if chambered then 1.0 else 0.0, on, readoutOff) |> Scene.translate(Vec3.make(3.5, 0.0, 0.0)),
  ]) |> Scene.translate(Vec3.make(-1.35, 0.0, 0.0))

/// What a gun's ammo adds to it, in its frame: the readout where its weapon
/// puts it, and the seated magazine, if any. `(kind, rounds or -1,
/// chambered, dry)`.
let ammoScene = (shown: (Props.Kind, float, bool, bool)): Scene.t =>
  let (k, rounds, chambered, dry) = shown in
  let w = weaponOf(k) in
  Scene.group([
    readoutGlyphs((rounds, chambered, dry))
      |> Scene.scale(w.readout.size) |> Scene.rotateX(Angle.radians(w.readout.tilt)) |> Scene.translate(Quat.toVec3(w.readout.at))
      |> Scene.rotateY(180deg),
    if rounds < 0.0 then Scene.group([]) else Gun.seated(w, Gun.magazine(w.well.kind, litPaint)),
  ])

/// A magazine prop with its count on its back (toward the shooter as it
/// drops out, and toward the player in its cradle). `(kind, look, rounds)`.
let magScene = (shown: (Ammo.Kind, Props.Look, float)): Scene.t =>
  let (fits, look, rounds) = shown in
  Scene.group([
    Props.scene(Props.Mag(fits, 0.0), look),
    Digits.number(rounds, readoutOn, readoutOff) |> Scene.scale(0.009) |> Scene.rotateY(180deg)
      |> Scene.translate(Vec3.make(0.0, 0.005, 0.0 - Gun.magFace(fits) - 0.0007)),
  ])

let litPaint = (r: float, g: float, b: float, s: Scene.t): Scene.t => s |> Scene.lit(Color.rgb(r, g, b))

// Held, drawn at the pose it is swept at; slotted, at its slot; free, where
// the world has it. A gun with its seated magazine and readout; a magazine
// with its count.
let propScene = (m: Model, st: Stations.Station, p: Prop, t: float) =>
  let look =
    if Option.isSome(p.held) then Props.Taken
    else if m.left.hover == Option.Some(p.kind) || m.right.hover == Option.Some(p.kind) then Props.Hovered
    else Props.Resting in
  let pose =
    match p.held with
    // A gun is drawn kicked by its recoil; it fires from `accepted`.
    | Option.Some(h) => posed(st, Props.weapon(p.kind) |> Option.map((w) => Gun.kicked(w, (gunOf(m, p.kind)).kick, h.accepted)) |> Option.defaultValue(h.accepted))
    | Option.None =>
      if Option.isSome(p.slot) then posed(st, p.pose)
      else Physics.pose(tagOf(m.gen, p.kind)) |> Option.defaultValue(posed(st, p.at)) in
  let shown =
    match Props.weapon(p.kind) with
    | Option.Some(_) =>
      let g = gunOf(m, p.kind) in
      let rounds = g.ammo.mag |> Option.map((mg) => mg.rounds) |> Option.defaultValue(-1.0) in
      let dry = t >= g.dryAt && t - g.dryAt < drySeconds in
      Scene.group(List.flatten([
        [Props.scene(p.kind, look), Scene.memo($"ammo-{Props.name(p.kind)}", (p.kind, rounds, g.ammo.chambered, dry), ammoScene)],
        if p.kind == Props.Rifle then [Longarm.selector(m.rifle.mode)] else [],
      ]))
    | Option.None =>
      (match p.kind with
       | Props.Mag(fits, id) => Scene.memo($"mag-{id}", (fits, look, Ammo.roundsIn(p.mag)), magScene)
       | _ => Props.scene(p.kind, look)) in
  shown |> Scene.atPose(pose)

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

/// A placed slot's look, for what the hands in it offer.
let slotLook = (m: Model, s: Slots.Placed): Slots.Look => Slots.look(s.slot, occupied(m, s.slot.id), offered(m, s))
let slotScene = (m: Model, st: Stations.Station, s: Slots.Placed, look: Slots.Look): Scene.t =>
  Slots.scene(s.slot, look) |> Scene.atPose(posed(st, s.pose))

/// The slots (each lit by what is offered to it), the belt, spine and gaze,
/// where they are worn and the head is tracked.
let bodyScene = (m: Model, st: Stations.Station): Scene.t =>
  match (if Stations.worn(st) then m.hips.head else Option.None) with
  | Option.None => Scene.group([])
  | Option.Some(head) =>
    let slots = Slots.placed(m.hips) in
    let up = Quat.v3(0.0, 0.22, 0.0) in
    Scene.group([
      Scene.group(slots |> List.map((s) => slotScene(m, st, s, slotLook(m, s)))),
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
    let drawn = drawnHand(m, side, g) in
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

// --- the range ---

let flashSeconds = 0.05
let tracerSeconds = 0.1
let hitGlowSeconds = 0.3

/// The rotation whose -Z is the unit `dir` (a shot's way), level when it can be.
let along = (dir: Input.point3): Input.quaternion =>
  let back = Quat.scale(0.0 - 1.0, dir) in
  let side = Quat.cross(Quat.v3(0.0, 1.0, 0.0), back) in
  let x = if Quat.length(side) < 0.001 then Quat.v3(1.0, 0.0, 0.0) else Quat.scale(1.0 / Quat.length(side), side) in
  Quat.fromBasis(x, Quat.cross(back, x), back)

expect (let d = Quat.v3(0.6, 0.0, -0.8) in Hold.near3(Quat.rotate(along(d), Quat.v3(0.0, 0.0, -1.0)), d))

/// What the shots left at the armory: the hit markers, the target last hit
/// lit for a moment, the last shot's tracer and muzzle flash fading out, and
/// the score.
let shotsScene = (m: Model, st: Stations.Station, t: float): Scene.t =>
  if st.id != Stations.armory.id then Scene.group([])
  else
    let marker = disc(0.012, -0.026, Color.rgb(0.1, 0.1, 0.1)) in
    // Each gun's hit markers, lit target, tracer and flash.
    let gunScene = (k: Props.Kind): List<Scene.t> =>
      let g = gunOf(m, k) in
      let last = g.last |> Option.filter((s) => t - s.t >= 0.0 && t - s.t < hitGlowSeconds) in
      let lit = last |> Option.bind((s) => s.impact) |> Option.filter((i) => i.target)
        |> Option.bind((i) => Stations.targetCentre(i.tag))
        |> Option.map((c) => [Scene.cube() |> Scene.scaleXYZ(Stations.targetSize, Stations.targetSize, 0.002)
             |> Scene.translate(Vec3.make(0.0, 0.0, -0.028)) |> Scene.emissive(Color.rgb(1.0, 0.85, 0.3)) |> Scene.opacity(0.6) |> Scene.atPose(atRange(c))])
        |> Option.defaultValue([]) in
      let trail = last |> Option.filter((s) => t - s.t < tracerSeconds) |> Option.map((s) =>
        let fade = 1.0 - (t - s.t) / tracerSeconds in
        // From the muzzle as drawn (kicked), to where the shot struck.
        let from = propOf(m, k).held |> Option.map((h) => (Gun.boreOf(weaponOf(k), Gun.kicked(weaponOf(k), g.kick, h.accepted))).muzzle)
          |> Option.defaultValue(s.from) in
        let end = Gun.endOf(s) in
        let length = Quat.length(Quat.sub(end, from)) in
        let mid = Quat.lerp(from, end, 0.5) in
        let hot = Color.rgb(1.0, 0.85, 0.45) in
        [Scene.cube() |> Scene.scaleXYZ(0.012, 0.012, length) |> Scene.emissive(hot) |> Scene.opacity(0.85 * fade)
           |> Scene.atPose(posed(st, Hold.at(mid, along(Quat.scale(1.0 / length, Quat.sub(end, from)))))),
         if t - s.t < flashSeconds then Scene.sphere() |> Scene.scale(0.035 * fade) |> Scene.emissive(hot) |> Scene.atPose(posed(st, Hold.at(from, Quat.identity)))
         else Scene.group([])])
        |> Option.defaultValue([]) in
      List.flatten([g.hits |> List.map((h) => marker |> Scene.atPose(posed(st, Hold.at(h.point, Quat.identity)))), lit, trail]) in
    Scene.group(List.flatten([
      gunScene(Props.Pistol),
      gunScene(Props.Rifle),
      [board(m.score, Stations.toWorld(st, Stations.rangeBoardAt), st.yaw)],
    ]))

/// The rack or cradle a holding hand is in, lit by what it holds.
let rackScene = (m: Model, st: Stations.Station): Scene.t =>
  let rs = racks(st) in
  let near = [Hold.Left, Hold.Right] |> List.concatMap((side) =>
    match propIn(m, side) |> Option.bind((_) => gripOf(m.xr, side)) |> Option.bind((g) => Slots.target(rs, g.position)) with
    | Option.Some(s) => [s]
    | Option.None => []) in
  let shown = match near with
    | [a, b] => if a.slot.id == b.slot.id then [a] else near
    | _ => near in
  Scene.group(shown |> List.map((s) => slotScene(m, st, s, slotLook(m, s))))

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

// Children 3 (the props, in `Stations.props` order) to 7 are what the e2e
// checks read from `/scene`: the left and right hands, then their ghosts. 8
// is what the body wears; 9 the haptic cues; 10 the range's shots; 11 the
// rifle rack's ring.
let draw = (m: Model, tts) =>
  let st = Stations.find(m.station) in
  let (left, leftGhost) = handScene(m, st, Hold.Left, Color.rgb(0.2, 0.85, 1.0)) in
  let (right, rightGhost) = handScene(m, st, Hold.Right, Color.rgb(1.0, 0.3, 0.7)) in
  Frame.createLit(
    camera(m, st),
    Scene.group([
      lab,
      pad(st.feet, 0.8, Color.rgb(0.2, 0.6, 0.65)),
      board(m.score, Stations.boardAt, Stations.grabTable.yaw),
      Scene.group(m.props |> List.map((p) => propScene(m, st, p, tts))),
      left,
      right,
      leftGhost,
      rightGhost,
      bodyScene(m, st),
      cues(m, st, tts),
      shotsScene(m, st, tts),
      rackScene(m, st),
    ]),
    lights)
    |> Frame.withClearColor(Color.rgb(0.07, 0.08, 0.11))
