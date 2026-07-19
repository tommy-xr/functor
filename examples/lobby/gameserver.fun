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

type Model = { registered: bool, joined: List<float> }

type Msg =
  | MasterUp(id: float)
  | ClientPacket(cid: float, wire: Protocol.Wire)
  | ClientGone(cid: float)
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
  | Net.Disconnected(cid) => ClientGone(cid)
  | _ => Ignore

let init = { registered: false, joined: [] }

let update = (m: Model, msg: Msg) =>
  match msg with
  | MasterUp(id) =>
      ({ m with registered: true },
       Effect.sendMsg(id, Protocol.Register(serverName, gameUrl)))
  | ClientPacket(cid, wire) =>
      (match wire with
       | Protocol.Join(who) =>
           // Track the joined connection (idempotent per cid); ClientGone
           // delists it, so `joined` is the CURRENT roster.
           ({ m with joined:
                [cid, ..m.joined |> List.filter((c) => not c == cid)] },
            Effect.sendMsg(cid, Protocol.Welcome(Text.concat("hello, ", who))))
       | _ => m)
  | ClientGone(cid) =>
      { m with joined: m.joined |> List.filter((c) => not c == cid) }
  | Ignore => m

let subscriptions = (m: Model) =>
  Sub.batch([
    Sub.listen(gameBind, fromClients),
    Sub.connect(Protocol.masterUrl, fromMaster)
  ])

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  // A blue cube per currently joined client; the tall center pillar goes
  // green once the server has registered with the master.
  let pillar =
    Scene.cube()
    |> Scene.scaleXYZ(0.4, 2.0, 0.4)
    |> Scene.lit(
         if m.registered then Color.rgb(0.35, 0.85, 0.45)
         else Color.rgb(0.6, 0.6, 0.6)) in
  let players =
    List.range(List.length(m.joined)) |> List.map((i) =>
      Scene.cube()
      |> Scene.scale(0.5)
      |> Scene.translate(Vec3.make(i * 1.2 + 1.2, 0.0, 0.0))
      |> Scene.lit(Color.rgb(0.35, 0.60, 0.95))) in
  View.frame([pillar, ..players])
