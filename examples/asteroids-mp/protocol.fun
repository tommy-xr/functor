// protocol.fun — the shared protocol for the multiplayer asteroids sample.
//
// `file = module`: client.fun and server.fun both load this sibling as
// `Protocol`, so the two roles typecheck against ONE declaration and cannot
// drift. When the netsim transport lands, values of the `Wire` ADT below are
// exactly what `Effect.sendMsg` will carry — no string codec, no parsing; a
// protocol typo is a check-time error on both ends.

// ---------- arena & timing constants ----------
let halfW = 24.0          // half-extent of the playfield in x (world units)
let halfH = 15.0          // half-extent in y
let tickHz = 60.0         // the fixed simulation rate both ends assume
// The simulated round trip, in ticks. The client renders its OWN predicted
// ship immediately, plus a ghost of where the server's confirmation of that
// ship would be — `rttTicks` behind. ~8 ticks at 60 Hz = ~133 ms of RTT.
let rttTicks = 8.0

// ---------- shared entity state (what a Snapshot carries) ----------
type Ship = { pid: float, x: float, y: float, angle: float, vx: float, vy: float }
type Rock = { x: float, y: float, vx: float, vy: float, size: float }
type Bullet = { pid: float, x: float, y: float, vx: float, vy: float, ttl: float }
type Score = { pid: float, points: float }

// What a client wants its ship to do, sampled from held controls each tick.
// turn: -1 (clockwise) / 0 / +1 (counter-clockwise); fire is the held level.
type Intent = { turn: float, thrust: bool, fire: bool }

// ---------- the wire ----------
// Client -> server: Join once, then a Steer per intent change.
// Server -> client: Welcome with your pid, then a Snapshot per tick.
type Wire =
  | Join
  | Welcome(pid: float)
  | Steer(pid: float, intent: Intent)
  | Snapshot(ships: List<Ship>, rocks: List<Rock>, bullets: List<Bullet>, scores: List<Score>)

// ---------- shared geometry ----------
// Wrap a coordinate onto the toroidal arena. Math.mod is Euclidean (always
// non-negative), so one expression handles both edges.
let wrap = (v: float, limit: float): float =>
  Math.mod(v + limit, limit * 2.0) - limit

// Shortest separation on the wrapped field, so entities hugging opposite
// edges still count as neighbors (a bullet at the left edge hits a rock
// peeking in from the right, like the arcade original).
let wrapDelta = (d: float, limit: float): float =>
  if d > limit then d - limit * 2.0
  else if d < 0.0 - limit then d + limit * 2.0
  else d

let dist2 = (ax: float, ay: float, bx: float, by: float): float =>
  let dx = wrapDelta(ax - bx, halfW) in
  let dy = wrapDelta(ay - by, halfH) in
  dx * dx + dy * dy

// ---------- shared tables ----------
let radiusOf = (size: float): float =>
  match size with
  | 3.0 => 2.3
  | 2.0 => 1.4
  | _ => 0.8

let pointsFor = (size: float): float =>
  match size with
  | 3.0 => 20.0
  | 2.0 => 50.0
  | _ => 100.0

let shipRadius = 1.0

// ---------- the prediction ring buffer ----------
// Push a state onto a newest-first history capped at rttTicks + 1 entries.
// The oldest entry (List.nth(rttTicks, history)) is the "server-confirmed"
// state the client's intent ghost renders; when the real transport lands,
// server snapshots replace this simulated round trip.
let pushHistory = (state: 'a, history: List<'a>): List<'a> =>
  [state, ..history] |> List.take(rttTicks + 1.0)

// The buffer never grows past rttTicks + 1 entries...
expect (
  List.range(20.0)
    |> List.fold((h, n) => pushHistory(n, h), [])
    |> List.length
) == rttTicks + 1.0
// ...and its oldest entry lags the newest by exactly rttTicks pushes.
expect (
  let h = List.range(20.0) |> List.fold((h, n) => pushHistory(n, h), []) in
  match List.nth(rttTicks, h) with
  | Option.Some(oldest) => oldest == 19.0 - rttTicks
  | Option.None => false
)

// The wrap helper is load-bearing for both roles — pin it.
expect wrap(halfW + 1.0, halfW) == 1.0 - halfW
expect wrap(0.0 - halfW - 1.0, halfW) == halfW - 1.0
expect wrap(3.0, halfW) == 3.0
expect wrapDelta(2.0 * halfW - 0.5, halfW) == 0.0 - 0.5
