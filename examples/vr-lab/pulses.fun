// Pulses: the controller buzz for each hold event (`Effect.haptic`), as plain
// data, and the per-hand state that fires the edge-triggered ones once.

/// What a hand felt this step.
type Event =
  // It entered an empty slot, holding a prop the slot takes.
  | Ready
  // It let go of a prop (thrown or dropped).
  | Release
  // It took a prop: free, or from the other hand.
  | Grab
  // It took a prop out of a slot.
  | Unstow
  // It put its prop away in a slot.
  | Stow
  // Its prop stopped against the level (the wall, the table) as it went on.
  | Blocked
  // It went too far past its blocked prop and lost it.
  | Break

/// One buzz: how hard (0..1) and how long.
type Pulse = { amplitude: float, seconds: float }

/// Each event's buzz, weakest to strongest: hover and release light, a
/// grab firmer, slots firmer still and longer, a stop sharp, a break the
/// strongest and longest.
let pulse = (e: Event): Pulse =>
  match e with
  | Ready => { amplitude: 0.15, seconds: 0.015 }
  | Release => { amplitude: 0.3, seconds: 0.02 }
  | Grab => { amplitude: 0.45, seconds: 0.03 }
  | Unstow => { amplitude: 0.55, seconds: 0.04 }
  | Stow => { amplitude: 0.7, seconds: 0.05 }
  | Blocked => { amplitude: 0.85, seconds: 0.02 }
  | Break => { amplitude: 1.0, seconds: 0.08 }

let all = [Ready, Release, Grab, Unstow, Stow, Blocked, Break]

// Every event is told apart by its amplitude alone.
expect all |> List.all((a) => all |> List.all((b) => a == b || (pulse(a)).amplitude != (pulse(b)).amplitude))

let effect = (side: Hold.Side, e: Event) =>
  let p = pulse(e) in
  let hand = (match side with | Hold.Left => Xr.Left | Hold.Right => Xr.Right) in
  Effect.haptic(hand, p.amplitude, Time.seconds(p.seconds))

/// The last event a hand fired, and when (for the debug cue).
type Fired = { event: Event, at: float }

/// A hand's edge state: whether its prop is blocked, the slot it is in, and
/// its last pulse.
type Hand = { blocked: bool, slot: Option.t<float>, last: Option.t<Fired> }

let idle: Hand = { blocked: false, slot: Option.None, last: Option.None }

/// Blocked once the hand is as far off where it holds its prop as a drawn
/// ghost (`Hold.ghosted`: 2 cm or 10°); free again under half that, so a
/// hand resting at the edge doesn't buzz twice.
let blocked = (was: bool, grip: Input.pose, holds: Input.pose): bool =>
  if was then
    Quat.length(Quat.sub(grip.position, holds.position)) > Hold.ghostDistance / 2.0
      || Quat.angle(grip.orientation, holds.orientation) > Hold.ghostTurn / 2.0
  else Hold.ghosted(grip, holds)

expect blocked(false, Hold.p(0.0, 0.0, 0.0), Hold.p(0.025, 0.0, 0.0))
expect not blocked(false, Hold.p(0.0, 0.0, 0.0), Hold.p(0.015, 0.0, 0.0))
expect blocked(true, Hold.p(0.0, 0.0, 0.0), Hold.p(0.015, 0.0, 0.0))
expect not blocked(true, Hold.p(0.0, 0.0, 0.0), Hold.p(0.009, 0.0, 0.0))

/// Step a hand's edges: whether its prop is blocked now, the slot it is in
/// now, and whether that slot would take its prop. Its events are `held` (a
/// grab, release, … from comparing holds), then the edges that rose: a stop,
/// or entering a slot ready for its prop (not one it was already in, so a
/// prop just taken out doesn't buzz again).
let step = (h: Hand, held: List<Event>, blockedNow: bool, slotNow: Option.t<float>, takes: bool, t: float): (Hand, List<Event>) =>
  let rising =
    List.append(
      if blockedNow && not h.blocked then [Blocked] else [],
      if takes && Option.isSome(slotNow) && slotNow != h.slot then [Ready] else []) in
  let events = List.append(held, rising) in
  let last = events |> List.last |> Option.map((e) => { event: e, at: t }) in
  ({ blocked: blockedNow, slot: slotNow, last: if Option.isSome(last) then last else h.last }, events)

// Pressed into the wall it buzzes once, not again while held there.
expect (
  let (h1, e1) = step(idle, [], true, Option.None, false, 0.0) in
  let (h2, e2) = step(h1, [], true, Option.None, false, 0.1) in
  e1 == [Blocked] && e2 == [] && h2.last == Option.Some({ event: Blocked, at: 0.0 })
)
// Ready once on entering a slot that takes the prop, again only for another;
// none for one that won't, or for a prop taken out where the hand already is.
expect (
  let (h1, e1) = step(idle, [], false, Option.Some(2.0), true, 0.0) in
  let (h2, e2) = step(h1, [], false, Option.Some(2.0), true, 0.1) in
  let (_, e3) = step(h2, [], false, Option.Some(1.0), true, 0.2) in
  let (_, refused) = step(idle, [], false, Option.Some(2.0), false, 0.0) in
  let (inside, _) = step(idle, [], false, Option.Some(2.0), false, 0.0) in
  let (_, takenOut) = step(inside, [Unstow], false, Option.Some(2.0), true, 0.1) in
  e1 == [Ready] && e2 == [] && e3 == [Ready] && refused == [] && takenOut == [Unstow]
)

// --- the debug cue ---

/// How long a pulse's cue shows.
let cueSeconds = 0.35

// A dashed ring of radius 1, level, as `Slots` draws its own.
let ring =
  List.range(16.0) |> List.map((i) =>
    let a = i * Math.pi / 8.0 in
    Instance.trs(Vec3.make(Math.sin(a), 0.0, Math.cos(a)), Angle.radians(a), 0.25, 0.12, 0.12))

/// A pulse made visible at the hand that felt it: a ring and a flash sized
/// by its amplitude, shrinking away over `cueSeconds`. None once it is over.
let cue = (h: Hand, t: float): Option.t<Scene.t> =>
  h.last |> Option.bind((f) =>
    let age = t - f.at in
    if age < 0.0 || age >= cueSeconds then Option.None
    else
      let fade = 1.0 - age / cueSeconds in
      let size = (0.04 + 0.1 * (pulse(f.event)).amplitude) * (0.6 + 0.4 * fade) in
      let glow = Color.rgb(1.0, 0.9, 0.35) in
      Option.Some(Scene.group([
        Scene.cube() |> Scene.emissive(glow) |> Scene.instanced(ring) |> Scene.scale(size),
        Scene.sphere() |> Scene.scale(size * 0.6) |> Scene.emissive(glow) |> Scene.opacity(0.35 * fade),
      ])))

expect Option.isNone(cue(idle, 1.0))
expect Option.isSome(cue({ idle with last: Option.Some({ event: Grab, at: 1.0 }) }, 1.2))
expect Option.isNone(cue({ idle with last: Option.Some({ event: Grab, at: 1.0 }) }, 1.4))
