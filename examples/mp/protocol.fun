// protocol.fun — the wire protocol shared by client.fun and server.fun
// (file = module: both entries load this sibling as `Protocol`, so the two
// roles cannot drift apart).
//
// Wire format: "pid,x*100,z*100|pid,x*100,z*100|...". Coordinates are
// fixed-point integers (x * 100), so an encoded coordinate rounds to within
// 0.01 world units (`Text.fixed(n, 0.0)` ROUNDS).

type Row = { pid: float, x: float, z: float }

let bind = "127.0.0.1:9001"
let serverUrl = "ws://127.0.0.1:9001/play"
let arena = 4.0

let row = (pid: float, x: float, z: float): Row => { pid: pid, x: x, z: z }

let encodeRow = (r: Row): string =>
  Text.join(
    ",",
    [Text.fixed(r.pid, 0.0), Text.fixed(r.x * 100.0, 0.0), Text.fixed(r.z * 100.0, 0.0)])

// [Row] -> "pid,x,z|pid,x,z|..." (the server's broadcast snapshot).
let encode = (rows: List<Row>): string =>
  Text.join("|", List.map(encodeRow, rows))

// "pid,x*100,z*100|..." -> [Row]. Fold+cons filters malformed rows (not
// exactly 3 fields) and undoes the *100 fixed-point; draw order is
// irrelevant, so the reversal cons introduces doesn't matter.
let decode = (s: string): List<Row> =>
  Text.split("|", s) |> List.fold((acc, rowText) =>
    match Text.split(",", rowText) with
    | [p, x, z] =>
        [row(Text.parseFloat(p),
             Text.parseFloat(x) / 100.0,
             Text.parseFloat(z) / 100.0), ..acc]
    | _ => acc,
    [])

// A distinct color per player id (wrapping every 4), shared so a given
// player is the same color in every pane of the netsim viewer.
let colorFor = (pid: float): (float, float, float) =>
  match Math.mod(pid, 4.0) with
  | 0.0 => (0.90, 0.35, 0.35)
  | 1.0 => (0.35, 0.60, 0.95)
  | 2.0 => (0.45, 0.85, 0.45)
  | _ => (0.95, 0.80, 0.35)
