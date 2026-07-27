// game.fun — 2D asteroids, structured for multiplayer, in ONE module.
//
//   functor -d examples/asteroids-mp run native
//
// Controls: Left/Right (or A/D) rotate, Up (or W) thrusts, hold Space to
// fire (the shared cooldown is the fire rate), R or Enter restarts after
// a game over.
//
// The file reads top-to-bottom as the four layers of a networked game:
//
//   PROTOCOL  the wire ADT a real transport will carry (Join/Steer/
//             Snapshot), the arena constants, toroidal geometry, and the
//             prediction ring buffer,
//   SERVER    the authoritative World and its PURE step functions,
//   BOT       a tiny scripted pilot standing in for a remote peer,
//   CLIENT    input, the two joined pilots, prediction, and rendering.
//
// Today the client HOSTS the authoritative simulation in-process. When the
// netsim transport lands, the protocol and server sections split into real
// functor.json `entries` roles (client + server sharing sibling modules) —
// `examples/mp` demonstrates that mechanism natively today — and the world
// moves behind the wire; the functions themselves do not change.

// ======================================================================
// ==========================  PROTOCOL  ================================
// ======================================================================
// The shared protocol: what both roles must agree on. When the netsim
// transport lands, values of the `Wire` ADT below are exactly what
// `Effect.sendMsg` will carry — no string codec, no parsing; a protocol
// typo is a check-time error on both ends.

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
// Client -> server: Join once, then a Steer per intent change. A Steer
// carries NO player id — the server keys it by the connection it arrived
// on (`recv`'s senderPid), so a client can only ever steer itself.
// Server -> client: Welcome with your pid, then a Snapshot per tick.
type Wire =
  | Join
  | Welcome(pid: float)
  | Steer(intent: Intent)
  | Snapshot(ships: List<Ship>, rocks: List<Rock>, bullets: List<Bullet>, scores: List<Score>)

// Read the pid out of a Welcome — the one message a joining client must
// decode. Any other variant answers -1 (join always answers a Welcome).
let welcomePid = (wire: Wire): float =>
  match wire with
  | Welcome(pid) => pid
  | _ => 0.0 - 1.0

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

expect welcomePid(Welcome(3.0)) == 3.0

// The wrap helper is load-bearing for both roles — pin it.
expect wrap(halfW + 1.0, halfW) == 1.0 - halfW
expect wrap(0.0 - halfW - 1.0, halfW) == halfW - 1.0
expect wrap(3.0, halfW) == 3.0
expect wrapDelta(2.0 * halfW - 0.5, halfW) == 0.0 - 0.5

// ======================================================================
// ===========================  SERVER  =================================
// ======================================================================
// The authoritative asteroids simulation, as PURE step functions over the
// protocol types. There is no socket transport yet, but nothing here is
// speculative: the client below HOSTS this authoritative world in-process.
// It `join`s two pilots (you and the bot), folds their Steers through
// `recv` each tick, and calls `step` once per tick. When the netsim
// transport lands, the world moves behind the wire and `snapshot` is
// broadcast instead of read in-process — the functions do not change.

// ---------- tuning ----------
let turnSpeed = 3.4       // radians/second
let thrustAccel = 26.0    // units/second^2
let drag = 0.6            // fraction of velocity shed per second
let bulletSpeed = 34.0
let bulletLife = 0.9      // seconds
let fireCooldown = 0.22   // seconds between shots while fire is held

// ---------- entity steppers (the deterministic shared sim) ----------
// angle 0 points +y; positive angle turns counter-clockwise, so the nose
// direction is (-sin a, cos a) — the same convention Sprite.rotate uses.
let stepShip = (intent: Intent, dt: float, ship: Ship): Ship =>
  let angle = ship.angle + intent.turn * turnSpeed * dt in
  let acc = if intent.thrust then thrustAccel else 0.0 in
  // Floored at 0 so a pathological frame (dt > 1.6s) can't reverse velocity.
  let keep = Math.max(0.0, 1.0 - drag * dt) in
  let vx = (ship.vx - Math.sin(angle) * acc * dt) * keep in
  let vy = (ship.vy + Math.cos(angle) * acc * dt) * keep in
  { ship with
      x: wrap(ship.x + vx * dt, halfW),
      y: wrap(ship.y + vy * dt, halfH),
      angle: angle, vx: vx, vy: vy }

