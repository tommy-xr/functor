// game.fun — Orbs: the smallest multiplayer sample.
//   functor -d examples/orbs run native
// Left/Right (A/D) turn, Up (W) thrusts, HOLD SPACE over a glowing orb to
// claim it: it takes your color, and your score is the orbs you own. A full
// board deals a fresh round. The client HOSTS the authoritative world
// in-process — when the netsim transport lands, the SERVER section moves
// behind the wire (a functor.json `entries` role, see examples/mp) unchanged.

// ==========================  PROTOCOL  ================================
// What both roles agree on — with a real transport, exactly what
// `Effect.sendMsg` carries (no string codec; a typo is a check-time error).

let halfW = 13.0          // arena half-extents (world units)
let halfH = 8.0

type Ship = { pid: float, x: float, y: float, rot: float }
type Orb = { id: float, x: float, y: float, owner: float }   // owner: -1 = unclaimed
type Intent = { turn: float, thrust: bool, claim: bool }     // held controls, per tick

// The wire, one variant per role in the loop:
//   Join           client -> server, once: opens the handshake
//   Welcome(pid)   server -> client: your identity (the handshake's reply)
//   Steer(intent)  client -> server, per tick: held controls. Carries NO pid —
//                  the server keys it by the connection it arrived on (recv's
//                  senderPid), so you can only ever steer yourself.
//   Claim(orbId)   client -> server: "I am over orb N with claim held" — a
//                  REQUEST, not a fact; the server re-checks and resolves it.
//   Snapshot(...)  server -> client, per tick: the authoritative state.
type Wire =
  | Join
  | Welcome(pid: float)
  | Steer(intent: Intent)
  | Claim(orbId: float)
  | Snapshot(ships: List<Ship>, orbs: List<Orb>)

let welcomePid = (wire: Wire): float =>   // the one message a joiner decodes
  match wire with
  | Welcome(pid) => pid
  | _ => 0.0 - 1.0
let dist2 = (ax: float, ay: float, bx: float, by: float): float =>
  (ax - bx) * (ax - bx) + (ay - by) * (ay - by)

// Claim reach — the client's proposal gate; the server re-runs the same test.
let claimRange = 1.8
let overOrb = (ship: Ship, o: Orb): bool =>
  dist2(ship.x, ship.y, o.x, o.y) < claimRange * claimRange

// ===========================  SERVER  =================================
// PURE functions over the protocol types: `join` admits a connection,
// `recv` folds one message in, `step` advances a tick and settles claims.

let turnSpeed = 3.2       // radians/second
let moveSpeed = 9.0       // units/second while thrusting

type Pilot = { ship: Ship, intent: Intent }
type PendingClaim = { pid: float, orbId: float }
type World = { pilots: List<Pilot>, orbs: List<Orb>,
               claims: List<PendingClaim>, nextPid: float }

let coast: Intent = { turn: 0.0, thrust: false, claim: false }

// Five orbs at fixed spots (deterministic — nothing to sync at startup).
let initialOrbs: List<Orb> = [
  { id: 0.0, x: 0.0, y: 0.0, owner: -1.0 }, { id: 1.0, x: -10.0, y: 4.0, owner: -1.0 },
  { id: 2.0, x: 10.0, y: 4.0, owner: -1.0 }, { id: 3.0, x: -6.0, y: -5.0, owner: -1.0 },
  { id: 4.0, x: 6.0, y: -5.0, owner: -1.0 }]

let newWorld = (): World => { pilots: [], orbs: initialOrbs, claims: [], nextPid: 0.0 }
let newShip = (pid: float): Ship =>   // one spawn slot per pid (this sample seats two)
  { pid: pid, x: -11.0 + pid * 22.0, y: 0.0, rot: 0.0 }
let join = (w: World): (World, Wire) =>   // allocate a pid, spawn, answer Welcome
  let pid = w.nextPid in
  let p = { ship: newShip(pid), intent: coast } in
  ({ w with pilots: [p, ..w.pilots], nextPid: pid + 1.0 }, Welcome(pid))
let shipOf = (pid: float, w: World): Option.t<Ship> =>
  w.pilots |> List.find((p) => p.ship.pid == pid) |> Option.map((p) => p.ship)

// Fold one arriving message in, keyed by the CONNECTION it arrived on —
// identity is never read from the wire value.
let recv = (senderPid: float, wire: Wire, w: World): World =>
  match wire with
  | Steer(intent) =>
      { w with pilots: w.pilots |> List.map((p) =>
          if p.ship.pid == senderPid then { p with intent: intent } else p) }
  | Claim(orbId) =>       // queue it; `step` resolves the tick's claims at once
      { w with claims: [{ pid: senderPid, orbId: orbId }, ..w.claims] }
  | Join => w             // handled by `join` (it returns the Welcome)
  | Welcome(_) => w       // server -> client only; nothing to fold
  | Snapshot(_, _) => w   // server -> client only; nothing to fold

