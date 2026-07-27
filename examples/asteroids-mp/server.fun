// server.fun — the authoritative asteroids simulation, as PURE step
// functions over the Protocol types.
//
// There is no transport yet: nothing here opens a socket or defines a game
// entry point. A future netsim transport will hold a `World`, fold arriving
// `Protocol.Wire` values through `recv`, call `step` once per tick, and
// broadcast `snapshot` to every client. Until then this file loads as an
// ordinary sibling module (`file = module`) — and the client ALREADY runs
// the entity steppers below as its local prediction, so by construction the
// predicted sim and the authoritative sim are the same functions.

// ---------- tuning ----------
let turnSpeed = 3.4       // radians/second
let thrustAccel = 26.0    // units/second^2
let drag = 0.6            // fraction of velocity shed per second
let bulletSpeed = 34.0
let bulletLife = 0.9      // seconds
let fireCooldown = 0.22   // seconds between autonomous shots (held fire)

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

// Fold one arriving client message into the world.
let recv = (wire: Protocol.Wire, w: World): World =>
  match wire with
  | Protocol.Steer(pid, intent) =>
      { w with pilots: w.pilots |> List.map((p) =>
          if p.ship.pid == pid then { p with intent: intent } else p) }
  // Join is handled by `join` (it needs to return the Welcome);
  // Welcome/Snapshot are server->client only.
  | _ => w

// One authoritative tick: integrate ships (spawning bullets on held fire),
// move bullets and rocks, resolve bullet/rock hits with splitting + scoring.
// Ships are not destroyed here — a hit is the CLIENT's game-over in this
// sample, keeping the authoritative world simple.
let step = (dt: float, w: World): World =>
  let pilots =
    w.pilots |> List.map((p) =>
      let ship = stepShip(p.intent, dt, p.ship) in
      let firing = p.intent.fire && p.cool <= 0.0 in
      { p with ship: ship, cool: if firing then fireCooldown else p.cool - dt }) in
  let spawned =
    pilots
      |> List.filter((p) => p.intent.fire && p.cool == fireCooldown)
      |> List.map((p) => fireBullet(p.ship)) in
  let bullets =
    w.bullets
      |> List.append(spawned)
      |> List.map((b) => stepBullet(dt, b))
      |> List.filter((b) => b.ttl > 0.0) in
  let rocks = w.rocks |> List.map((r) => stepRock(dt, r)) in
  let hitR = (r: Protocol.Rock) => bullets |> List.any((b) => bulletHitsRock(r, b)) in
  let struck = rocks |> List.filter(hitR) in
  let kept = rocks |> List.filter((r) => not hitR(r)) in
  let survivors =
    bullets |> List.filter((b) =>
      not (rocks |> List.any((r) => bulletHitsRock(r, b)))) in
  let gained = struck |> List.fold((acc, r) => acc + Protocol.pointsFor(r.size), 0.0) in
  let scored =
    // Naive: every pilot whose bullet is in flight shares the credit; a real
    // server would track which bullet struck (Bullet carries pid for that).
    pilots |> List.map((p) => { p with points: p.points + gained }) in
  let remaining = kept |> List.append(struck |> List.concatMap(splitRock)) in
  let (_, nextSeed) = Random.step(w.seed) in
  if List.isEmpty(remaining) then
    { w with pilots: scored, bullets: survivors, wave: w.wave + 1.0,
             seed: nextSeed,
             rocks: spawnWave(nextSeed, 3.0 + w.wave) }
  else
    { w with pilots: scored, bullets: survivors, rocks: remaining }

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