// A bullet leaves the ship's nose, inheriting the ship's velocity.
let fireBullet = (ship: Ship): Bullet =>
  { pid: ship.pid,
    x: ship.x - Math.sin(ship.angle) * 1.4,
    y: ship.y + Math.cos(ship.angle) * 1.4,
    vx: ship.vx - Math.sin(ship.angle) * bulletSpeed,
    vy: ship.vy + Math.cos(ship.angle) * bulletSpeed,
    ttl: bulletLife }

let stepBullet = (dt: float, b: Bullet): Bullet =>
  { b with x: wrap(b.x + b.vx * dt, halfW),
           y: wrap(b.y + b.vy * dt, halfH),
           ttl: b.ttl - dt }

let stepRock = (dt: float, r: Rock): Rock =>
  { r with x: wrap(r.x + r.vx * dt, halfW),
           y: wrap(r.y + r.vy * dt, halfH) }

// A destroyed rock (sizes 3/2) splits into two smaller children flung off
// the parent's course by opposite fixed kicks (deterministic — both ends
// must agree on the children without another random draw).
let childRock = (ang: float, r: Rock): Rock =>
  { r with vx: (r.vx * Math.cos(ang) - r.vy * Math.sin(ang)) * 1.3,
           vy: (r.vx * Math.sin(ang) + r.vy * Math.cos(ang)) * 1.3,
           size: r.size - 1.0 }

let splitRock = (r: Rock): List<Rock> =>
  if r.size <= 1.0 then [] else [childRock(0.8, r), childRock(0.0 - 0.8, r)]

// ---------- spawning ----------
// Rock n draws from its own forked stream, on a ring around the arena
// center (never on a center-spawned ship), drifting in a random direction.
let spawnRock = (seed: Random.Seed, n: float): Rock =>
  let s0 = seed |> Random.fork(n) in
  let (ang, s1) = Random.range(0.0, 6.28318, s0) in
  let (ring, s2) = Random.range(8.0, 13.0, s1) in
  let (dir, s3) = Random.range(0.0, 6.28318, s2) in
  let (speed, _) = Random.range(2.0, 4.5, s3) in
  { x: Math.sin(ang) * ring, y: Math.cos(ang) * ring,
    vx: Math.sin(dir) * speed, vy: Math.cos(dir) * speed, size: 3.0 }

let spawnWave = (seed: Random.Seed, count: float): List<Rock> =>
  List.range(count) |> List.map((n) => spawnRock(seed, n))

// ---------- collision predicates ----------
let bulletHitsRock = (r: Rock, b: Bullet): bool =>
  dist2(b.x, b.y, r.x, r.y) < radiusOf(r.size) * radiusOf(r.size)

let rockHitsShip = (ship: Ship, r: Rock): bool =>
  let reach = radiusOf(r.size) + shipRadius in
  dist2(r.x, r.y, ship.x, ship.y) < reach * reach

// ---------- bullet/rock resolution (ONE resolver for both roles) ----------
// One-to-one: each bullet is consumed by the FIRST rock it overlaps this
// tick, that rock is struck (it will split), and the SHOOTER — the pid the
// bullet carries — is credited that rock's points.
type Hits = { rocks: List<Rock>, struck: List<Rock>,
              bullets: List<Bullet>, credits: List<Score> }

// Drop the first element equal to `target` (structural equality — even
// co-located split children are distinct records, so exactly one matches).
let removeFirst = (target: 'a, xs: List<'a>): List<'a> =>
  let out =
    xs |> List.fold((acc, x) =>
      if not acc.found && x == target
      then { acc with found: true }
      else { acc with kept: [x, ..acc.kept] },
      { found: false, kept: [] }) in
  out.kept |> List.reverse

