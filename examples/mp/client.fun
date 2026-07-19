// client.fun — the multiplayer client (`client` is the default entry):
//
//   functor -d examples/mp run native
//
// Connects to server.fun, sends its movement input, and renders the world
// snapshots the server broadcasts. It holds NO authoritative state — it just
// draws what the server last sent (naive, so movement lags by the round
// trip). WASD drives it live; on connect it auto-moves +x so a headless test
// produces motion with no input injection. The wire format, palette, and
// scene live in the shared Protocol / View siblings (file = module).

type ConnState =
  | Online(id: float)
  | Offline

type Model = { conn: ConnState, world: List<Protocol.Row>, status: string }

type Msg =
  | Joined(id: float)
  | Snapshot(text: string)
  | Dropped
  | ConnErr(text: string)

// NetEvent -> a game-specific Msg (a closure tagger).
let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => Joined(id)
  | Net.Message(_, text) => Snapshot(text)
  // The mp wire is the string snapshot; typed payloads are not part of it.
  | Net.Data(_, _) => ConnErr("unexpected typed message")
  | Net.Disconnected(_) => Dropped
  | Net.Error(_, message) => ConnErr(message)

let init = { conn: Offline, world: [], status: "connecting" }

let update = (m: Model, msg: Msg) =>
  match msg with
  // On connect, auto-move +x so a headless test produces motion with no input.
  | Joined(id) => ({ m with conn: Online(id), status: "connected" }, Effect.send(id, "1 0"))
  | Snapshot(text) => { m with world: Protocol.decode(text), status: "in-world" }
  | Dropped => { m with conn: Offline, status: "disconnected" }
  | ConnErr(message) => { m with status: Text.concat("error: ", message) }

// Declaring the connection in `subscriptions` keeps it open.
let subscriptions = (m: Model) => Sub.connect(Protocol.serverUrl, toMsg)

let tick = (m: Model, dt: float, tts: float) => m

// WASD -> a velocity string the server understands; other keys send nothing.
let velocityFor = (key: Key.t): string =>
  match key with
  | Key.W => "0 1"
  | Key.S => "0 -1"
  | Key.A => "-1 0"
  | Key.D => "1 0"
  | _ => ""

// Key down sends the direction; key release sends a stop. Both need the live
// connection id, so nothing is sent until the socket has opened.
let input = (m: Model, key: Key.t, isDown: bool) =>
  match m.conn with
  | Offline => m
  | Online(id) =>
    (match isDown with
     | true =>
       (match velocityFor(key) with
        | "" => m
        | v => (m, Effect.send(id, v)))
     | false => (m, Effect.send(id, "0 0")))

let draw = (m: Model, tts: float) => View.world(m.world)
