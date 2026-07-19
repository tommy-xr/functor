// protocol.fun — the lobby wire shared by master.fun, gameserver.fun, and
// client.fun (file = module: every entry loads this sibling as `Protocol`).
// Three conversations, one typed ADT — sent with `Effect.sendMsg`, received
// as `Net.Data`, no string codec anywhere:
//
//   game server -> master:  Register (connection-scoped: dropping the
//                           connection delists the server)
//   client     <-> master:  ListServers / Servers (discovery)
//   client     <-> server:  Join / Welcome (the "game")

type ServerInfo = { name: string, addr: string }

type Wire =
  | Register(name: string, addr: string)
  | ListServers
  | Servers(servers: List<ServerInfo>)
  | Join(who: string)
  | Welcome(motd: string)

let masterBind = "127.0.0.1:9200"
let masterUrl = "ws://127.0.0.1:9200/lobby"

let serverInfo = (name: string, addr: string): ServerInfo => { name: name, addr: addr }