let resolveHits = (bullets: List<Bullet>, rocks: List<Rock>): Hits =>
  let claim = (acc: Hits, b: Bullet): Hits =>
    match acc.rocks |> List.find((r) => bulletHitsRock(r, b)) with
    | Option.None => { acc with bullets: [b, ..acc.bullets] }
    | Option.Some(r) =>
      { acc with rocks: removeFirst(r, acc.rocks),
                 struck: [r, ..acc.struck],
                 credits: [{ pid: b.pid, points: pointsFor(r.size) },
                           ..acc.credits] } in
  bullets |> List.fold(claim, { rocks: rocks, struck: [], bullets: [], credits: [] })

// ---------- the authoritative world ----------
type Pilot = { ship: Ship, intent: Intent, cool: float, points: float }
type World = { pilots: List<Pilot>, rocks: List<Rock>,
               bullets: List<Bullet>, seed: Random.Seed,
               nextPid: float, wave: float }

let coast: Intent = { turn: 0.0, thrust: false, fire: false }

let newWorld = (seed: Random.Seed): World =>
  { pilots: [], rocks: spawnWave(seed, 4.0), bullets: [],
    seed: seed, nextPid: 0.0, wave: 1.0 }

let newShip = (pid: float): Ship =>
  { pid: pid, x: 0.0, y: 0.0, angle: 0.0, vx: 0.0, vy: 0.0 }

// A client joined: allocate a pid and spawn its ship at the center.
// Returns the world plus the Welcome the transport should send back.
let join = (w: World): (World, Wire) =>
  let pid = w.nextPid in
  let p = { ship: newShip(pid), intent: coast, cool: 0.0, points: 0.0 } in
  ({ w with pilots: [p, ..w.pilots], nextPid: pid + 1.0 }, Welcome(pid))

// Look up a pilot by pid (the client reads its own — and the bot's — ship back).
let pilotOf = (pid: float, w: World): Option.t<Pilot> =>
  w.pilots |> List.find((p) => p.ship.pid == pid)

// Re-center a pilot's ship — the respawn after the client's ship is hit.
let respawn = (pid: float, w: World): World =>
  { w with pilots: w.pilots |> List.map((p) =>
      if p.ship.pid == pid then { p with ship: newShip(pid) } else p) }

// Fold one arriving client message into the world. `senderPid` is the pid
// of the CONNECTION the message arrived on — identity is never read from
// the wire value, so a client cannot steer another player's ship.
let recv = (senderPid: float, wire: Wire, w: World): World =>
  match wire with
  | Steer(intent) =>
      { w with pilots: w.pilots |> List.map((p) =>
          if p.ship.pid == senderPid then { p with intent: intent } else p) }
  | Join => w                  // handled by `join` (it returns the Welcome)
  | Welcome(_) => w            // server -> client only; nothing to fold
  | Snapshot(_, _, _, _) => w  // server -> client only; nothing to fold

