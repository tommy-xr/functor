// examples/hardware — a fit test coupon: one of each `Hardware` cut in an
// M3 plate. Print it to check your printer's `Fit` before a real part:
// counterbored and countersunk screw holes, a nut trap, a heat-set insert
// pocket and a press-fit magnet pocket.
//
//   functor -d examples/hardware export -o coupon.stl
//   functor -d examples/hardware run native
//
// Units are millimetres, Z up.

let thickness = 8.0
let screw = Hardware.m3
// Minimum plastic between a cut and a face: three 0.4 mm perimeters.
let minWall = 1.2

let length = 66.0
let width = 20.0
let plate = Solid.box(length, width, thickness) |> Solid.translate(Vec3.make(0.0, 0.0, thickness / 2.0))

// Each cut opens on the top face, `x` mm along the plate.
let onTop = (x: float, cut) => Solid.translate(Vec3.make(x, 0.0, thickness), cut)

let counterbore = Hardware.counterbore(screw, Fit.clearance, thickness + 1.0) |> onTop(-26.0)
let countersink = Hardware.countersink(screw, Fit.clearance, thickness + 1.0) |> onTop(-13.0)
let nutTrap =
  Solid.union([
    Hardware.nutTrap(screw, Fit.clearance, screw.nutHeight + 0.4),
    Hardware.clearanceHole(screw, Fit.clearance, thickness + 1.0),
  ]) |> onTop(0.0)
let insert = Hardware.insertPocket(Hardware.insertM3, Hardware.insertM3.length + 1.0) |> onTop(13.0)
// A 6 × 3 mm disc magnet.
let magnet = Hardware.magnetPocket(Fit.press, 6.0, 3.0) |> onTop(26.0)

let cuts = [counterbore, countersink, nutTrap, insert, magnet]

let part = plate |> Solid.subtract(Solid.union(cuts))

// What each cut takes out of the plate.
let removed = (cut) => Solid.intersect(plate, cut)

// The cuts don't overlap, so the part loses exactly what each one takes.
expect Math.abs(Solid.volume(part) - (Solid.volume(plate) - List.sum(List.map((c) => Solid.volume(removed(c)), cuts)))) < 0.01

// Walls of at least `minWall`: between neighbouring cuts and to the ends,
let spans = List.map((c) => Solid.bounds(removed(c)), cuts)
// (List.append(b, a) is a then b.)
let lefts = List.append(List.map((b) => Vec3.x(b.max), spans), [0.0 - length / 2.0])
let rights = List.append([length / 2.0], List.map((b) => Vec3.x(b.min), spans))
expect List.all((p) => (let (l, r) = p in r - l >= minWall), List.zip(rights, lefts))
// to the sides,
expect List.all((b) => Vec3.y(b.min) >= minWall - width / 2.0 && Vec3.y(b.max) <= width / 2.0 - minWall, spans)
// and under the blind pockets.
expect Vec3.z(Solid.bounds(removed(insert)).min) >= minWall
expect Vec3.z(Solid.bounds(removed(magnet)).min) >= minWall
