// server.fun — the authoritative multiplayer server:
//
//   functor -d examples/mp run --entry server native
//
// Tracks a player per connection, integrates their movement, and broadcasts
// the whole world to every client each tick. Naive (full-state, no delta, no
// prediction) — enough to prove the loop and to drive deterministically
// through the netsim. The wire format, palette, and scene live in the shared
// Protocol / View siblings (file = module), so the roles cannot drift.
//
// `Sub.listen(bind, toMsg)` binds the address; each accepted client surfaces
// through the `toMsg` closure carrying its own connection id, which we reply
// to with `Effect.send`.

type Player = { cid: float, pid: float, x: float, z: float, vx: float, vz: float }

type Model = { players: List<Player>, nextPid: float }

type Msg =
  | Joined(cid: float)
  | Moved(cid: float, text: string)
  | Left(cid: float)
  | NetErr(text: string)

let speed = 2.0

// NetEvent -> a game-specific Msg (a closure tagger).
let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(cid) => Joined(cid)
  | Net.Message(cid, text) => Moved(cid, text)
  | Net.Disconnected(cid) => Left(cid)
  | Net.Error(cid, message) => NetErr(message)

// Integrate one axis a frame, wrapping around the arena edges
// (asteroids-style) so entities stay in a fixed playfield instead of
// drifting off-screen.
let wrapAxis = (pos: float, vel: float, dt: float): float =>
  let p = pos + vel * speed * dt in
  if p > Protocol.arena then p - 2.0 * Protocol.arena
  else if p < 0.0 - Protocol.arena then p + 2.0 * Protocol.arena
  else p

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
                 if p.cid == cid then { p with vx: vx, vz: vz } else p) }
       | _ => m)
  | Left(cid) =>
      { m with players: m.players |> List.filter((p) => not p.cid == cid) }
  | NetErr(_) => m

// Declaring the listener in `subscriptions` keeps the server bound.
let subscriptions = (m: Model) => Sub.listen(Protocol.bind, toMsg)

let tick = (m: Model, dt: float, tts: float) =>
  // Integrate, then broadcast the snapshot to every client.
  let players =
    m.players |> List.map((p) =>
      { p with x: wrapAxis(p.x, p.vx, dt), z: wrapAxis(p.z, p.vz, dt) }) in
  let snapshot = Protocol.encode(players |> List.map((p) => Protocol.row(p.pid, p.x, p.z))) in
  let sends = players |> List.map((p) => Effect.send(p.cid, snapshot)) in
  ({ m with players: players }, Effect.batch(sends))

let draw = (m: Model, tts: float) =>
  // The authoritative world, through the same shared view as the clients.
  View.world(m.players |> List.map((p) => Protocol.row(p.pid, p.x, p.z)))