// One authoritative tick: integrate ships — every pilot fires through the
// SAME held-fire cooldown (`fired` is an explicit bool) — move bullets and
// rocks, then resolve hits one-to-one with splitting, per-shooter scoring,
// and wave respawns. Ships are not destroyed here — a hit is the CLIENT's
// game-over in this sample, keeping the authoritative world simple.
let step = (dt: float, w: World): World =>
  let stepped =
    w.pilots |> List.map((p) =>
      let ship = stepShip(p.intent, dt, p.ship) in
      let firing = p.intent.fire && p.cool <= 0.0 in
      { pilot: { p with ship: ship,
                        cool: if firing then fireCooldown else p.cool - dt },
        fired: firing }) in
  let pilots = stepped |> List.map((s) => s.pilot) in
  let spawned =
    stepped |> List.filter((s) => s.fired) |> List.map((s) => fireBullet(s.pilot.ship)) in
  let bullets =
    w.bullets
      |> List.append(spawned)
      |> List.map((b) => stepBullet(dt, b))
      |> List.filter((b) => b.ttl > 0.0) in
  let rocks = w.rocks |> List.map((r) => stepRock(dt, r)) in
  let hits = resolveHits(bullets, rocks) in
  let scored =
    pilots |> List.map((p) =>
      let gained =
        hits.credits
          |> List.filter((c) => c.pid == p.ship.pid)
          |> List.fold((acc, c) => acc + c.points, 0.0) in
      { p with points: p.points + gained }) in
  let remaining = hits.rocks |> List.append(hits.struck |> List.concatMap(splitRock)) in
  let (_, nextSeed) = Random.step(w.seed) in
  if List.isEmpty(remaining) then
    let nextWave = w.wave + 1.0 in
    { w with pilots: scored, bullets: hits.bullets, wave: nextWave,
             seed: nextSeed, rocks: spawnWave(nextSeed, 3.0 + nextWave) }
  else
    { w with pilots: scored, bullets: hits.bullets, rocks: remaining }

// The full-state broadcast for this tick (naive: no deltas, no interest
// management — plenty for a sample-sized arena).
let snapshot = (w: World): Wire =>
  Snapshot(
    w.pilots |> List.map((p) => p.ship),
    w.rocks,
    w.bullets,
    w.pilots |> List.map((p) => { pid: p.ship.pid, points: p.points }))

// ---------- inline tests (run with `functor -d examples/asteroids-mp test`) ----------
expect List.length(splitRock({ x: 0.0, y: 0.0, vx: 1.0, vy: 0.0, size: 3.0 })) == 2.0
expect List.length(splitRock({ x: 0.0, y: 0.0, vx: 1.0, vy: 0.0, size: 1.0 })) == 0.0
expect (
  // A thrusting ship at angle 0 accelerates along +y.
  let s = stepShip({ turn: 0.0, thrust: true, fire: false }, 0.1, newShip(0.0)) in
  s.vy > 0.0 && Math.abs(s.vx) < 0.001
)
expect (
  // Join allocates ascending pids and keeps the ships.
  let (w1, _) = join(newWorld(Random.seed(1.0))) in
  let (w2, welcome) = join(w1) in
  List.length(w2.pilots) == 2.0
    && (match welcome with | Welcome(pid) => pid == 1.0 | _ => false)
)
expect (
  // A tick never loses a pilot, and rocks stay inside the wrapped arena.
  let (w1, _) = join(newWorld(Random.seed(2.0))) in
  let w2 = step(1.0 / tickHz, w1) in
  List.length(w2.pilots) == 1.0
    && (w2.rocks |> List.all((r) => Math.abs(r.x) <= halfW))
)
expect (
  // recv keys a Steer by the CONNECTION's pid — the wire value carries none,
  // so pilot 0's thrust changes and pilot 1's does not.
  let (w1, _) = join(newWorld(Random.seed(4.0))) in
  let (w2, _) = join(w1) in
  let w3 = w2 |> recv(0.0, Steer({ turn: 0.0, thrust: true, fire: false })) in
  w3.pilots |> List.all((p) =>
    if p.ship.pid == 0.0 then p.intent.thrust else not p.intent.thrust)
)
expect (
  // Held fire obeys the shared cooldown: one bullet immediately, none on
  // the very next tick, a second only after fireCooldown elapses.
  let (w1, _) = join(newWorld(Random.seed(3.0))) in
  let armed =
    { w1 with rocks: [{ x: 20.0, y: 12.0, vx: 0.0, vy: 0.0, size: 3.0 }],
              pilots: w1.pilots |> List.map((p) =>
                { p with intent: { turn: 0.0, thrust: false, fire: true } }) } in
  let dt = 1.0 / tickHz in
  let w2 = step(dt, armed) in
  let w3 = step(dt, w2) in
  let w4 = List.range(20.0) |> List.fold((w, n) => step(dt, w), w3) in
  List.length(w2.bullets) == 1.0
    && List.length(w3.bullets) == 1.0
    && List.length(w4.bullets) == 2.0
)
expect (
  // The resolver is one-to-one: one bullet through two overlapping rocks
  // strikes exactly one, is consumed, and credits only the shooter's pid.
  let r1 = { x: 0.0, y: 3.0, vx: 0.0, vy: 0.0, size: 1.0 } in
  let r2 = { x: 0.3, y: 3.0, vx: 0.0, vy: 0.0, size: 1.0 } in
  let b = { pid: 7.0, x: 0.0, y: 3.0, vx: 0.0, vy: 0.0, ttl: 0.5 } in
  let hits = resolveHits([b], [r1, r2]) in
  List.length(hits.struck) == 1.0
    && List.length(hits.rocks) == 1.0
    && List.length(hits.bullets) == 0.0
    && (match List.head(hits.credits) with
        | Option.Some(c) => c.pid == 7.0 && c.points == pointsFor(1.0)
        | Option.None => false)
)

