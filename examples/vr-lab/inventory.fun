// The inventory station's contract, headless: Game.step with no world, a
// standing head, and hands moved by script.

let dt = 1.0 / 60.0
let start = Game.reset(Game.init, Stations.inventory.id)
let openAt = (p: Input.point3) => Game.hand(p, 0.0)
let closedAt = (p: Input.point3) => Game.hand(p, 1.0)
let rest = Game.rest

// A standing head (`Hips.headAt`): the pelvis is under the neck, at
// (0, 1.49 × 0.62, 0.09), and the slots at their offsets from it.
let root = Hips.rootOf(Hips.headAt(0.0, 0.0))
let slotAt = (s: Slots.Slot): Input.point3 => Quat.add(root, s.offset.position)
let rightHolster = slotAt(Slots.rightHolster)
let leftHolster = slotAt(Slots.leftHolster)
let pouch = slotAt(Slots.pouch)
let off = (p: Input.point3, x: float, y: float, z: float) => Quat.add(p, Quat.v3(x, y, z))

/// One step's input: the head turned `turn` degrees, and both hands.
type Step = { turn: float, l: Input.controller, r: Input.controller }
let hands = (l: Input.controller, r: Input.controller): Step => { turn: 0.0, l: l, r: r }

/// Every model a script passes through, the start first.
let play = (m0: Game.Model, script: List<Step>): List<Game.Model> =>
  script
    |> List.fold((acc, x) =>
         match acc with
         | [m, ..rest] =>
           let xr = { head: Option.Some(Hips.headAt(x.turn, 0.0)), left: x.l, right: x.r } in
           let (next, _) = Game.step({ m with xr: Option.Some(xr) }, Stations.inventory, Game.noWorld, (p) => p, List.length(acc) * dt, dt) in
           [next, m, ..rest]
         | [] => [],
       [m0])
    |> List.reverse

let last = (ms: List<Game.Model>): Game.Model => List.last(ms) |> Option.defaultValue(start)
let nth = (i: float, ms: List<Game.Model>): Game.Model => List.nth(i, ms) |> Option.defaultValue(start)
let prop = (m: Game.Model, k: Props.Kind): Game.Prop => Game.propOf(m, k)
let slotOf = (m: Game.Model, k: Props.Kind) => prop(m, k).slot
let holder = (m: Game.Model, k: Props.Kind) => prop(m, k).held |> Option.map((h) => h.side)

/// Every prop in exactly one place (a hand, a slot, or free), no slot or
/// hand with two.
let conserved = (m: Game.Model): bool =>
  let held = m.props |> List.filter((p) => Option.isSome(p.held)) |> List.length in
  let slotted = m.props |> List.filter((p) => Option.isSome(p.slot)) |> List.length in
  let free = m.props |> List.filter((p) => Option.isNone(p.held) && Option.isNone(p.slot)) |> List.length in
  let once = (n: float) => n <= 1.0 in
  held + slotted + free == List.length(start.props) && List.length(m.props) == List.length(start.props)
    && (Slots.all |> List.all((s) => once(m.props |> List.filter((p) => p.slot == Option.Some(s.id)) |> List.length)))
    && once(m.props |> List.filter((p) => holder(m, p.kind) == Option.Some(Hold.Left)) |> List.length)
    && once(m.props |> List.filter((p) => holder(m, p.kind) == Option.Some(Hold.Right)) |> List.length)

let away = Game.away
let brick = rest(Props.Brick)
let bottleBase = Game.bottleBase
// The right hand takes the brick, the left the bottle by its base.
let takeBoth = [hands(openAt(bottleBase), openAt(brick)), hands(closedAt(bottleBase), closedAt(brick))]
let takeBrick = [hands(away, openAt(brick)), hands(away, closedAt(brick))]

// --- deposit, retrieve, hand-off ---

// Brick into the right holster; the left takes it back out across the body
// and hands it to the right, which puts it in the left holster.
let tour = play(start, List.flatten([
  takeBrick,
  [hands(away, closedAt(rightHolster)),
   hands(away, openAt(rightHolster)),                                  // 4: stored
   hands(openAt(off(rightHolster, -0.02, 0.0, 0.0)), openAt(Quat.v3(0.3, 1.0, -0.3))),
   hands(closedAt(off(rightHolster, -0.02, 0.0, 0.0)), openAt(Quat.v3(0.3, 1.0, -0.3))),   // 6: retrieved
   hands(closedAt(off(rightHolster, -0.02, 0.0, 0.0)), openAt(rightHolster)),
   hands(closedAt(off(rightHolster, -0.02, 0.0, 0.0)), closedAt(rightHolster)),            // 8: handed off
   hands(openAt(off(rightHolster, -0.02, 0.0, 0.0)), closedAt(leftHolster)),
   hands(away, openAt(leftHolster))],                                  // 10: stored
]))

