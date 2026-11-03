// BENCH: float-heavy per-corner math, shaped like a level loader's vertex
// build -- 20k corners, each a closure call with ~12 `let`s, ~40 float ops,
// record field reads, `List.nth` vector components and a small record out.
let n = (k: float, v: List<float>): float => Option.defaultValue(0.0, List.nth(k, v))
type face = { u: float, v: float, width: float, height: float }
let main = () =>
  let f: face = { u: 3.0, v: 5.0, width: 16.0, height: 8.0 } in
  let axisU = [1.0, 0.25, 0.0] in
  let axisV = [0.0, 0.5, 1.0] in
  let ux = n(0.0, axisU) in
  let uy = n(1.0, axisU) in
  let uz = n(2.0, axisU) in
  let vx = n(0.0, axisV) in
  let vy = n(1.0, axisV) in
  let vz = n(2.0, axisV) in
  let mu = ux * ux + uy * uy + uz * uz in
  let mv = vx * vx + vy * vy + vz * vz in
  let uv = ux * vx + uy * vy + uz * vz in
  let corr = 1.0 / (mu * mv - uv * uv) in
  let corners = List.range(20000.0) |> List.map((i) =>
    let p = [i * 0.5, i * 0.25, i * 0.125] in
    let px = n(0.0, p) in
    let py = n(1.0, p) in
    let pz = n(2.0, p) in
    let dx = px - 1.0 in
    let dy = py - 2.0 in
    let dz = pz - 3.0 in
    let au = ux * dx + uy * dy + uz * dz in
    let av = vx * dx + vy * dy + vz * dz in
    let pu = corr * (au * mv - av * uv) in
    let pv = corr * (av * mu - au * uv) in
    let lu = (4.0 * pu + 0.5 - f.u + f.u / 1024.0) / f.width in
    let lv = (4.0 * pv + 0.5 - f.v + f.v / 1024.0) / f.height in
    { p: [0.3048 * px, 0.3048 * pz, 0.3048 * (0.0 - py)], uv: [pu / 4.0, pv / 4.0],
      uv2: [Math.clamp01(lu), Math.clamp01(lv)] }) in
  List.length(corners)
