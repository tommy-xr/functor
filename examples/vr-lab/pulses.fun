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
  // Its gun fired (the firing hand's, and a supporting hand's).
  | Fire
  // Its gun's trigger was pulled but nothing fired (the muzzle against the
  // level, or nothing chambered).
  | Click
  // The magazine it holds caught in a well's mouth.
  | Latch
  // The magazine went home in the well (the hand that pushed it, and the
  // gun's).
  | Seat
  // The shell it pushed went into the shotgun's tube (its hand, and the gun's).
  | Load
  // The slide, charging handle or pump it works reached the back.
  | SlideBack
  // It went home (the racking hand's, and the gun's).
  | SlideHome

/// One buzz: how hard (0..1) and how long.
type Pulse = { amplitude: float, seconds: float }

/// Each event's buzz, weakest to strongest: hover and release light (a
/// refused trigger's click lighter still, and shortest; a magazine's latch
/// between them; a shell into the tube a little firmer), a grab firmer, the slide's strokes and slots firmer still
/// and longer, a seat a firm click, a stop sharp, a shot sharper, a break
/// the strongest and longest.
let pulse = (e: Event): Pulse =>
  match e with
  | Ready => { amplitude: 0.15, seconds: 0.015 }
  | Latch => { amplitude: 0.2, seconds: 0.012 }
  | Load => { amplitude: 0.4, seconds: 0.015 }
  | Click => { amplitude: 0.25, seconds: 0.01 }
  | SlideBack => { amplitude: 0.5, seconds: 0.025 }
  | SlideHome => { amplitude: 0.65, seconds: 0.03 }
  | Seat => { amplitude: 0.8, seconds: 0.025 }
  | Release => { amplitude: 0.3, seconds: 0.02 }
  | Grab => { amplitude: 0.45, seconds: 0.03 }
  | Unstow => { amplitude: 0.55, seconds: 0.04 }
  | Stow => { amplitude: 0.7, seconds: 0.05 }
  | Blocked => { amplitude: 0.85, seconds: 0.02 }
  | Fire => { amplitude: 0.95, seconds: 0.035 }
  | Break => { amplitude: 1.0, seconds: 0.08 }

let all = [Ready, Latch, Click, Release, Load, Grab, SlideBack, Unstow, SlideHome, Stow, Seat, Blocked, Fire, Break]

// Every event is told apart by its amplitude alone.
expect all |> List.all((a) => all |> List.all((b) => a == b || (pulse(a)).amplitude != (pulse(b)).amplitude))

let effect = (side: Hold.Side, e: Event) =>
  let p = pulse(e) in
  let hand = (match side with | Hold.Left => Xr.Left | Hold.Right => Xr.Right) in
  Effect.haptic(hand, p.amplitude, Time.seconds(p.seconds))

/// The strongest event a hand last fired, and when (for the debug cue).
type Fired = { event: Event, at: float }

/// A hand's edge state: whether its prop is blocked, the slot it is in, and
/// its last pulse.
type Hand = { blocked: bool, slot: Option.t<float>, last: Option.t<Fired> }

let idle: Hand = { blocked: false, slot: Option.None, last: Option.None }

/// Blocked once the hand is as far off where it holds its prop as a drawn
/// ghost (`Hold.ghosted`: 2 cm or 10°); free again under half that, so a
/// hand resting at the edge doesn't buzz twice.
let blocked = (was: bool, grip: Input.pose, holds: Input.pose): bool =>
  Hold.ghostedBy(if was then 0.5 else 1.0, grip, holds)

expect blocked(false, Hold.p(0.0, 0.0, 0.0), Hold.p(0.025, 0.0, 0.0))
expect not blocked(false, Hold.p(0.0, 0.0, 0.0), Hold.p(0.015, 0.0, 0.0))
expect blocked(true, Hold.p(0.0, 0.0, 0.0), Hold.p(0.015, 0.0, 0.0))
expect not blocked(true, Hold.p(0.0, 0.0, 0.0), Hold.p(0.009, 0.0, 0.0))

/// Step a hand's edges: whether its prop is blocked now, the slot it holds
/// it in now, and whether that slot would take it. Its events are `held` (a
/// grab, release, … from comparing holds), then the edges that rose: a stop,
/// or carrying it into a slot ready for it (not on the step it was taken,
/// so a prop taken out of a slot doesn't buzz ready too).
let step = (h: Hand, held: List<Event>, blockedNow: bool, slotNow: Option.t<float>, takes: bool, t: float): (Hand, List<Event>) =>
  if List.isEmpty(held) && blockedNow == h.blocked && slotNow == h.slot then (h, [])
  else
  let rising =
    List.append(
      if blockedNow && not h.blocked then [Blocked] else [],
      if takes && List.isEmpty(held) && Option.isSome(slotNow) && slotNow != h.slot then [Ready] else []) in
  let events = List.append(held, rising) in
  // The cue shows the pulse the hand felt: per hand the strongest wins.
  let strongest = events |> List.sortBy((e) => 0.0 - (pulse(e)).amplitude) |> List.head in
  let last = strongest |> Option.map((e) => { event: e, at: t }) in
  ({ blocked: blockedNow, slot: slotNow, last: if Option.isSome(last) then last else h.last }, events)

// Pressed into the wall it buzzes once, not again while held there.
expect (
  let (h1, e1) = step(idle, [], true, Option.None, false, 0.0) in
  let (h2, e2) = step(h1, [], true, Option.None, false, 0.1) in
  e1 == [Blocked] && e2 == [] && h2.last == Option.Some({ event: Blocked, at: 0.0 })
)
// A stop and a slot in one step: the cue is the stop's, the stronger.
expect (let (h, _) = step(idle, [], true, Option.Some(2.0), true, 0.0) in h.last == Option.Some({ event: Blocked, at: 0.0 }))
// Ready once on entering a slot that takes the prop, again only for another;
// none for one that won't, or for a prop taken out where the hand already is.
expect (
  let (h1, e1) = step(idle, [], false, Option.Some(2.0), true, 0.0) in
  let (h2, e2) = step(h1, [], false, Option.Some(2.0), true, 0.1) in
  let (_, e3) = step(h2, [], false, Option.Some(1.0), true, 0.2) in
  let (_, refused) = step(idle, [], false, Option.Some(2.0), false, 0.0) in
  let (_, takenOut) = step(idle, [Unstow], false, Option.Some(2.0), true, 0.1) in
  e1 == [Ready] && e2 == [] && e3 == [Ready] && refused == [] && takenOut == [Unstow]
)

// --- the debug cue ---

/// How long a pulse's cue shows.
let cueSeconds = 0.35

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
        Scene.cube() |> Scene.emissive(glow) |> Scene.instanced(Slots.dashes) |> Scene.scale(size),
        Scene.sphere() |> Scene.scale(size * 0.6) |> Scene.emissive(glow) |> Scene.opacity(0.35 * fade),
      ])))

expect Option.isNone(cue(idle, 1.0))
expect Option.isSome(cue({ idle with last: Option.Some({ event: Grab, at: 1.0 }) }, 1.2))
expect Option.isNone(cue({ idle with last: Option.Some({ event: Grab, at: 1.0 }) }, 1.4))