// ======================================================================
// =============================  BOT  ==================================
// ======================================================================
// A tiny scripted pilot standing in for a remote peer. The bot owns no
// world state: each tick the client asks it for an Intent (the same plain
// record a keyboard produces) and folds it in as a `Steer` through `recv`,
// exactly as a remote player's packets will arrive once the netsim
// transport lands.
//
// The policy is deliberately tiny: point the nose at the nearest rock,
// thrust while it is far, hold fire while roughly aimed — the shared
// cooldown in `step` paces the actual shots.

let botIntent = (ship: Ship, rocks: List<Rock>): Intent =>
  let nearest =
    rocks
      |> List.sortBy((r) => dist2(ship.x, ship.y, r.x, r.y))
      |> List.head in
  match nearest with
  | Option.None => coast
  | Option.Some(r) =>
    let dx = wrapDelta(r.x - ship.x, halfW) in
    let dy = wrapDelta(r.y - ship.y, halfH) in
    // The nose at angle a points (-sin a, cos a), so the aim angle is
    // atan2(-dx, dy); steer by the wrapped angle difference.
    let want = Math.atan2(0.0 - dx, dy) in
    let diff = Math.mod(want - ship.angle + Math.pi, Math.pi * 2.0) - Math.pi in
    { turn: Math.sign(diff),
      thrust: dx * dx + dy * dy > 36.0,
      fire: Math.abs(diff) < 0.4 }

expect (
  // A rock dead ahead: no turn needed, fire held, close enough to coast.
  let rock = { x: 0.0, y: 6.0, vx: 0.0, vy: 0.0, size: 3.0 } in
  let i = botIntent(newShip(1.0), [rock]) in
  i.fire && i.turn == 0.0 && not i.thrust
)

// ======================================================================
// ===========================  CLIENT  =================================
// ======================================================================
// Input, the two joined pilots, prediction, and rendering.
//
// TWO pilots join the world through the real protocol path: YOU (cyan),
// and the bot (pink) standing in for a remote peer. Every tick each one's
// Intent is folded in as a `Steer` via `recv` — keyed by the sender's
// connection pid, exactly as arriving packets will be — then `step`
// advances the world once. When the netsim transport lands, the world
// moves behind the wire and the client keeps only its own Steer; nothing
// else changes shape.
//
// THE CONFIRMED GHOST: the faint outline trailing your ship is where the
// server's CONFIRMATION of your ship would currently be — your own ship,
// `rttTicks` (~133 ms) ago, read from a short ring buffer of past ship
// states. The solid ship is the optimistic prediction; the ghost is what
// the network would have acknowledged so far. Its "confirmed" label
// appears only while the two are visibly apart (accelerate or turn hard).

