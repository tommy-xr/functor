// examples/split — a 300 mm tower, too tall for a 256 mm bed, split in two
// with dowel pins and laid out to print. Each half and the pins export as
// their own part of one 3MF.
//
//   functor -d examples/split export -o tower.3mf --part top --part bottom --part pins
//   functor -d examples/split run native
//
// Units are millimetres, Z up.

let bed = 256.0
let lift = (z: float, s) => Solid.translate(Vec3.make(0.0, 0.0, z), s)

// A square tower with a bore and a spire, standing on z = 0.
let width = 50.0
let bore = 10.0
let tower =
  Solid.union([
    Solid.box(width, width, 250.0) |> lift(125.0),
    Solid.cone(width / 2.0, 0.0, 50.0) |> lift(275.0),
  ])
    |> Solid.subtract(Solid.cylinder(bore, 232.0) |> lift(115.0))

// Cut halfway up, with two 5 × 16 mm pins in the wall either side of the bore.
let plane = { normal: Vec3.make(0.0, 0.0, 1.0), offset: 150.0 }
let dowel = { diameter: 5.0, length: 16.0 }
let fit = Fit.slip
let spots = [{ x: -17.5, y: 0.0 }, { x: 17.5, y: 0.0 }]

let halves = Connector.dowelled(plane, fit, dowel, spots, tower)
let upper = match halves with | (above, _) => above
let lower = match halves with | (_, below) => below

// On the bed: each half on its flat end (the top half on its cut face, so
// the spire points up), and the pins standing beside them.
let down = Vec3.make(0.0, 0.0, -1.0)
let top = Connector.layFlat(down, upper) |> Solid.translate(Vec3.make(-35.0, 0.0, 0.0))
let bottom = Connector.layFlat(down, lower) |> Solid.translate(Vec3.make(35.0, 0.0, 0.0))
let pins = Solid.group([
  Connector.pin(dowel) |> Solid.translate(Vec3.make(-6.0, -40.0, 0.0)),
  Connector.pin(dowel) |> Solid.translate(Vec3.make(6.0, -40.0, 0.0)),
])

let part = Solid.group([top, bottom, pins])

// The assembly pulled apart along the cut, pins between the halves — set
// `let part = exploded` to see it.
let seated = (at: Input.point2) => Connector.onPlane(plane, at, Connector.pin(dowel) |> lift(0.0 - dowel.length / 2.0))
let exploded = Solid.group([
  lower,
  Solid.group(List.map(seated, spots)) |> lift(25.0),
  upper |> lift(50.0),
])

// Too tall to print whole; each half fits the bed.
expect Vec3.z(Solid.bounds(tower).max) > bed
expect List.all((s) => Vec3.z(Solid.bounds(s).max) <= bed && Math.abs(Vec3.z(Solid.bounds(s).min)) < 0.001, [top, bottom, pins])

// The halves hold all of the tower but the two holes.
expect Math.abs(Solid.volume(upper) + Solid.volume(lower) - (Solid.volume(tower) - 2.0 * Solid.volume(Connector.hole(fit, dowel)))) < 0.01

// Pins seated in their holes touch neither half, with the fit's gap all round.
expect List.all((at: Input.point2) => Solid.volume(Solid.intersect(upper, seated(at))) < 0.000001 && Solid.volume(Solid.intersect(lower, seated(at))) < 0.000001, spots)
expect Math.abs(Vec3.x(Solid.bounds(Connector.hole(fit, dowel)).max) - dowel.diameter / 2.0 - fit.gap) < 0.000001

// At least 1.2 mm of wall (three 0.4 mm lines) between each hole and the
// bore, and between it and the tower's sides.
let holeRadius = dowel.diameter / 2.0 + fit.gap
expect List.all((at: Input.point2) =>
  Math.sqrt(at.x * at.x + at.y * at.y) - holeRadius - bore >= 1.2
    && width / 2.0 - Math.max(Math.abs(at.x), Math.abs(at.y)) - holeRadius >= 1.2, spots)
