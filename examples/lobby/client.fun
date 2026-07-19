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
// If the discovered server dies (or a stale listing fails to connect), the
// client falls back to Discovering and resumes polling instead of wedging.

type Phase =
  | Discovering
  | Joining(addr: string)
  | Playing(addr: string, motd: string)

type Model = { masterConn: float, phase: Phase }

type Msg =
  | MasterUp(id: float)
  | FromMaster(id: float, wire: Protocol.Wire)
  | ServerUp(id: float)
  | FromServer(id: float, wire: Protocol.Wire)
  | ServerGone
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
  // The discovered server dying — or a stale listing that never connects —
  // must not wedge the client: fall back to discovery.
  | Net.Disconnected(_) => ServerGone
  | Net.Error(_, _) => ServerGone
  | _ => Ignore

let init = { masterConn: -1.0, phase: Discovering }

let update = (m: Model, msg: Msg) =>
  match msg with
  | MasterUp(id) => ({ m with masterConn: id }, Effect.sendMsg(id, Protocol.ListServers))
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
  | ServerGone =>
      // Drop the dead server connection and re-ask NOW — nothing else would
      // (the master link stays quiet once a non-empty list was delivered).
      // Already-Discovering dedups the Error+Disconnected double event.
      (match m.phase with
       | Discovering => m
       | _ => ({ m with phase: Discovering },
               Effect.sendMsg(m.masterConn, Protocol.ListServers)))
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
  View.frame([Scene.cube() |> Scene.lit(color)])