// ---------- palette ----------
// Zero-arg functions, not top-level values: engine constants (Color.*)
// only exist under the runner host, and the sandbox's single-buffer
// lang-intel EVALUATES this module under the plain no-engine prelude —
// a top-level `Color.rgb(...)` initializer would fail there. Wrapping
// keeps the draw path identical at one call's cost.
let cyan = () => Color.rgb(0.35, 0.95, 1.0)
let pink = () => Color.rgb(1.0, 0.35, 0.75)
let p2 = () => Color.rgb(0.91, 0.35, 0.72)    // player 2 — the site's --scrub-p2 pink (#e858b8)
let rockColor = () => Color.rgb(0.62, 0.58, 0.52)
let rockCore = () => Color.rgb(0.35, 0.32, 0.3)
let bulletColor = () => Color.rgb(1.0, 0.9, 0.35)
let starColor = () => Color.rgb(0.75, 0.8, 1.0)
let bg = () => Color.rgb(0.02, 0.025, 0.06)

// ---------- model ----------
type Phase =
  | Playing
  | GameOver

// The model: phase, the two pids this client knows (its own and the
// bot's), the authoritative World, MY held intent (sampled each tick),
// `history` — the ghost's ring buffer of my last rttTicks+1 ship states,
// newest first — plus lives, the respawn shield, and the run's seed.
let freshRun = (seed: Random.Seed) =>
  let (w1, meWelcome) = join(newWorld(seed)) in
  let (w2, botWelcome) = join(w1) in
  { phase: Playing,
    myPid: welcomePid(meWelcome),
    botPid: welcomePid(botWelcome),
    world: w2, intent: coast, history: [],
    lives: 3.0, shield: 2.0, seed: seed }

// A fixed seed keeps runs reproducible (swap in Effect.random for variety).
let init = freshRun(Random.seed(0.42))

// ---------- input ----------
// ALL controls are held LEVELS read from `sampledInput` once per tick —
// fire included: holding Space fires at the shared cooldown's rate, the
// same rule the bot (and any remote peer) plays under. This sampled
// record is precisely what a transport forwards as a `Steer`.
let holds = (k: Key.t, keys: List<Key.t>): bool =>
  keys |> List.any((h) => h == k)

let sampledInput = (m, snap: Input.snapshot) =>
  let keys = snap.heldKeys in
  let left = holds(Key.Left, keys) || holds(Key.A, keys) in
  let right = holds(Key.Right, keys) || holds(Key.D, keys) in
  let turn = (if left then 1.0 else 0.0) - (if right then 1.0 else 0.0) in
  { m with intent: { turn: turn,
                     thrust: holds(Key.Up, keys) || holds(Key.W, keys),
                     fire: holds(Key.Space, keys) } }

let input = (m, key: Key.t, isDown: bool) =>
  match m.phase with
  | Playing => m
  | GameOver =>
    (if isDown && (key == Key.R || key == Key.Enter)
     then freshRun(m.seed)
     else m)

// The bot's steering lives in the BOT section (`botIntent` — nearest rock,
// aim, thrust, hold fire); here it is just another peer whose Intent
// arrives through the same wire path as yours.
let botSteer = (m) =>
  match pilotOf(m.botPid, m.world) with
  | Option.None => coast
  | Option.Some(p) => botIntent(p.ship, m.world.rocks)

// ---------- simulation ----------
// One tick: both intents arrive as Steers keyed by their connection pid,
// the world advances once, and my new ship state feeds the ghost buffer.
let tickPlaying = (m, dt: float) =>
  let world =
    m.world
      |> recv(m.myPid, Steer(m.intent))
      |> recv(m.botPid, Steer(botSteer(m)))
      |> step(dt) in
  match pilotOf(m.myPid, world) with
  | Option.None => { m with world: world }    // unreachable: step never drops pilots
  | Option.Some(me) =>
    let history = pushHistory(me.ship, m.history) in
    let shield = Math.max(0.0, m.shield - dt) in
    let hit =
      shield == 0.0
        && (world.rocks |> List.any((r) => rockHitsShip(me.ship, r))) in
    if not hit then
      { m with world: world, history: history, shield: shield }
    else if m.lives <= 1.0 then
      // Out of ships. My pilot coasts (and stops rendering); the bot keeps
      // clearing rocks behind the game-over card.
      { m with phase: GameOver, lives: 0.0, history: [],
               world: world |> recv(m.myPid, Steer(coast)) }
    else
      // Hit: lose a life; the "server" re-centers my ship behind a fresh
      // shield, and the ghost buffer restarts with it.
      { m with world: respawn(m.myPid, world), history: [],
               lives: m.lives - 1.0, shield: 2.5 }

