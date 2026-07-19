// gameserver.fun — a game server that registers itself with the master and
// serves game clients:
//
//   functor -d examples/lobby run --entry server native --headless
//
// Two connections declared side by side in ONE `subscriptions`: the listener
// for game clients, and the outbound registration link to the master. On the
// master link opening it sends a typed Register carrying its own public url;
// registration is connection-scoped, so if this process dies the master's
// `Disconnected` event delists it — no heartbeat protocol needed.

type Model = { registered: bool, players: float }

type Msg =
  | MasterUp(id: float)
  | ClientPacket(cid: float, wire: Protocol.Wire)
  | Ignore

let gameBind = "127.0.0.1:9201"
let gameUrl = "ws://127.0.0.1:9201/game"
let serverName = "arena-1"

let fromMaster = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => MasterUp(id)
  | _ => Ignore

let fromClients = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Data(cid, wire) => ClientPacket(cid, wire)
  | _ => Ignore

let init = { registered: false, players: 0.0 }

let update = (m: Model, msg: Msg) =>
  match msg with
  | MasterUp(id) =>
      ({ m with registered: true },
       Effect.sendMsg(id, Protocol.Register(serverName, gameUrl)))
  | ClientPacket(cid, wire) =>
      (match wire with
       | Protocol.Join(who) =>
           ({ m with players: m.players + 1.0 },
            Effect.sendMsg(cid, Protocol.Welcome(Text.concat("hello, ", who))))
       | _ => m)
  | Ignore => m

let subscriptions = (m: Model) =>
  Sub.batch([
    Sub.listen(gameBind, fromClients),
    Sub.connect(Protocol.masterUrl, fromMaster)
  ])

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  // A blue cube per joined player; the tall center pillar goes green once
  // the server has registered with the master.
  let pillar =
    Scene.cube()
    |> Scene.scaleXYZ(0.4, 2.0, 0.4)
    |> Scene.lit(
         if m.registered then Color.rgb(0.35, 0.85, 0.45)
         else Color.rgb(0.6, 0.6, 0.6)) in
  let players =
    List.range(m.players) |> List.map((i) =>
      Scene.cube()
      |> Scene.scale(0.5)
      |> Scene.translate(Vec3.make(i * 1.2 + 1.2, 0.0, 0.0))
      |> Scene.lit(Color.rgb(0.35, 0.60, 0.95))) in
  let ground = Scene.plane() |> Scene.scale(8.0) |> Scene.lit(Color.rgb(0.18, 0.2, 0.28)) in
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 4.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([ground, pillar, ..players]),
    [ Light.ambient(Color.rgb(0.35, 0.35, 0.42)),
      Light.directional(Vec3.make(-0.4, -1.0, -0.35), Color.rgb(1.0, 0.95, 0.85), 1.1) ])
