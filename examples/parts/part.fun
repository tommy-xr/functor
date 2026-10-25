// examples/parts — a printable part defined in code. Export it with:
//
//   functor -d examples/parts export -o part.stl
//
// Units are millimetres, Z up. `Solid.group` keeps each piece its own closed
// shell, and slicers print overlapping shells as one solid. Pieces overlap by
// `embed`: the peg's rim sitting exactly on the ball's equator would join the
// two at an edge, which won't export.

let plateSize = 40.0
let plateThickness = 4.0
let pegRadius = 6.0
let pegHeight = 12.0
let embed = 1.0

// The plate rests on the bed: shift it up by half its thickness.
let plate =
  Solid.box(plateSize, plateSize, plateThickness)
    |> Solid.translate(Vec3.make(0.0, 0.0, plateThickness / 2.0))

let pegTop = plateThickness - embed + pegHeight

let peg =
  Solid.cylinder(pegRadius, pegHeight)
    |> Solid.translate(Vec3.make(0.0, 0.0, pegTop - pegHeight / 2.0))

let ball =
  Solid.sphere(pegRadius)
    |> Solid.translate(Vec3.make(0.0, 0.0, pegTop - embed))

let part = Solid.group([plate, peg, ball])

expect Solid.volume(plate) == plateSize * plateSize * plateThickness
expect Vec3.z(Solid.bounds(part).min) == 0.0
expect Vec3.z(Solid.bounds(part).max) == pegTop - embed + pegRadius
expect Vec3.x(Solid.bounds(part).max) == plateSize / 2.0
