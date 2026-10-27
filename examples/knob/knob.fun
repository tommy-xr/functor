// examples/knob — a printable knob for a 6 mm D-shaft potentiometer, built
// with 3D booleans: a domed body, a knurled grip, an indicator groove, and a
// D-shaped bore with print clearance. Export it, or view it on a turntable:
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

// The viewer: the knob on a plane, turning slowly. Scene units are metres.
let init = {}

let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 0.06, 0.05), Vec3.make(0.0, 0.006, 0.0))
      |> Camera3D.clip(0.005, 10.0),
    Scene.group([
      Scene.plane() |> Scene.scale(0.5) |> Scene.lit(Color.rgb(0.55, 0.56, 0.6)),
      Solid.toScene(part)
        |> Scene.rotateY(Angle.radians(tts * 0.4))
        |> Scene.lit(Color.rgb(0.25, 0.5, 0.85)),
    ]),
    [
      Light.ambient(Color.rgb(0.18, 0.18, 0.22)),
      Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.97, 0.92), 0.9),
    ])
