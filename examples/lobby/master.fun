// master.fun — the master server:
//
//   functor -d examples/lobby run --entry master native --headless
//
// A tiny CONNECTION-SCOPED registry, pure Functor Lang — there is no engine
// "master server" surface, just `Sub.listen` + typed messages. A game server
// Registers over its own connection and stays listed exactly as long as that
// connection lives: `Net.Disconnected` delists it, so no TTL machinery is
// needed. Clients send ListServers and get the current Servers back.

type Entry = { cid: float, info: Protocol.ServerInfo }

type Model = { entries: List<Entry> }

type Msg =
  | Packet(cid: float, wire: Protocol.Wire)
  | Gone(cid: float)
  | Ignore

let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Data(cid, wire) => Packet(cid, wire)
  | Net.Disconnected(cid) => Gone(cid)
  | _ => Ignore

let init = { entries: [] }

let listed = (m: Model): List<Protocol.ServerInfo> =>
  m.entries |> List.map((e) => e.info)

let update = (m: Model, msg: Msg) =>
  match msg with
  | Packet(cid, wire) =>
      (match wire with
       | Protocol.Register(name, addr) =>
           // Upsert: re-registering over the same connection replaces.
           let others = m.entries |> List.filter((e) => not e.cid == cid) in
           { m with entries:
               [{ cid: cid, info: Protocol.serverInfo(name, addr) }, ..others] }
       | Protocol.ListServers => (m, Effect.sendMsg(cid, Protocol.Servers(listed(m))))
       | _ => m)
  | Gone(cid) =>
      { m with entries: m.entries |> List.filter((e) => not e.cid == cid) }
  | Ignore => m

let subscriptions = (m: Model) => Sub.listen(Protocol.masterBind, toMsg)

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  // One green cube per registered server, in a row — an at-a-glance registry.
  View.frame(
    m.entries |> List.map((e) =>
      Scene.cube()
      |> Scene.scale(0.7)
      |> Scene.translate(Vec3.make(e.cid * 1.5, 0.0, 0.0))
      |> Scene.lit(Color.rgb(0.35, 0.85, 0.45))))
