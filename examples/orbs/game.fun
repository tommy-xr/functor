// game.fun — Orbs: the smallest multiplayer sample, over a REAL wire.
//   functor -d examples/orbs run native --entry server   # the authority
//   functor -d examples/orbs run native                  # a pilot dials it
//   functor -d examples/orbs run native                  # …and a second: the rival
// Left/Right (A/D) turn, Up (W) thrusts, HOLD SPACE over a glowing orb to
// claim it: it takes your color, and your score is the orbs you own. A full
// board deals a fresh round. The server owns the world; a client sends what
// it is doing and draws the snapshots it gets back.

// ==========================  PROTOCOL  ================================
// What both roles agree on — exactly what `Effect.sendMsg` carries (no string
// codec; a typo is a check-time error). One file, so ONE declaration: the two
// ends cannot drift apart.

let bind = "127.0.0.1:9101"                  // the server's listen address…
let serverUrl = "ws://127.0.0.1:9101/orbs"   // …and the string a client dials

let halfW = 13.0          // arena half-extents (world units)
let halfH = 8.0

type Ship = { pid: float, x: float, y: float, rot: float }
type Orb = { id: float, x: float, y: float, owner: float }   // owner: -1 = unclaimed
type Intent = { turn: float, thrust: bool, claim: bool }     // held controls, per tick

// The wire, one variant per role in the loop:
//   Join           client -> server, once: opens the handshake
//   Welcome(pid)   server -> client: your identity (the handshake's reply)
//   Steer(intent)  client -> server, per tick: held controls. Carries NO pid —
//                  the server keys it by the connection it arrived on, so you
//                  can only ever steer yourself.
//   Claim(orbId)   client -> server: "I am over orb N with claim held" — a
//                  REQUEST, not a fact; the server re-checks and resolves it.
//   Snapshot(...)  server -> client, per tick: the authoritative state.
type Wire =
  | Join
  | Welcome(pid: float)
  | Steer(intent: Intent)
  | Claim(orbId: float)
  | Snapshot(ships: List<Ship>, orbs: List<Orb>)

// Both ends see the same socket events; only what they DO with them differs.
type Msg =
  | Opened(id: float)               // the socket is up
  | Packet(id: float, wire: Wire)   // a Wire value arrived, already decoded
  | Closed(id: float)               // …and gone for good
  | Noise(why: string)              // a bad frame or a hiccup — NOT a disconnect

let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => Opened(id)
  | Net.Data(id, wire) => Packet(id, wire)
  | Net.Disconnected(id) => Closed(id)
  | Net.Error(_, why) => Noise(why)
  | Net.Message(_, _) => Noise("unexpected text frame")   // this wire is typed

let dist2 = (ax: float, ay: float, bx: float, by: float): float =>
  (ax - bx) * (ax - bx) + (ay - by) * (ay - by)

// Claim reach — the client's proposal gate; the server re-runs the same test.
let claimRange = 1.8
let overOrb = (ship: Ship, o: Orb): bool =>
  dist2(ship.x, ship.y, o.x, o.y) < claimRange * claimRange

// ===========================  SERVER  =================================
// PURE functions over the protocol types: `join` seats a pilot, `recv` folds
// one arriving message in, `step` advances a tick and settles claims. The
// role that puts them behind a socket is at the bottom of the file.

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
let newShip = (pid: float): Ship =>   // spawn slots along the arena, wrapping every four
  { pid: pid, x: 0.0 - 11.0 + Math.mod(pid, 4.0) * 7.3, y: 0.0, rot: 0.0 }
let join = (w: World): (World, float) =>   // allocate a pid and spawn its ship
  let pid = w.nextPid in
  let p = { ship: newShip(pid), intent: coast } in
  ({ w with pilots: [p, ..w.pilots], nextPid: pid + 1.0 }, pid)
let leave = (pid: float, w: World): World =>   // a departure releases its orbs
  { w with pilots: w.pilots |> List.filter((p) => not p.ship.pid == pid),
      orbs: w.orbs |> List.map((o) =>
        if o.owner == pid then { o with owner: 0.0 - 1.0 } else o) }
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
  | Join => w             // the handshake, answered where seats are handed out
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

// ===========================  CLIENT  =================================
// The `client` role is this file's ordinary top-level contract
// (init/subscriptions/update/sampledInput/tick/draw). It holds NO authority:
// it dials the server, says what it is doing, and draws the last Snapshot it
// was sent. Your pid arrives in the Welcome — identity comes from the server.

