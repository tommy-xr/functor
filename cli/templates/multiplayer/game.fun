// An authoritative multiplayer starter: move your square, the SERVER owns the
// world. Start the authority, then dial it with one client per player:
//   functor -d . run native --entry server
//   functor -d . run native                  # a player (repeat for a second)
// Arrows or WASD move. Saving this file hot-reloads BOTH roles at once.
//
// Both roles are inline modules of this ONE file — `module Client` and
// `module Server` at the bottom; each block's members ARE that role's contract
// (`Client.init`/`Client.tick`/…), and functor.json maps the roles to them.
// Everything above the blocks is shared: the protocol is declared exactly once,
// so the two ends cannot drift apart.

// ============================  PROTOCOL  ==============================

let bind = "127.0.0.1:9300"                  // the server's listen address…
let serverUrl = "ws://127.0.0.1:9300/game"   // …and the string a client dials

let half = 8.0          // arena half-extent (world units)
let moveSpeed = 7.0     // units/second

type Player = { pid: float, x: float, y: float }
type Intent = { dx: float, dy: float }       // held controls, per tick

// The wire. `Effect.sendMsg` carries these values structurally — no string
// codec, and a typo is a check-time error.
//   Join            client -> server, once: opens the handshake
//   Welcome(pid)    server -> client: your identity
//   Steer(intent)   client -> server, per tick: held controls. Carries NO pid —
//                   the server keys it by the connection it arrived on.
//   Snapshot(...)   server -> client, per tick: the authoritative state
type Wire =
  | Join
  | Welcome(pid: float)
  | Steer(intent: Intent)
  | Snapshot(players: List<Player>)

// Both ends see the same socket events; only what they DO with them differs.
type Msg =
  | Opened(id: float)
  | Packet(id: float, wire: Wire)
  | Closed(id: float)
  | Noise(why: string)          // a bad frame or a hiccup — NOT a disconnect

let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => Opened(id)
  | Net.Data(id, wire) => Packet(id, wire)
  | Net.Disconnected(id) => Closed(id)
  | Net.Error(_, why) => Noise(why)
  | Net.Message(_, _) => Noise("unexpected text frame")   // this wire is typed

// =========================  THE WORLD (shared)  =======================
// PURE functions over the protocol types. `module Server` is the role that
// puts them behind a socket.

type Pilot = { player: Player, intent: Intent }
type World = { pilots: List<Pilot>, nextPid: float }

let still: Intent = { dx: 0.0, dy: 0.0 }
let newWorld = (): World => { pilots: [], nextPid: 0.0 }

let join = (w: World): (World, float) =>      // allocate a pid and spawn it
  let pid = w.nextPid in
  let spawn = { pid: pid, x: 0.0 - 6.0 + Math.mod(pid, 4.0) * 4.0, y: 0.0 } in
  ({ pilots: [{ player: spawn, intent: still }, ..w.pilots], nextPid: pid + 1.0 }, pid)

let leave = (pid: float, w: World): World =>
  { w with pilots: w.pilots |> List.filter((p: Pilot) => not (p.player.pid == pid)) }

// Fold one arriving message in, keyed by the CONNECTION it arrived on —
// identity is never read from the wire value, so nobody can steer a rival.
let recv = (senderPid: float, wire: Wire, w: World): World =>
  match wire with
  | Steer(intent) =>
      { w with pilots: w.pilots |> List.map((p: Pilot) =>
          if p.player.pid == senderPid then { p with intent: intent } else p) }
  | _ => w              // Join/Welcome/Snapshot: nothing to fold here

let stepPlayer = (i: Intent, dt: float, p: Player): Player =>
  { p with x: Math.clamp(0.0 - half, half, p.x + i.dx * moveSpeed * dt),
           y: Math.clamp(0.0 - half, half, p.y + i.dy * moveSpeed * dt) }

let step = (dt: float, w: World): World =>
  { w with pilots: w.pilots |> List.map((p: Pilot) =>
      { p with player: stepPlayer(p.intent, dt, p.player) }) }

// ======================  PRESENTATION (shared)  =======================
// Both roles draw the same board — the client from the last Snapshot it was
// sent, the authority from the world it owns. Color carries IDENTITY, so a
// player looks the same in every window.

let ink = (pid: float) =>
  if Math.mod(pid, 2.0) == 0.0 then Color.rgb(0.25, 0.85, 0.9)
  else Color.rgb(0.91, 0.35, 0.72)

let board = (me: float, status: string, players: List<Player>) =>
  Frame.create2D(
    Camera2D.create(half * 2.0 + 4.0, half * 2.0 + 4.0),
    Sprite.group(
      [Sprite.rectangle(Color.rgb(0.12, 0.14, 0.25), half * 2.0, half * 2.0),
       status |> Sprite.text(Color.rgb(0.75, 0.8, 0.95), 0.8)
              |> Sprite.move(0.0, half + 1.2)]
        |> List.append(players |> List.map((p: Player) =>
             Sprite.rectangle(ink(p.pid), 1.2, 1.2)
               |> Sprite.fade(if p.pid == me then 1.0 else 0.55)   // yours is bright
               |> Sprite.move(p.x, p.y)))))
    |> Frame.withClearColor(Color.rgb(0.03, 0.03, 0.08))