let tick = (m, dt: float, tts: float) =>
  match m.phase with
  | Playing => tickPlaying(m, dt)
  | GameOver =>
    // The world keeps running behind the card — the bot is still a live
    // peer, steering and firing through the same wire path.
    { m with world: m.world
                      |> recv(m.botPid, Steer(botSteer(m)))
                      |> step(dt) }

// ---------- wrap-aware placement ----------
// Collision (dist2) is wrap-aware, so rendering must be too: an entity
// within its radius of an arena edge is drawn AGAIN on the opposite side
// (the arcade standard) — nothing can hit you from off-screen.
let wrapOffsets = (rad: float, v: float, limit: float): List<float> =>
  if v > limit - rad then [0.0, 0.0 - limit * 2.0]
  else if v < rad - limit then [0.0, limit * 2.0]
  else [0.0]

let wrapPlace = (rad: float, x: float, y: float, sprite: Sprite.t): List<Sprite.t> =>
  wrapOffsets(rad, x, halfW) |> List.concatMap((ox) =>
    wrapOffsets(rad, y, halfH) |> List.map((oy) =>
      sprite |> Sprite.move(x + ox, y + oy)))

// ---------- rendering ----------
// The ship triangle, nose up (+y at angle 0). Sprite.polygon points are
// used verbatim, so building it around the origin lets Sprite.rotate/move
// place it — the same convention stepShip integrates in.
let nose = { x: 0.0, y: 1.4 }
let tailL = { x: -0.9, y: -1.0 }
let tailR = { x: 0.9, y: -1.0 }

let shipBody = (color: Color.t, thrusting: bool, tts: float) =>
  let flame =
    if thrusting && Math.sin(tts * 30.0) > -0.4 then
      [Sprite.polygon(pink(), [{ x: -0.4, y: -1.0 }, { x: 0.4, y: -1.0 },
                               { x: 0.0, y: -1.9 }])]
    else [] in
  Sprite.group([Sprite.polygon(color, [nose, tailL, tailR]), ..flame])

// My ship blinks behind the respawn shield (and hides on game over); the
// bot draws whenever alive. 1.9 covers the triangle plus its flame, so a
// wrap copy appears as soon as any part of a ship crosses an edge.
let shipReach = 1.9

let shipSprites = (m, tts: float) =>
  let showMine =
    (match m.phase with
     | GameOver => false
     | Playing => not (m.shield > 0.001 && Math.sin(tts * 18.0) <= 0.0)) in
  m.world.pilots |> List.concatMap((p) =>
    let mine = p.ship.pid == m.myPid in
    if mine && not showMine then []
    else
      shipBody((if mine then cyan() else p2()), p.intent.thrust, tts)
        |> Sprite.rotate(Angle.radians(p.ship.angle))
        |> wrapPlace(shipReach, p.ship.x, p.ship.y))

// THE CONFIRMED GHOST: the same triangle as an outline (three lines —
// there is no unfilled polygon), always faint, drawn at the ring buffer's
// OLDEST entry: where the server's confirmation of my ship would currently
// be, rttTicks behind the prediction you steer. The "confirmed" label only
// appears while the ghost is visibly separated from the predicted ship, so
// it never reads as the ship's nametag.
let ghostOutline = () =>
  Sprite.group([
    Sprite.line(cyan(), 0.12, nose, tailL),
    Sprite.line(cyan(), 0.12, tailL, tailR),
    Sprite.line(cyan(), 0.12, tailR, nose),
  ])

