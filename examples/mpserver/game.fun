// mpserver — the Functor Lang port of examples/mpserver (roadmap E2, docs/functor-lang.md).
//
// Authoritative multiplayer server: tracks a player per connection, integrates
// their movement, and broadcasts the whole world to every client each tick.
// Naive (full-state, no delta, no prediction) — enough to prove the loop and to
// drive deterministically through the netsim. Pairs with mpclient.
//
//   functor -d examples/mpserver run native
//
// Notes on the port from F#:
//   - `Sub.listen(bind, toMsg)` binds the address; each accepted client
//     surfaces through the `toMsg` closure carrying its own connection id,
//     which we reply to with `Effect.send`.
//   - The wire snapshot uses fixed-point integers (`x * 100`). `Text.fixed(n,
//     0.0)` ROUNDS (F# `int(...)` truncates), so an encoded coordinate can
//     differ from F# by <=0.01 world units — sub-visual. (F# integrates in
//     float32, Functor Lang in f64, so positions agree to well within that 0.01 wire
//     unit rather than being bit-identical.)
//   - Functor Lang has no `%` or `<>`, so `mod4` (bounded recursion) picks the color
//     and a bool-literal match expresses "not equal".

type Player = { cid: float, pid: float, x: float, z: float, vx: float, vz: float }

type Model = { players: List<Player>, nextPid: float }

type Msg =
  | Joined(cid: float)
  | Moved(cid: float, text: string)
  | Left(cid: float)
  | NetErr(text: string)

let bind = "127.0.0.1:9001"
let speed = 2.0
let arena = 4.0

// NetEvent -> a game-specific Msg (a closure tagger, per the F# original).
let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(cid) => Joined(cid)
  | Net.Message(cid, text) => Moved(cid, text)
  | Net.Disconnected(cid) => Left(cid)
  | Net.Error(cid, message) => NetErr(message)

// Integrate one axis a frame, wrapping around the arena edges (asteroids-style)
// so entities stay in a fixed playfield instead of drifting off-screen.
let wrapAxis = (pos: float, vel: float, dt: float): float =>
  let p = pos + vel * speed * dt in
  match p > arena with
  | true => p - 2.0 * arena
  | false =>
    (match p < -arena with
     | true => p + 2.0 * arena
     | false => p)

// Wire format: "pid,x*100,z*100|pid,x*100,z*100|...".
let encodePlayer = (p: Player): string =>
  Text.join(
    ",",
    [Text.fixed(p.pid, 0.0), Text.fixed(p.x * 100.0, 0.0), Text.fixed(p.z * 100.0, 0.0)])

let encode = (players: List<Player>): string =>
  Text.join("|", List.map(encodePlayer, players))

let init = { players: [], nextPid: 0.0 }

let update = (m: Model, msg: Msg): Model =>
  match msg with
  | Joined(cid) =>
      // Spawn the new player on its own z-lane so players don't overlap.
      let p = { cid: cid, pid: m.nextPid, x: -2.0, z: m.nextPid * 1.8 - 1.8, vx: 0.0, vz: 0.0 } in
      { m with players: [p, ..m.players], nextPid: m.nextPid + 1.0 }
  | Moved(cid, text) =>
      // "vx vz" -> set that client's velocity; a malformed packet is ignored.
      (match Text.split(" ", text) with
       | [vxs, vzs] =>
           let vx = Text.parseFloat(vxs) in
           let vz = Text.parseFloat(vzs) in
           { m with players:
               m.players |> List.map((p) =>
                 match p.cid == cid with
                 | true => { p with vx: vx, vz: vz }
                 | false => p) }
       | _ => m)
  | Left(cid) =>
      { m with players:
          m.players |> List.filter((p) =>
            match p.cid == cid with
            | true => false
            | false => true) }
  | NetErr(_) => m

// Declaring the listener in `subscriptions` keeps the server bound.
let subscriptions = (m: Model) => Sub.listen(bind, toMsg)

let tick = (m: Model, dt: float, tts: float) =>
  // Integrate, then broadcast the snapshot to every client.
  let players =
    m.players |> List.map((p) =>
      { p with x: wrapAxis(p.x, p.vx, dt), z: wrapAxis(p.z, p.vz, dt) }) in
  let snapshot = encode(players) in
  let sends = players |> List.map((p) => Effect.send(p.cid, snapshot)) in
  ({ m with players: players }, Effect.batch(sends))

// A distinct color per player id, shared with the client so a given player is
// the same color in every pane. Functor Lang has no `%`, so mod4 wraps by recursion —
// bounded by the demo's small player count (the z-lane spawn runs players off
// the arena past a handful, so pid stays small).
let mod4 = (n: float): float =>
  match n < 4.0 with
  | true => n
  | false => mod4(n - 4.0)

let colorFor = (pid: float): (float, float, float) =>
  match mod4(pid) with
  | 0.0 => (0.90, 0.35, 0.35)
  | 1.0 => (0.35, 0.60, 0.95)
  | 2.0 => (0.45, 0.85, 0.45)
  | _ => (0.95, 0.80, 0.35)

let draw = (m: Model, tts: float) =>
  // One colored cube per player at its authoritative position, on a ground
  // plane sized to the wrap boundary (so the playfield edges are visible).
  let playerNodes =
    m.players |> List.map((p) =>
      let (r, g, b) = colorFor(p.pid) in
      Scene.cube()
      |> Scene.scale(0.6)
      |> Scene.translate(p.x, 0.0, p.z)
      |> Scene.lit(Color.rgb(r, g, b))) in
  let ground =
    Scene.plane() |> Scene.scale(2.0 * arena) |> Scene.lit(Color.rgb(0.18, 0.2, 0.28)) in
  let scene = Scene.group([ground, ..playerNodes]) in
  // Top-down-ish view so player movement stays on screen.
  let camera =
    Camera.firstPerson(
      0.0, 9.0, -2.0,
      Angle.radians(0.0), Angle.radians(-1.2), Angle.degrees(70.0)) in
  Frame.createLit(
    camera, scene,
    [ Light.ambient(Color.rgb(0.35, 0.35, 0.42)),
      Light.directional(-0.4, -1.0, -0.35, Color.rgb(1.0, 0.95, 0.85), 1.1) ])
