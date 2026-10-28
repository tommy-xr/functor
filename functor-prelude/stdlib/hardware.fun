// Hardware — cut solids for screws, nuts, heat-set inserts and magnets.
// Subtract them from a part: each opens at z = 0 and goes down (-z), plus
// 1 mm above z = 0 so the cut never leaves a skin on the face. Translate it
// onto the face it opens on.
//
//   let plate =
//     Solid.box(40.0, 20.0, 6.0) |> Solid.translate(Vec3.make(0.0, 0.0, -3.0))
//       |> Solid.subtract(Hardware.counterbore(Hardware.m3, Fit.clearance, 10.0))

// Metric screw sizes (mm): socket head cap (ISO 4762) heads, 90° countersunk
// heads at their theoretical (sharp-edge) diameter (ISO 10642 from M3; M2 and
// M2.5 from supplier tables), and hex nuts (ISO 4032).
type screw = {
  name: string,
  diameter: float,
  headDiameter: float,
  headHeight: float,
  sinkDiameter: float,
  nutWidth: float,
  nutHeight: float,
}

let m2: screw = { name: "M2", diameter: 2.0, headDiameter: 3.8, headHeight: 2.0, sinkDiameter: 4.7, nutWidth: 4.0, nutHeight: 1.6 }
let m2_5: screw = { name: "M2.5", diameter: 2.5, headDiameter: 4.5, headHeight: 2.5, sinkDiameter: 5.88, nutWidth: 5.0, nutHeight: 2.0 }
let m3: screw = { name: "M3", diameter: 3.0, headDiameter: 5.5, headHeight: 3.0, sinkDiameter: 6.72, nutWidth: 5.5, nutHeight: 2.4 }
let m4: screw = { name: "M4", diameter: 4.0, headDiameter: 7.0, headHeight: 4.0, sinkDiameter: 8.96, nutWidth: 7.0, nutHeight: 3.2 }
let m5: screw = { name: "M5", diameter: 5.0, headDiameter: 8.5, headHeight: 5.0, sinkDiameter: 11.2, nutWidth: 8.0, nutHeight: 4.7 }
let m6: screw = { name: "M6", diameter: 6.0, headDiameter: 10.0, headHeight: 6.0, sinkDiameter: 13.44, nutWidth: 10.0, nutHeight: 5.2 }
let m8: screw = { name: "M8", diameter: 8.0, headDiameter: 13.0, headHeight: 8.0, sinkDiameter: 17.92, nutWidth: 13.0, nutHeight: 6.8 }

let screws = [m2, m2_5, m3, m4, m5, m6, m8]

// A heat-set insert: the hole it melts into and its length (mm). The values
// below are typical of common brass inserts; check your insert's datasheet,
// or pass your own record (e.g. a measured hole: `{ Hardware.insertM3 with hole: 4.1 }`).
type insert = { name: string, hole: float, length: float }

let insertM2: insert = { name: "M2", hole: 3.2, length: 4.0 }
let insertM2_5: insert = { name: "M2.5", hole: 3.6, length: 5.7 }
let insertM3: insert = { name: "M3", hole: 4.0, length: 5.7 }
let insertM4: insert = { name: "M4", hole: 5.6, length: 8.1 }
let insertM5: insert = { name: "M5", hole: 6.4, length: 9.5 }
let insertM6: insert = { name: "M6", hole: 8.0, length: 12.7 }
let insertM8: insert = { name: "M8", hole: 9.7, length: 12.7 }

let inserts = [insertM2, insertM2_5, insertM3, insertM4, insertM5, insertM6, insertM8]

// Room above the face, so cuts pass cleanly through it.
let above = 1.0

// A Z cylinder of `radius` from z = `bottom` to `top`.
let post = (radius: float, bottom: float, top: float) =>
  Solid.cylinder(radius, top - bottom) |> Solid.translate(Vec3.make(0.0, 0.0, (bottom + top) / 2.0))

// A hole the screw passes through freely, `depth` deep.
let clearanceHole = (s: screw, fit: Fit.t, depth: float) =>
  post(s.diameter / 2.0 + fit.gap, 0.0 - depth, above)

// A clearance hole with a pocket that sinks a socket head flush.
let counterbore = (s: screw, fit: Fit.t, depth: float) =>
  Solid.union([
    clearanceHole(s, fit, depth),
    post(s.headDiameter / 2.0 + fit.gap, 0.0 - s.headHeight - Math.max(fit.gap, 0.0), above),
  ])

// A clearance hole with a 90° cone that sinks a countersunk head flush.
let countersink = (s: screw, fit: Fit.t, depth: float) =>
  let hole = s.diameter / 2.0 + fit.gap in
  let sink = s.sinkDiameter / 2.0 + fit.gap in
  // At 45° the radius shrinks 1 mm per mm down; the cone stops at the hole
  // (so a blind hole keeps its floor) and runs on past the face to `above`.
  Solid.union([
    clearanceHole(s, fit, depth),
    Solid.cone(hole, sink + above, sink - hole + above)
      |> Solid.translate(Vec3.make(0.0, 0.0, (above - (sink - hole)) / 2.0)),
  ])

// A hexagonal pocket that holds a nut from turning, `depth` deep (at least
// `nutHeight`). Flats face ±Y. Add a clearance hole for the screw.
let nutTrap = (s: screw, fit: Fit.t, depth: float) =>
  let w = s.nutWidth / 2.0 + fit.gap in
  // Corner radius of a hexagon `2w` across flats.
  let r = w * 2.0 / Math.sqrt(3.0) in
  Sketch.polygon([
    { x: r, y: 0.0 },
    { x: r / 2.0, y: w },
    { x: 0.0 - r / 2.0, y: w },
    { x: 0.0 - r, y: 0.0 },
    { x: 0.0 - r / 2.0, y: 0.0 - w },
    { x: r / 2.0, y: 0.0 - w },
  ])
    |> Solid.extrude(depth + above)
    |> Solid.translate(Vec3.make(0.0, 0.0, 0.0 - depth))

// A pocket a heat-set insert is melted into, `depth` deep: its length plus
// about two thread pitches (+1 mm for M3) leaves room for displaced plastic.
// The hole is deliberately tight, so no fit is added.
let insertPocket = (i: insert, depth: float) => post(i.hole / 2.0, 0.0 - depth, above)

// A pocket for a disc magnet; it sits flush or just below the face (a
// negative gap tightens the sides, never the depth).
let magnetPocket = (fit: Fit.t, diameter: float, thickness: float) =>
  post(diameter / 2.0 + fit.gap, 0.0 - thickness - Math.max(fit.gap, 0.0), above)
