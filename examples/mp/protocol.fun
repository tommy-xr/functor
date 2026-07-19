// protocol.fun — the typed wire protocol shared by client.fun and server.fun
// (file = module: both entries load this sibling as `Protocol`, so the two
// roles typecheck against ONE declaration and cannot drift).
//
// The `Wire` ADT below IS the protocol: values are sent with
// `Effect.sendMsg(conn, wire)` and arrive on the other end decoded, as
// `Net.Data(id, wire)` — no string codec, no parsing, full float precision.

type Row = { pid: float, x: float, z: float }

type Wire =
  | Move(vx: float, vz: float)
  | Snapshot(rows: List<Row>)

let bind = "127.0.0.1:9001"
let serverUrl = "ws://127.0.0.1:9001/play"
let arena = 4.0

let row = (pid: float, x: float, z: float): Row => { pid: pid, x: x, z: z }

// A distinct color per player id (wrapping every 4), shared so a given
// player is the same color in every pane of the netsim viewer.
let colorFor = (pid: float): (float, float, float) =>
  match Math.mod(pid, 4.0) with
  | 0.0 => (0.90, 0.35, 0.35)
  | 1.0 => (0.35, 0.60, 0.95)
  | 2.0 => (0.45, 0.85, 0.45)
  | _ => (0.95, 0.80, 0.35)
