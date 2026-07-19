// client.fun — the lobby client (`client` is the default entry):
//
//   functor -d examples/lobby run native
//
// The full discovery flow, all typed: connect to the master, ask
// ListServers, pick the FIRST listed server, connect to it, Join, and land
// in Playing on its Welcome. The connection to the game server exists only
// in `subscriptions` once the model holds a discovered addr — the runtime's
// per-frame reconciler opens it on the next frame (a model-driven
// connection set). While the list is empty it simply asks again on each
// (empty) reply — naive once-per-round-trip polling until a server appears.

type Phase =
  | Discovering
  | Joining(addr: string)
  | Playing(addr: string, motd: string)

type Model = { phase: Phase }

type Msg =
  | MasterUp(id: float)
  | FromMaster(id: float, wire: Protocol.Wire)
  | ServerUp(id: float)
  | FromServer(id: float, wire: Protocol.Wire)
  | Ignore

let fromMaster = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => MasterUp(id)
  | Net.Data(id, wire) => FromMaster(id, wire)
  | _ => Ignore

let fromServer = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => ServerUp(id)
  | Net.Data(id, wire) => FromServer(id, wire)
  | _ => Ignore

let init = { phase: Discovering }

let update = (m: Model, msg: Msg) =>
  match msg with
  | MasterUp(id) => (m, Effect.sendMsg(id, Protocol.ListServers))
  | FromMaster(id, wire) =>
      (match wire with
       | Protocol.Servers(servers) =>
           (match servers with
            | [first, ..rest] => { m with phase: Joining(first.addr) }
            // Nothing listed yet: ask again (poll once per round trip).
            | _ => (m, Effect.sendMsg(id, Protocol.ListServers)))
       | _ => m)
  | ServerUp(id) => (m, Effect.sendMsg(id, Protocol.Join("newcomer")))
  | FromServer(_, wire) =>
      (match wire with
       | Protocol.Welcome(motd) =>
           (match m.phase with
            | Joining(addr) => { m with phase: Playing(addr, motd) }
            | _ => m)
       | _ => m)
  | Ignore => m

let subscriptions = (m: Model) =>
  match m.phase with
  | Discovering => Sub.connect(Protocol.masterUrl, fromMaster)
  | Joining(addr) =>
      Sub.batch([Sub.connect(Protocol.masterUrl, fromMaster), Sub.connect(addr, fromServer)])
  | Playing(addr, _) =>
      Sub.batch([Sub.connect(Protocol.masterUrl, fromMaster), Sub.connect(addr, fromServer)])

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  // One cube, colored by phase: grey while discovering, blue while joining,
  // green once Playing (the Welcome landed).
  let color =
    match m.phase with
    | Discovering => Color.rgb(0.6, 0.6, 0.6)
    | Joining(_) => Color.rgb(0.35, 0.60, 0.95)
    | Playing(_, _) => Color.rgb(0.35, 0.85, 0.45) in
  let cube = Scene.cube() |> Scene.lit(color) in
  let ground = Scene.plane() |> Scene.scale(8.0) |> Scene.lit(Color.rgb(0.18, 0.2, 0.28)) in
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 3.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([ground, cube]),
    [ Light.ambient(Color.rgb(0.35, 0.35, 0.42)),
      Light.directional(Vec3.make(-0.4, -1.0, -0.35), Color.rgb(1.0, 0.95, 0.85), 1.1) ])
