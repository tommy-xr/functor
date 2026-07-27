// server.fun — the authoritative asteroids simulation, as PURE step
// functions over the Protocol types.
//
// There is no socket transport yet, but nothing here is speculative: the
// client HOSTS this authoritative world in-process. It `join`s two pilots
// (you and the bot), folds their Steers through `recv` each tick, and calls
// `step` once per tick. When the netsim transport lands, the world moves
// behind the wire and `snapshot` is broadcast instead of read in-process —
// the functions themselves do not change.

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
let stepShip = (intent: Protocol.Intent, dt: float, ship: Protocol.Ship): Protocol.Ship =>
  let angle = ship.angle + intent.turn * turnSpeed * dt in
  let acc = if intent.thrust then thrustAccel else 0.0 in
  // Floored at 0 so a pathological frame (dt > 1.6s) can't reverse velocity.
  let keep = Math.max(0.0, 1.0 - drag * dt) in
  let vx = (ship.vx - Math.sin(angle) * acc * dt) * keep in
  let vy = (ship.vy + Math.cos(angle) * acc * dt) * keep in
  { ship with
      x: Protocol.wrap(ship.x + vx * dt, Protocol.halfW),
      y: Protocol.wrap(ship.y + vy * dt, Protocol.halfH),
      angle: angle, vx: vx, vy: vy }

// A bullet leaves the ship's nose, inheriting the ship's velocity.
let fireBullet = (ship: Protocol.Ship): Protocol.Bullet =>
  { pid: ship.pid,
    x: ship.x - Math.sin(ship.angle) * 1.4,
    y: ship.y + Math.cos(ship.angle) * 1.4,
    vx: ship.vx - Math.sin(ship.angle) * bulletSpeed,
    vy: ship.vy + Math.cos(ship.angle) * bulletSpeed,
    ttl: bulletLife }

let stepBullet = (dt: float, b: Protocol.Bullet): Protocol.Bullet =>
  { b with x: Protocol.wrap(b.x + b.vx * dt, Protocol.halfW),
           y: Protocol.wrap(b.y + b.vy * dt, Protocol.halfH),
           ttl: b.ttl - dt }

let stepRock = (dt: float, r: Protocol.Rock): Protocol.Rock =>
  { r with x: Protocol.wrap(r.x + r.vx * dt, Protocol.halfW),
           y: Protocol.wrap(r.y + r.vy * dt, Protocol.halfH) }

// A destroyed rock (sizes 3/2) splits into two smaller children flung off
// the parent's course by opposite fixed kicks (deterministic — both ends
// must agree on the children without another random draw).
let childRock = (ang: float, r: Protocol.Rock): Protocol.Rock =>
  { r with vx: (r.vx * Math.cos(ang) - r.vy * Math.sin(ang)) * 1.3,
           vy: (r.vx * Math.sin(ang) + r.vy * Math.cos(ang)) * 1.3,
           size: r.size - 1.0 }

let splitRock = (r: Protocol.Rock): List<Protocol.Rock> =>
  if r.size <= 1.0 then [] else [childRock(0.8, r), childRock(0.0 - 0.8, r)]

// ---------- spawning ----------
// Rock n draws from its own forked stream, on a ring around the arena
// center (never on a center-spawned ship), drifting in a random direction.
let spawnRock = (seed: Random.Seed, n: float): Protocol.Rock =>
  let s0 = seed |> Random.fork(n) in
  let (ang, s1) = Random.range(0.0, 6.28318, s0) in
  let (ring, s2) = Random.range(8.0, 13.0, s1) in
  let (dir, s3) = Random.range(0.0, 6.28318, s2) in
  let (speed, _) = Random.range(2.0, 4.5, s3) in
  { x: Math.sin(ang) * ring, y: Math.cos(ang) * ring,
    vx: Math.sin(dir) * speed, vy: Math.cos(dir) * speed, size: 3.0 }

let spawnWave = (seed: Random.Seed, count: float): List<Protocol.Rock> =>
  List.range(count) |> List.map((n) => spawnRock(seed, n))

// ---------- collision predicates ----------
let bulletHitsRock = (r: Protocol.Rock, b: Protocol.Bullet): bool =>
  Protocol.dist2(b.x, b.y, r.x, r.y)
    < Protocol.radiusOf(r.size) * Protocol.radiusOf(r.size)

