// client.fun — the multiplayer client (`client` is the default entry):
//
//   functor -d examples/mp run native
//
// Connects to server.fun, sends its movement as typed `Protocol.Move` values,
// and renders the `Protocol.Snapshot` rows the server broadcasts — the wire
// is the shared `Protocol.Wire` ADT (file = module), sent with
// `Effect.sendMsg` and received as `Net.Data`, so there is no parsing and a
// protocol typo is a check-time error. The client holds NO authoritative
// state — it just draws what the server last sent (naive, so movement lags by
// the round trip). WASD drives it live; on connect it auto-moves +x so a
// headless test produces motion with no input injection.

type ConnState =
  | Online(id: float)
  | Offline

type Model = { conn: ConnState, world: List<Protocol.Row>, status: string }

type Msg =
  | Joined(id: float)
  | Packet(id: float, wire: Protocol.Wire)
  | Dropped
  | ConnErr(text: string)

// NetEvent -> a game-specific Msg (a closure tagger).
let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => Joined(id)
  | Net.Data(id, wire) => Packet(id, wire)
  // The mp protocol is typed; plain text is not part of it.
  | Net.Message(_, _) => ConnErr("unexpected text message")
  | Net.Disconnected(_) => Dropped
  | Net.Error(_, message) => ConnErr(message)

let init = { conn: Offline, world: [], status: "connecting" }

let update = (m: Model, msg: Msg) =>
  match msg with
  // On connect, auto-move +x so a headless test produces motion with no input.
  | Joined(id) => ({ m with conn: Online(id), status: "connected" },
                   Effect.sendMsg(id, Protocol.Move(1.0, 0.0)))
  | Packet(_, wire) =>
      // A Snapshot replaces the drawn world; any other wire value from the
      // server is ignored (a Move is client->server only).
      (match wire with
       | Protocol.Snapshot(rows) => { m with world: rows, status: "in-world" }
       | _ => m)
  | Dropped => { m with conn: Offline, status: "disconnected" }
  | ConnErr(message) => { m with status: Text.concat("error: ", message) }

// Declaring the connection in `subscriptions` keeps it open.
let subscriptions = (m: Model) => Sub.connect(Protocol.serverUrl, toMsg)

let tick = (m: Model, dt: float, tts: float) => m

// WASD -> a velocity for the server; other keys map to the empty list.
let velocityFor = (key: Key.t): List<float> =>
  match key with
  | Key.W => [0.0, 1.0]
  | Key.S => [0.0, -1.0]
  | Key.A => [-1.0, 0.0]
  | Key.D => [1.0, 0.0]
  | _ => []

// Key down sends the direction; key release sends a stop. Both need the live
// connection id, so nothing is sent until the socket has opened.
let input = (m: Model, key: Key.t, isDown: bool) =>
  match m.conn with
  | Offline => m
  | Online(id) =>
    (match isDown with
     | true =>
       (match velocityFor(key) with
        | [vx, vz] => (m, Effect.sendMsg(id, Protocol.Move(vx, vz)))
        | _ => m)
     | false => (m, Effect.sendMsg(id, Protocol.Move(0.0, 0.0))))

let draw = (m: Model, tts: float) => View.world(m.world)