// Palette: zero-arg functions, not top-level values — the sandbox's lang-intel
// evaluates this module under the plain prelude, where Color.* doesn't exist.
let cyan = () => Color.rgb(0.255, 0.847, 0.902)   // the site's #41d8e6
let pink = () => Color.rgb(0.91, 0.345, 0.72)     // the site's #e858b8
let orbFree = () => Color.rgb(0.85, 0.88, 0.95)   // unclaimed: dim white glow
let wallColor = () => Color.rgb(0.3, 0.35, 0.55)
let bg = () => Color.rgb(0.02, 0.025, 0.06)

// Color carries IDENTITY, not "me": a pid looks the same in every pane.
let inkFor = (pid: float) => if Math.mod(pid, 2.0) == 0.0 then cyan() else pink()

type Client = { conn: Option.t<float>, myPid: float,
                ships: List<Ship>, orbs: List<Orb>, intent: Intent }

let init: Client = { conn: Option.None, myPid: 0.0 - 1.0,
                     ships: [], orbs: [], intent: coast }

// Declaring the connection in `subscriptions` keeps the socket open.
let subscriptions = (m: Client) => Sub.connect(serverUrl, toMsg)

let update = (m: Client, msg: Msg) =>
  match msg with
  | Opened(id) => ({ m with conn: Option.Some(id) }, Effect.sendMsg(id, Join))
  | Packet(_, wire) =>
      (match wire with
       | Welcome(pid) => { m with myPid: pid }
       | Snapshot(ships, orbs) => { m with ships: ships, orbs: orbs }
       | _ => m)          // Join/Steer/Claim are client -> server only
  | Closed(_) => { m with conn: Option.None, ships: [], orbs: [] }
  | Noise(_) => m       // a hiccup is not a disconnect: keep playing

// Held LEVELS, read once per tick — exactly what the Steer forwards.
let sampledInput = (m: Client, snap: Input.snapshot) =>
  let held = (k: Key.t) => snap.heldKeys |> List.any((h) => h == k) in
  { m with intent: {
      turn: (if held(Key.Left) || held(Key.A) then 1.0 else 0.0)
          - (if held(Key.Right) || held(Key.D) then 1.0 else 0.0),
      thrust: held(Key.Up) || held(Key.W),
      claim: held(Key.Space) } }

// One burst per tick — and nothing at all before the server has seated us.
// `Steer` reports the input we are holding right now; a `Claim` rides along
// only when the last Snapshot we were sent puts us over a free orb. Both are
// PROPOSALS — neither is a fact until the server re-checks it.
let tick = (m: Client, dt: float, tts: float) =>
  match m.conn with
  | Option.None => m
  | Option.Some(id) =>
    (match m.ships |> List.find((s) => s.pid == m.myPid) with
     | Option.None => m
     | Option.Some(s) =>
       let claim =
         if not m.intent.claim then []
         else (match m.orbs |> List.find((o) => o.owner < 0.0 && overOrb(s, o)) with
               | Option.Some(o) => [Claim(o.id)]
               | Option.None => []) in
       (m, Effect.batch([Steer(m.intent), ..claim]
                          |> List.map((w) => Effect.sendMsg(id, w)))))

// ---------- rendering ----------
// Unclaimed orbs glow dim white; a claimed orb burns its owner's color.
let orbSprite = (o: Orb) =>
  let claimed = o.owner >= 0.0 in
  let c = if claimed then inkFor(o.owner) else orbFree() in
  Sprite.group([
    Sprite.circle(c, 1.5) |> Sprite.fade(if claimed then 0.3 else 0.16),
    Sprite.circle(c, 0.6) |> Sprite.fade(if claimed then 1.0 else 0.7),
  ]) |> Sprite.move(o.x, o.y)

let shipSprite = (me: float, s: Ship) =>
  Sprite.polygon(inkFor(s.pid),
                 [{ x: 0.0, y: 1.2 }, { x: -0.8, y: -0.9 }, { x: 0.8, y: -0.9 }])
    |> Sprite.fade(if s.pid == me then 1.0 else 0.6)   // yours is the bright one
    |> Sprite.rotate(Angle.radians(s.rot))
    |> Sprite.move(s.x, s.y)