// =============================  CLIENT  ===============================
// Holds NO authority: it dials the server, says what it is doing, and draws
// the last Snapshot it was sent. Your pid arrives in the Welcome.

module Client {
  type Model = { conn: Option.t<float>, myPid: float,
                 players: List<Player>, intent: Intent }

  let init: Model = { conn: Option.None, myPid: 0.0 - 1.0,
                      players: [], intent: still }

  // Declaring the connection in `subscriptions` keeps the socket open.
  let subscriptions = (m: Model) => Sub.connect(serverUrl, toMsg)

  let update = (m: Model, msg: Msg) =>
    match msg with
    | Opened(id) => ({ m with conn: Option.Some(id) }, Effect.sendMsg(id, Join))
    | Packet(_, wire) =>
        (match wire with
         | Welcome(pid) => { m with myPid: pid }
         | Snapshot(players) => { m with players: players }
         | _ => m)                     // Join/Steer are client -> server only
    | Closed(_) => { m with conn: Option.None, players: [] }
    | Noise(_) => m                    // a hiccup is not a disconnect

  // Held LEVELS, read once per tick — exactly what the Steer forwards.
  let sampledInput = (m: Model, snap: Input.snapshot) =>
    let held = (k: Key.t) => snap.heldKeys |> List.any((h: Key.t) => h == k) in
    { m with intent: {
        dx: (if held(Key.Right) || held(Key.D) then 1.0 else 0.0)
          - (if held(Key.Left) || held(Key.A) then 1.0 else 0.0),
        dy: (if held(Key.Up) || held(Key.W) then 1.0 else 0.0)
          - (if held(Key.Down) || held(Key.S) then 1.0 else 0.0) } }

  // One burst per tick — a PROPOSAL, not a fact: the server decides where you
  // actually end up, and the next Snapshot says so.
  let tick = (m: Model, dt: float, tts: float) =>
    match m.conn with
    | Option.None => m
    | Option.Some(id) => (m, Effect.sendMsg(id, Steer(m.intent)))

  let draw = (m: Model, tts: float) =>
    match m.conn with
    | Option.None => board(m.myPid, "connecting to the server...", m.players)
    | Option.Some(_) => board(m.myPid, "arrows / WASD to move", m.players)
}

// =============================  SERVER  ===============================
// The same file's other block, run with `--entry server`. It owns the world:
// every client sends intent, the server steps and broadcasts the truth.

module Server {
  type Seat = { cid: float, pid: float }   // connection -> the player it steers

  let init = { world: newWorld(), seats: [] }

  // Declaring the listener in `subscriptions` keeps the server bound.
  let subscriptions = (m) => Sub.listen(bind, toMsg)

  let seatPid = (cid: float, m): Option.t<float> =>
    m.seats |> List.find((s: Seat) => s.cid == cid) |> Option.map((s: Seat) => s.pid)

  let update = (m, msg: Msg) =>
    match msg with
    | Opened(_) => m       // a socket, not yet a player: the Join seats it
    | Packet(cid, wire) =>
        (match wire with
         | Join =>          // the handshake: seat a player, answer its identity
           (match seatPid(cid, m) with
            | Option.Some(pid) => (m, Effect.sendMsg(cid, Welcome(pid)))
            | Option.None =>
              let (w, pid) = join(m.world) in
              ({ m with world: w, seats: [{ cid: cid, pid: pid }, ..m.seats] },
               Effect.sendMsg(cid, Welcome(pid))))
         | _ =>             // keyed by the connection, never by the wire value
           (match seatPid(cid, m) with
            | Option.Some(pid) => { m with world: m.world |> recv(pid, wire) }
            | Option.None => m))
    | Closed(cid) =>
        (match seatPid(cid, m) with
         | Option.Some(pid) =>
           { m with world: m.world |> leave(pid),
                    seats: m.seats |> List.filter((s: Seat) => not (s.cid == cid)) }
         | Option.None => m)
    | Noise(_) => m

  let tick = (m, dt: float, tts: float) =>
    let stepped = m.world |> step(dt) in
    let snap = Snapshot(stepped.pilots |> List.map((p: Pilot) => p.player)) in
    ({ m with world: stepped },
     Effect.batch(m.seats |> List.map((s: Seat) => Effect.sendMsg(s.cid, snap))))

  let draw = (m, tts: float) =>   // the authority holds no seat of its own
    board(0.0 - 1.0, "server - the authority",
          m.world.pilots |> List.map((p: Pilot) => p.player))
}