// rot 0 points +y, nose (-sin, cos) — Sprite.rotate's convention; the arena clamps.
let stepShip = (intent: Intent, dt: float, ship: Ship): Ship =>
  let rot = ship.rot + intent.turn * turnSpeed * dt in
  let speed = if intent.thrust then moveSpeed else 0.0 in
  { ship with rot: rot,
      x: Math.clamp(0.0 - halfW, halfW, ship.x - Math.sin(rot) * speed * dt),
      y: Math.clamp(0.0 - halfH, halfH, ship.y + Math.cos(rot) * speed * dt) }

// ---------- claim resolution: THE authoritative-server lesson ----------
// Two clients can each honestly believe they grabbed the same orb on the
// same tick — each saw it unclaimed on its own screen. The SERVER breaks
// the tie with one rule, in one place: the orb must still be unclaimed
// this round, the claimant must actually be in range (never trust the
// client), and of the valid claimants this tick the CLOSEST ship wins.
// Clients learn the outcome from the next Snapshot — no client override.
let resolveOrb = (w: World, o: Orb): Orb =>
  if o.owner >= 0.0 then o
  else
    let winner =
      w.claims
        |> List.filter((c) => c.orbId == o.id)
        |> List.concatMap((c) =>
            match shipOf(c.pid, w) with
            | Option.Some(s) =>
              if overOrb(s, o) then [{ pid: c.pid, d: dist2(s.x, s.y, o.x, o.y) }] else []
            | Option.None => [])
        |> List.sortBy((c) => c.pid)   // exact-tie fallback: the LOWER pid —
        |> List.sortBy((c) => c.d)     // stable sorts keep the rule total
        |> List.head in
    match winner with
    | Option.Some(c) => { o with owner: c.pid }
    | Option.None => o

// One tick: integrate ships, settle claims against the moved positions, clear
// the queue. A full board deals a fresh round, so the arena never goes dead.
let step = (dt: float, w: World): World =>
  let moved =
    { w with pilots: w.pilots |> List.map((p) =>
        { p with ship: stepShip(p.intent, dt, p.ship) }) } in
  let orbs = moved.orbs |> List.map((o) => resolveOrb(moved, o)) in
  let full = orbs |> List.all((o) => o.owner >= 0.0) in
  { moved with claims: [], orbs: if full then initialOrbs else orbs }

// Your score IS the orbs you own this round — nothing stored can drift.
let scoreOf = (pid: float, orbs: List<Orb>): float =>
  orbs |> List.filter((o) => o.owner == pid) |> List.length

// =============================  BOT  ==================================
// A remote peer owning NO world state: the client asks it for an Intent each
// tick and folds it through the SAME Steer/Claim path real packets will take.

let botIntent = (ship: Ship, orbs: List<Orb>): Intent =>
  let target =
    orbs
      |> List.filter((o) => o.owner < 0.0)
      |> List.sortBy((o) => dist2(ship.x, ship.y, o.x, o.y))
      |> List.head in
  match target with
  | Option.None => coast    // nothing unclaimed right now
  | Option.Some(o) =>
    // Nose at rot a points (-sin a, cos a), so the aim angle is atan2(-dx, dy).
    let want = Math.atan2(ship.x - o.x, o.y - ship.y) in
    let diff = Math.mod(want - ship.rot + Math.pi, Math.pi * 2.0) - Math.pi in
    let arrived = overOrb(ship, o) in
    { turn: if arrived then 0.0 else Math.sign(diff),
      thrust: not arrived, claim: arrived }

// ===========================  CLIENT  =================================
// YOU (cyan) and the bot (pink) join through the real protocol path; both
// Intents fold in as Steers + Claims, then `step` runs. With a real
// transport the client keeps only its OWN sends; nothing changes shape.

// Palette: zero-arg functions, not top-level values — the sandbox's lang-intel
// evaluates this module under the plain prelude, where Color.* doesn't exist.
let cyan = () => Color.rgb(0.255, 0.847, 0.902)   // me — the site's #41d8e6
let pink = () => Color.rgb(0.91, 0.345, 0.72)     // bot — the site's #e858b8
let orbFree = () => Color.rgb(0.85, 0.88, 0.95)   // unclaimed: dim white glow
let wallColor = () => Color.rgb(0.3, 0.35, 0.55)
let bg = () => Color.rgb(0.02, 0.025, 0.06)

let init =
  let (w1, meWelcome) = join(newWorld()) in
  let (w2, botWelcome) = join(w1) in
  { myPid: welcomePid(meWelcome), botPid: welcomePid(botWelcome),
    world: w2, intent: coast }