// One score per seated pilot, over a status line that stays honest about who is
// actually in the arena. An EMPTY board means something different to each role,
// so each supplies its own `alone` line; one lonely ship reads the same either
// way — you can still fly and claim uncontested, but it takes two to race.
let hud = (alone: string, ships: List<Ship>, orbs: List<Orb>) =>
  let scores = ships |> List.map((s) =>
    $"P{s.pid} {scoreOf(s.pid, orbs)}"
      |> Sprite.text(inkFor(s.pid), 1.0)
      |> Sprite.move(0.0 - 10.5 + Math.mod(s.pid, 4.0) * 7.0, halfH + 2.2)) in
  let waiting =
    if List.length(ships) >= 2.0 then []
    else [(if List.length(ships) == 0.0 then alone else "waiting for a rival...")
            |> Sprite.text(orbFree(), 1.0)
            |> Sprite.move(0.0, 0.0 - halfH - 2.2)] in
  Sprite.group(scores |> List.append(waiting))

// The board both roles render — from ships and orbs, which is all a Snapshot
// carries and all the authority needs to show.
let board = (me: float, alone: string, ships: List<Ship>, orbs: List<Orb>) =>
  Frame.create2D(
    Camera2D.create((halfW + 2.0) * 2.0, (halfH + 3.0) * 2.0),
    Sprite.group(
      [Sprite.rectangle(wallColor(), halfW * 2.0 + 2.6, halfH * 2.0 + 2.6),
       Sprite.rectangle(bg(), halfW * 2.0 + 2.0, halfH * 2.0 + 2.0)]
        |> List.append(orbs |> List.map(orbSprite))
        |> List.append(ships |> List.map((s) => shipSprite(me, s)))
        |> List.append([hud(alone, ships, orbs)])))
    |> Frame.withClearColor(bg())

let draw = (m: Client, tts: float) =>
  board(m.myPid, "waiting for the server...", m.ships, m.orbs)

// ========================  SERVER ROLE  ===============================
// The same file is ALSO the `server` entry: functor.json maps the role to
// { "file": "game.fun", "prefix": "server" }, so the runner resolves this
// section's bindings as the contract — serverInit/serverTick/serverDraw/…
// (functor -d examples/orbs run native --entry server). One buffer, both
// roles, one hot reload.

type Seat = { cid: float, pid: float }   // connection -> the pilot it steers

// An empty arena: every pilot arrives over a wire, so the board starts with
// five unclaimed orbs and nobody in it.
let serverInit = { world: newWorld(), seats: [] }

// Declaring the listener in `subscriptions` keeps the server bound.
let serverSubscriptions = (m) => Sub.listen(bind, toMsg)

let seatPid = (cid: float, m): Option.t<float> =>
  m.seats |> List.find((s) => s.cid == cid) |> Option.map((s) => s.pid)

let serverUpdate = (m, msg: Msg) =>
  match msg with
  | Opened(_) => m        // a socket, not yet a pilot: the Join seats it
  | Packet(cid, wire) =>
      (match wire with
       // The handshake: seat a pilot for this connection, answer its identity.
       // A re-Join is answered from the seat it already has — one connection
       // is one pilot, however many times it asks.
       | Join =>
         (match seatPid(cid, m) with
          | Option.Some(pid) => (m, Effect.sendMsg(cid, Welcome(pid)))
          | Option.None =>
            let (w, pid) = join(m.world) in
            ({ m with world: w, seats: [{ cid: cid, pid: pid }, ..m.seats] },
             Effect.sendMsg(cid, Welcome(pid))))
       // Everything else is keyed by the connection, never by the wire value.
       | _ =>
         (match seatPid(cid, m) with
          | Option.Some(pid) => { m with world: m.world |> recv(pid, wire) }
          | Option.None => m))
  | Noise(_) => m
  | Closed(cid) =>
      (match seatPid(cid, m) with
       | Option.Some(pid) => { m with world: m.world |> leave(pid),
                               seats: m.seats |> List.filter((s) => not s.cid == cid) }
       | Option.None => m)

let serverTick = (m, dt: float, tts: float) =>
  let stepped = m.world |> step(dt) in
  let snap = Snapshot(stepped.pilots |> List.map((p) => p.ship), stepped.orbs) in
  ({ m with world: stepped },
   Effect.batch(m.seats |> List.map((s) => Effect.sendMsg(s.cid, snap))))

let serverDraw = (m, tts: float) =>   // the authority holds no seat of its own
  board(0.0 - 1.0, "waiting for a pilot...",
        m.world.pilots |> List.map((p) => p.ship), m.world.orbs)
