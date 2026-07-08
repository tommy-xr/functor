// string builtins: Text.split / Text.join / Text.parseFloat — the wire-protocol
// trio the multiplayer ports use to encode and decode snapshots. Fixed-point
// integers keep the wire trivial (Text.fixed(n, 0.0) is the F# `%d` shape).

// Encode one player as "pid,x*100,z*100".
let encode = (pid: float, x: float, z: float): string =>
  Text.join(
    ",",
    [Text.fixed(pid, 0.0), Text.fixed(x * 100.0, 0.0), Text.fixed(z * 100.0, 0.0)])

// The full snapshot: players joined with '|'.
let snapshot = (rows: List<string>): string => Text.join("|", rows)

// Decode one row back to world units (the *100 fixed-point undone).
let parseRow = (row: string): (float, float, float) =>
  match Text.split(",", row) with
  | [p, x, z] => (Text.parseFloat(p), Text.parseFloat(x) / 100.0, Text.parseFloat(z) / 100.0)
  | _ => (0.0, 0.0, 0.0)

let main = () =>
  let wire = snapshot([encode(0.0, 2.5, -1.8), encode(1.0, 0.12, 0.44)]) in
  (wire, parseRow("0,250,-180"), parseRow("bogus"))
