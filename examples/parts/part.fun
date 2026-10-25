// examples/parts — a printable part defined in code. Export it, or view it
// at true size (a turntable on desktop, 1:1 in VR):
//
//   functor -d examples/parts export -o part.stl
//   functor -d examples/parts run native
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

// The viewer: the part sits on a table-top plane, turning slowly. Scene units
// are metres, so the 40 mm plate is 0.04 wide — the camera sits close.
let init = {}

let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 0.07, -0.12), Vec3.make(0.0, 0.008, 0.0))
      |> Camera3D.clip(0.005, 10.0),
    Scene.group([
      Scene.plane() |> Scene.scale(0.5) |> Scene.lit(Color.rgb(0.55, 0.56, 0.6)),
      Solid.toScene(part)
        |> Scene.rotateY(Angle.radians(tts * 0.4))
        |> Scene.lit(Color.rgb(0.88, 0.36, 0.6)),
    ]),
    [
      Light.ambient(Color.rgb(0.18, 0.18, 0.22)),
      Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.97, 0.92), 0.9)
        |> Light.castShadows,
    ])

expect Solid.volume(plate) == plateSize * plateSize * plateThickness
expect Vec3.z(Solid.bounds(part).min) == 0.0
expect Vec3.z(Solid.bounds(part).max) == pegTop - embed + pegRadius
expect Vec3.x(Solid.bounds(part).max) == plateSize / 2.0