let rockHitsShip = (ship: Protocol.Ship, r: Protocol.Rock): bool =>
  let reach = Protocol.radiusOf(r.size) + Protocol.shipRadius in
  Protocol.dist2(r.x, r.y, ship.x, ship.y) < reach * reach

// ---------- bullet/rock resolution (ONE resolver for both roles) ----------
// One-to-one: each bullet is consumed by the FIRST rock it overlaps this
// tick, that rock is struck (it will split), and the SHOOTER — the pid the
// bullet carries — is credited that rock's points.
type Hits = { rocks: List<Protocol.Rock>, struck: List<Protocol.Rock>,
              bullets: List<Protocol.Bullet>, credits: List<Protocol.Score> }

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

let resolveHits = (bullets: List<Protocol.Bullet>, rocks: List<Protocol.Rock>): Hits =>
  let claim = (acc: Hits, b: Protocol.Bullet): Hits =>
    match acc.rocks |> List.find((r) => bulletHitsRock(r, b)) with
    | Option.None => { acc with bullets: [b, ..acc.bullets] }
    | Option.Some(r) =>
      { acc with rocks: removeFirst(r, acc.rocks),
                 struck: [r, ..acc.struck],
                 credits: [{ pid: b.pid, points: Protocol.pointsFor(r.size) },
                           ..acc.credits] } in
  bullets |> List.fold(claim, { rocks: rocks, struck: [], bullets: [], credits: [] })

// ---------- the authoritative world ----------
type Pilot = { ship: Protocol.Ship, intent: Protocol.Intent, cool: float, points: float }
type World = { pilots: List<Pilot>, rocks: List<Protocol.Rock>,
               bullets: List<Protocol.Bullet>, seed: Random.Seed,
               nextPid: float, wave: float }

let coast: Protocol.Intent = { turn: 0.0, thrust: false, fire: false }

let newWorld = (seed: Random.Seed): World =>
  { pilots: [], rocks: spawnWave(seed, 4.0), bullets: [],
    seed: seed, nextPid: 0.0, wave: 1.0 }

let newShip = (pid: float): Protocol.Ship =>
  { pid: pid, x: 0.0, y: 0.0, angle: 0.0, vx: 0.0, vy: 0.0 }

// A client joined: allocate a pid and spawn its ship at the center.
// Returns the world plus the Welcome the transport should send back.
let join = (w: World): (World, Protocol.Wire) =>
  let pid = w.nextPid in
  let p = { ship: newShip(pid), intent: coast, cool: 0.0, points: 0.0 } in
  ({ w with pilots: [p, ..w.pilots], nextPid: pid + 1.0 }, Protocol.Welcome(pid))

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
let recv = (senderPid: float, wire: Protocol.Wire, w: World): World =>
  match wire with
  | Protocol.Steer(intent) =>
      { w with pilots: w.pilots |> List.map((p) =>
          if p.ship.pid == senderPid then { p with intent: intent } else p) }
  | Protocol.Join => w                  // handled by `join` (it returns the Welcome)
  | Protocol.Welcome(_) => w            // server -> client only; nothing to fold
  | Protocol.Snapshot(_, _, _, _) => w  // server -> client only; nothing to fold

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
let snapshot = (w: World): Protocol.Wire =>
  Protocol.Snapshot(
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
    && (match welcome with | Protocol.Welcome(pid) => pid == 1.0 | _ => false)
)
expect (
  // A tick never loses a pilot, and rocks stay inside the wrapped arena.
  let (w1, _) = join(newWorld(Random.seed(2.0))) in
  let w2 = step(1.0 / Protocol.tickHz, w1) in
  List.length(w2.pilots) == 1.0
    && (w2.rocks |> List.all((r) => Math.abs(r.x) <= Protocol.halfW))
)
expect (
  // recv keys a Steer by the CONNECTION's pid — the wire value carries none,
  // so pilot 0's thrust changes and pilot 1's does not.
  let (w1, _) = join(newWorld(Random.seed(4.0))) in
  let (w2, _) = join(w1) in
  let w3 = w2 |> recv(0.0, Protocol.Steer({ turn: 0.0, thrust: true, fire: false })) in
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
  let dt = 1.0 / Protocol.tickHz in
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
        | Option.Some(c) => c.pid == 7.0 && c.points == Protocol.pointsFor(1.0)
        | Option.None => false)
)