// Held LEVELS, read once per tick — exactly what a transport forwards as a Steer.
let sampledInput = (m, snap: Input.snapshot) =>
  let held = (k: Key.t) => snap.heldKeys |> List.any((h) => h == k) in
  { m with intent: {
      turn: (if held(Key.Left) || held(Key.A) then 1.0 else 0.0)
          - (if held(Key.Right) || held(Key.D) then 1.0 else 0.0),
      thrust: held(Key.Up) || held(Key.W),
      claim: held(Key.Space) } }

// Over an unclaimed orb with claim held -> a Claim on the wire; the server decides.
let sendClaim = (pid: float, intent: Intent, w: World): World =>
  if not intent.claim then w
  else
    match shipOf(pid, w) with
    | Option.None => w
    | Option.Some(s) =>
      (match w.orbs |> List.find((o) => o.owner < 0.0 && overOrb(s, o)) with
       | Option.Some(o) => w |> recv(pid, Claim(o.id))
       | Option.None => w)

let tick = (m, dt: float, tts: float) =>
  let bot =
    (match shipOf(m.botPid, m.world) with
     | Option.None => coast
     | Option.Some(s) => botIntent(s, m.world.orbs)) in
  { m with world:
      m.world
        |> recv(m.myPid, Steer(m.intent))
        |> recv(m.botPid, Steer(bot))
        |> sendClaim(m.myPid, m.intent)
        |> sendClaim(m.botPid, bot)
        |> step(dt) }

// ---------- rendering ----------
let colorFor = (m, pid: float) => if pid == m.myPid then cyan() else pink()

// Unclaimed orbs glow dim white; a claimed orb burns its owner's color.
let orbSprite = (m, o: Orb) =>
  let claimed = o.owner >= 0.0 in
  let c = if claimed then colorFor(m, o.owner) else orbFree() in
  Sprite.group([
    Sprite.circle(c, 1.5) |> Sprite.fade(if claimed then 0.3 else 0.16),
    Sprite.circle(c, 0.6) |> Sprite.fade(if claimed then 1.0 else 0.7),
  ]) |> Sprite.move(o.x, o.y)

let shipSprite = (m, p: Pilot) =>
  Sprite.polygon(colorFor(m, p.ship.pid),
                 [{ x: 0.0, y: 1.2 }, { x: -0.8, y: -0.9 }, { x: 0.8, y: -0.9 }])
    |> Sprite.rotate(Angle.radians(p.ship.rot))
    |> Sprite.move(p.ship.x, p.ship.y)

let hud = (m) =>
  Sprite.group([
    Text.concat("YOU ", Text.fixed(scoreOf(m.myPid, m.world.orbs), 0.0))
      |> Sprite.text(cyan(), 1.2) |> Sprite.move(0.0 - halfW * 0.5, halfH + 2.2),
    Text.concat("BOT ", Text.fixed(scoreOf(m.botPid, m.world.orbs), 0.0))
      |> Sprite.text(pink(), 1.2) |> Sprite.move(halfW * 0.5, halfH + 2.2)])

let draw = (m, tts: float) =>
  Frame.create2D(
    Camera2D.create((halfW + 2.0) * 2.0, (halfH + 3.0) * 2.0),
    Sprite.group(
      [Sprite.rectangle(wallColor(), halfW * 2.0 + 2.6, halfH * 2.0 + 2.6),
       Sprite.rectangle(bg(), halfW * 2.0 + 2.0, halfH * 2.0 + 2.0)]
        |> List.append(m.world.orbs |> List.map((o) => orbSprite(m, o)))
        |> List.append(m.world.pilots |> List.map((p) => shipSprite(m, p)))
        |> List.append([hud(m)])))
    |> Frame.withClearColor(bg())

// ========================  SERVER ROLE  ===============================
// The same file is ALSO the `server` entry: functor.json maps the role to
// { "file": "game.fun", "prefix": "server" }, so the runner resolves
// serverInit/serverTick/serverDraw here (functor -d examples/orbs run
// native --entry server) — one buffer, both roles, one hot reload. The
// role is the authoritative SERVER section stepped directly, with the bot
// steering both seats so the view moves on its own.

let serverInit = init

let serverTick = (m, dt: float, tts: float) =>
  let steer = (pid: float, w: World): World =>
    (match shipOf(pid, w) with
     | Option.None => w
     | Option.Some(s) =>
       let i = botIntent(s, w.orbs) in
       w |> recv(pid, Steer(i)) |> sendClaim(pid, i)) in
  { m with world: m.world |> steer(m.myPid) |> steer(m.botPid) |> step(dt) }

let serverDraw = draw   // the same board, seen from the authority