expect tour |> List.all(conserved)
expect slotOf(nth(3.0, tour), Props.Brick) == Option.None && slotOf(nth(4.0, tour), Props.Brick) == Option.Some(Slots.rightHolster.id)
// Stored, it sits at the slot, and has no body.
expect Hold.near(prop(nth(4.0, tour), Props.Brick).pose, Hold.at(rightHolster, Quat.identity))
expect (
  let m = nth(4.0, tour) in
  Option.isNone(m.lastRelease) && Option.isNone(prop(m, Props.Brick).held)
)
// Taken back out exactly where it sat: no pop.
expect (
  let m = nth(6.0, tour) in
  holder(m, Props.Brick) == Option.Some(Hold.Left) && slotOf(m, Props.Brick) == Option.None
    && (prop(m, Props.Brick).held |> Option.map((h) => Hold.near(h.accepted, prop(nth(5.0, tour), Props.Brick).pose)) |> Option.defaultValue(false))
)
expect holder(nth(8.0, tour), Props.Brick) == Option.Some(Hold.Right)
expect slotOf(last(tour), Props.Brick) == Option.Some(Slots.leftHolster.id) && Option.isNone(holder(last(tour), Props.Brick))

// --- what a slot shows ---

let lookAt = (m: Game.Model, s: Slots.Slot): Slots.Look =>
  Slots.placed(m.hips)
    |> List.find((p) => p.slot.id == s.id)
    |> Option.map((p) => Slots.look(s, Game.occupied(m, s.id), Game.offered(m, p)))
    |> Option.defaultValue(Slots.Idle)

// Offered the brick it takes, a holster is ready; offered the stick, it refuses;
// full, it refuses the bottle. Nothing offered, idle.
expect lookAt(nth(3.0, tour), Slots.rightHolster) == Slots.Ready && lookAt(nth(3.0, tour), Slots.leftHolster) == Slots.Idle
expect (
  let stick = Game.stickHandle in
  let m = last(play(start, [hands(away, openAt(stick)), hands(away, closedAt(stick)), hands(away, closedAt(leftHolster))])) in
  lookAt(m, Slots.leftHolster) == Slots.Refused
)

// --- what won't go in ---

// The stick fits no slot: let go in the holster, it drops.
expect (
  let stick = Game.stickHandle in
  let m = last(play(start, [hands(away, openAt(stick)), hands(away, closedAt(stick)), hands(away, closedAt(rightHolster)), hands(away, openAt(rightHolster))])) in
  slotOf(m, Props.Stick) == Option.None && Game.releaseOf(m, Props.Stick, Game.Drop) && conserved(m)
)
// The ball doesn't fit a holster, but fits the pouch.
expect (
  let ball = rest(Props.Ball) in
  let into = (at: Input.point3) => last(play(start, [hands(away, openAt(ball)), hands(away, closedAt(ball)), hands(away, closedAt(at)), hands(away, openAt(at))])) in
  slotOf(into(leftHolster), Props.Ball) == Option.None && slotOf(into(pouch), Props.Ball) == Option.Some(Slots.pouch.id)
)
// A full holster refuses the bottle: it drops, and the brick stays.
expect (
  let m = last(play(start, List.flatten([
    takeBoth,
    [hands(closedAt(bottleBase), closedAt(rightHolster)), hands(closedAt(bottleBase), openAt(rightHolster)),
     hands(closedAt(rightHolster), openAt(Quat.v3(0.3, 1.0, -0.3))), hands(openAt(rightHolster), openAt(Quat.v3(0.3, 1.0, -0.3)))],
  ]))) in
  slotOf(m, Props.Brick) == Option.Some(Slots.rightHolster.id) && slotOf(m, Props.Bottle) == Option.None
    && Game.releaseOf(m, Props.Bottle, Game.Drop) && conserved(m)
)

// --- ordering ---

// Both hands open in one holster in one step: the nearer its centre stores,
// the right on a tie; the other's prop is let go.
let both = (lx: float, rx: float) =>
  last(play(start, List.flatten([
    takeBoth,
    [hands(closedAt(off(rightHolster, lx, 0.0, 0.0)), closedAt(off(rightHolster, rx, 0.0, 0.0))),
     hands(openAt(off(rightHolster, lx, 0.0, 0.0)), openAt(off(rightHolster, rx, 0.0, 0.0)))],
  ])))