let ghostSprites = (m) =>
  match m.phase with
  | GameOver => []    // no prediction to confirm — the buffer was cleared
  | Playing =>
    (match List.nth(rttTicks, m.history) with
     | Option.None => []    // buffer still filling (right after a respawn)
     | Option.Some(g) =>
       let separation =
         (match List.head(m.history) with
          | Option.None => 0.0
          | Option.Some(now) => Math.sqrt(dist2(now.x, now.y, g.x, g.y))) in
       let label =
         if separation > 0.8 then
           [Sprite.text(cyan(), 0.7, "confirmed") |> Sprite.move(g.x, g.y - 2.2)]
         else [] in
       [Sprite.group([
          ghostOutline()
            |> Sprite.rotate(Angle.radians(g.angle))
            |> Sprite.move(g.x, g.y),
          ..label]) |> Sprite.fade(0.25)])

// Rocks are filled circles with a darker core so the size classes read.
let rockSprites = (r) =>
  let rad = radiusOf(r.size) in
  Sprite.group([
    Sprite.circle(rockColor(), rad),
    Sprite.circle(rockCore(), rad * 0.55),
  ]) |> wrapPlace(rad, r.x, r.y)

let bulletSprites = (b) =>
  Sprite.circle(bulletColor(), 0.18) |> wrapPlace(0.18, b.x, b.y)

// A fixed backdrop of faint stars, each drawn from its own forked stream.
// (A function, not a top-level value — it builds Sprites, and the seed
// alone stays a plain top-level number.)
let starSeed = Random.seed(9.1)
let stars = () =>
  List.range(40.0) |> List.map((n) =>
    let (x01, s1) = Random.step(starSeed |> Random.fork(n)) in
    let (y01, s2) = Random.step(s1) in
    let (tw, _) = Random.step(s2) in
    Sprite.circle(starColor(), 0.06 + tw * 0.07)
      |> Sprite.fade(0.25 + tw * 0.5)
      |> Sprite.move((x01 * 2.0 - 1.0) * halfW,
                     (y01 * 2.0 - 1.0) * halfH))

// HUD text, left-aligned via Sprite.measure (text is centered on its box,
// so the left edge lands at x + width/2).
let hudLine = (color: Color.t, y: float, s: string) =>
  let w = Sprite.measure(1.1, s).width in
  Sprite.text(color, 1.1, s)
    |> Sprite.move(1.0 - halfW + w * 0.5, y)

let hud = (m) =>
  let myPoints =
    (match pilotOf(m.myPid, m.world) with
     | Option.None => 0.0
     | Option.Some(p) => p.points) in
  Sprite.group([
    hudLine(bulletColor(), halfH - 1.2,
            Text.concat("SCORE ", Text.fixed(myPoints, 0.0))),
    hudLine(cyan(), halfH - 2.6,
            Text.concat("SHIPS ", Text.fixed(m.lives, 0.0))),
  ])

let gameOverCard = () =>
  Sprite.group([
    Sprite.text(pink(), 2.4, "GAME OVER") |> Sprite.move(0.0, 1.5),
    Sprite.text(cyan(), 1.0, "PRESS R TO RESTART") |> Sprite.move(0.0, -1.0),
  ])

let draw = (m, tts: float) =>
  let card = (match m.phase with | GameOver => [gameOverCard()] | Playing => []) in
  Frame.create2D(
    Camera2D.create(halfW * 2.0, halfH * 2.0),
    Sprite.group(
      stars()
        |> List.append(m.world.rocks |> List.concatMap(rockSprites))
        |> List.append(m.world.bullets |> List.concatMap(bulletSprites))
        |> List.append(ghostSprites(m))          // ghost under the live ships
        |> List.append(shipSprites(m, tts))
        |> List.append([hud(m)])
        |> List.append(card)))
    |> Frame.withClearColor(bg())

// The pure logic above lives beside its `expect` tests — run
// `functor -d examples/asteroids-mp test`.
