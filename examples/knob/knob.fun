// examples/knob — a printable knob for a 6 mm D-shaft potentiometer, built
// with 3D booleans: a domed body, a knurled grip, an indicator groove, and a
// D-shaped bore with print clearance. Export it, or view it (a file that is
// only a `part` runs in the default part viewer):
//
//   functor -d examples/knob export -o knob.stl
//   functor -d examples/knob run native
//
// Units are millimetres, Z up.

let radius = 15.0
let height = 14.0
let knurls = 24.0
let knurlRadius = 1.5
// Printed holes come out small: bore the shaft this much oversize.
let clearance = 0.15
let shaftRadius = 3.0 + clearance
// A D-shaft's flat sits 4.5 mm across from the round side; both faces get
// clearance.
let flatAcross = 4.5 + 2.0 * clearance
let boreDepth = 11.0

let lift = (z: float, s) => Solid.translate(Vec3.make(0.0, 0.0, z), s)

// The body: a cylinder whose top is domed by a large sphere. The cylinder
// runs past the dome so the sphere alone shapes the top (no tangent point).
let body =
  Solid.cylinder(radius, height * 2.0)
    |> lift(height)
    |> Solid.intersect(Solid.sphere(40.0) |> lift(height - 40.0))

// Grooves around the rim for grip.
let grip =
  List.range(knurls)
    |> List.map((i) =>
      Solid.cylinder(knurlRadius, height * 3.0)
        |> Solid.translate(Vec3.make(radius, 0.0, 0.0))
        |> Solid.rotateZ(Angle.degrees(360.0 * i / knurls)))

let groove =
  Solid.box(1.2, radius, 2.0)
    |> Solid.translate(Vec3.make(0.0, radius / 2.0 + 2.0, height))

// The D bore: a round hole cut flat on one side, open at the bottom.
let bore =
  Solid.cylinder(shaftRadius, boreDepth * 2.0)
    |> Solid.intersect(
      Solid.box(shaftRadius * 2.0, flatAcross, boreDepth * 2.0)
        |> Solid.translate(Vec3.make(0.0, flatAcross / 2.0 - shaftRadius, 0.0)))

let part = body |> Solid.subtract(Solid.union(List.append(grip, [groove, bore])))

expect Solid.volume(part) > 0.0
expect Solid.volume(part) < Solid.volume(body)
expect Vec3.z(Solid.bounds(part).min) == 0.0
expect Vec3.z(Solid.bounds(part).max) < height + 0.001
// The bore's flat sits clearance beyond the shaft's 1.5 mm flat.
expect Math.abs(Vec3.y(Solid.bounds(bore).max) - (1.5 + clearance)) < 0.0001

