// examples/part-views — a review sheet for a printable part: eight
// orthographic views (front, back, left, right, top, bottom, two isometric),
// each framed to the part, from `PartViews.sheet`.
//
//   functor -d examples/part-views run native
//
// Units are millimetres, Z up, the front facing -Y.

// An angle bracket: a plate with two screw holes and a wall with a slot.
let plate =
  Sketch.roundedRect(40.0, 24.0, 3.0)
    |> Sketch.subtract(Sketch.circle(2.5) |> Sketch.translate(-12.0, 0.0))
    |> Sketch.subtract(Sketch.circle(2.5) |> Sketch.translate(2.0, 0.0))
    |> Solid.extrude(4.0)

let wall =
  Solid.box(4.0, 24.0, 30.0)
    |> Solid.translate(Vec3.make(18.0, 0.0, 15.0))
    |> Solid.subtract(Solid.box(10.0, 6.0, 14.0) |> Solid.translate(Vec3.make(18.0, 0.0, 18.0)))

let part = Solid.union([plate, wall])

// A peg standing in one hole, drawn in its own colour.
let peg = Solid.cylinder(2.5, 14.0) |> Solid.translate(Vec3.make(-12.0, 0.0, 7.0))

let init = 0.0
let tick = (m, dt, tts) => m
let draw = (m, tts) =>
  PartViews.sheet([
    { solid: part, color: Color.rgb(0.25, 0.5, 0.85) },
    { solid: peg, color: Color.rgb(0.9, 0.55, 0.2) },
  ])