expect (
  let m = both(-0.03, 0.01) in
  slotOf(m, Props.Brick) == Option.Some(Slots.rightHolster.id) && slotOf(m, Props.Bottle) == Option.None
    && Game.releaseOf(m, Props.Bottle, Game.Drop) && conserved(m)
)
expect (
  let m = both(-0.01, 0.03) in
  slotOf(m, Props.Bottle) == Option.Some(Slots.rightHolster.id) && slotOf(m, Props.Brick) == Option.None && conserved(m)
)
expect slotOf(both(-0.02, 0.02), Props.Brick) == Option.Some(Slots.rightHolster.id)

// Storing beats a hand-off: the right opens in the holster as the left
// squeezes on the brick; it goes in the holster, and the left gets nothing.
expect (
  let ms = play(start, List.flatten([
    takeBrick,
    [hands(openAt(off(rightHolster, -0.03, 0.0, 0.0)), closedAt(rightHolster)),
     hands(closedAt(off(rightHolster, -0.03, 0.0, 0.0)), openAt(rightHolster)),
     hands(closedAt(off(rightHolster, -0.03, 0.0, 0.0)), openAt(Quat.v3(0.3, 1.0, -0.3)))],
  ])) in
  slotOf(last(ms), Props.Brick) == Option.Some(Slots.rightHolster.id) && Option.isNone(holder(last(ms), Props.Brick))
    && (ms |> List.all(conserved))
)

// --- tracking loss ---

// Lost in the holster, the brick stays in the hand, frozen; back already
// open, it drops rather than going in.
expect (
  let ms = play(start, List.flatten([
    takeBrick,
    [hands(away, closedAt(rightHolster)), hands(away, away), hands(away, away), hands(away, away), hands(away, openAt(rightHolster))],
  ])) in
  (List.range(3.0) |> List.all((i) => holder(nth(i + 4.0, ms), Props.Brick) == Option.Some(Hold.Right) && slotOf(nth(i + 4.0, ms), Props.Brick) == Option.None))
    && slotOf(last(ms), Props.Brick) == Option.None && Game.releaseOf(last(ms), Props.Brick, Game.Drop)
    && (ms |> List.all(conserved))
)

// --- the frame ---

// A hand near a slot pins the hips while the head turns past the deadzone;
// moved away, they chase the head again.
expect (
  let near = { turn: 0.0, l: openAt(off(leftHolster, 0.0, 0.0, -0.15)), r: away } in
  let ms = play(start, List.flatten([
    [near],
    List.range(20.0) |> List.map((i) => { near with turn: 2.5 * (i + 1.0) }),
    [{ turn: 50.0, l: away, r: away }],
  ])) in
  (List.range(20.0) |> List.all((i) => let h = nth(i + 2.0, ms).hips in h.pinned && h.yaw == 0.0))
    && not last(ms).hips.pinned && last(ms).hips.yaw > 0.0
)
// Arms hanging by the holsters pin the hips only for `Hips.pinHold` at a
// time (again as the holsters come back round them): turned a quarter turn,
// they come round with the body.
expect (
  let hanging = (turn: float) => { turn: turn, l: openAt(Quat.v3(-0.25, 0.78, 0.08)), r: openAt(Quat.v3(0.25, 0.78, 0.08)) } in
  let m = last(play(start, List.flatten([
    List.range(30.0) |> List.map((i) => hanging(3.0 * (i + 1.0))),
    List.range(330.0) |> List.map((i) => hanging(90.0)),
  ]))) in
  Math.abs(m.hips.yaw - Math.pi / 2.0) < 0.001 && not m.hips.pinned
)
// Stored, the brick rides the hips round as the body turns.
expect (
  let ms = play(start, List.flatten([
    takeBrick,
    [hands(away, closedAt(rightHolster)), hands(away, openAt(rightHolster))],
    List.range(150.0) |> List.map((i) => { turn: Math.min(90.0, 3.0 * (i + 1.0)), l: away, r: away }),
  ])) in
  let m = last(ms) in
  let turned = Quat.add(Hips.rootOf(Hips.headAt(90.0, 0.0)), Quat.rotate(Quat.yaw(m.hips.yaw), Slots.rightHolster.offset.position)) in
  Math.abs(m.hips.yaw - Math.pi / 2.0) < 0.05 && Hold.near3(prop(m, Props.Brick).pose.position, turned)
)

// A station reset returns everything: no prop slotted, the hips forgotten.
expect (
  let r = Game.reset(nth(4.0, tour), Stations.inventory.id) in
  (r.props |> List.all((p) => Option.isNone(p.slot) && Option.isNone(p.held))) && Option.isNone(r.hips.head)
)
