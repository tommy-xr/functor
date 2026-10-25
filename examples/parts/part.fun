// examples/parts — a printable keychain tag defined in code: a rounded plate
// with a ring hole and a raised name and heart, drawn as 2D sketches and
// extruded.
// Export it, or view it at true size on a turntable:
//
//   functor -d examples/parts export -o part.stl
//   functor -d examples/parts run native
//
// Units are millimetres, Z up.

let tagWidth = 60.0
let tagHeight = 30.0
let thickness = 3.0
let reliefHeight = 1.0
// The heart sinks this far into the plate, so the two pieces overlap rather
// than meet face to face.
let embed = 0.2

let hole = Sketch.circle(3.0) |> Sketch.translate(-23.0, 0.0)

let plate =
  Sketch.roundedRect(tagWidth, tagHeight, 5.0)
    |> Sketch.subtract(hole)
    |> Solid.extrude(thickness)

// A heart pointing down, centered on the origin: a square with a circle on
// two adjacent edges, turned 45°.
let heart = (size: float) =>
  let r = size / 2.0 in
  Sketch.union([
    Sketch.rect(size, size) |> Sketch.translate(r, r),
    Sketch.circle(r) |> Sketch.translate(size, r),
    Sketch.circle(r) |> Sketch.translate(r, size),
  ])
    |> Sketch.rotate(Angle.degrees(45.0))
    |> Sketch.translate(0.0, 0.0 - 0.78 * size)

let name = "Love"
let font = Typeface.script()
let nameSize = 6.5

// The name, centered on the plate's right of the hole; a small heart beneath.
let relief =
  Sketch.union([
    Sketch.text(font, nameSize, name)
      |> Sketch.translate(4.0 - Sketch.textWidth(font, nameSize, name) / 2.0, 0.5),
    heart(5.0) |> Sketch.translate(4.0, -7.5),
  ])
    |> Solid.extrude(reliefHeight + embed)
    |> Solid.translate(Vec3.make(0.0, 0.0, thickness - embed))

let part = Solid.group([plate, relief])

expect Sketch.area(Sketch.rect(tagWidth, tagHeight)) == tagWidth * tagHeight
expect Sketch.area(Sketch.roundedRect(tagWidth, tagHeight, 5.0)) < tagWidth * tagHeight
expect Vec3.z(Solid.bounds(part).max) == thickness + reliefHeight
expect Vec3.x(Solid.bounds(part).max) == tagWidth / 2.0
expect Vec3.y(Solid.bounds(part).max) == tagHeight / 2.0

// The viewer: the part sits on a plane, turning slowly. Scene units are
// metres, so the 60 mm tag is 0.06 wide — the camera sits close. (No
// shadows: the shadow map spans tens of metres, too coarse for a 6 cm part.)
let init = {}

let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 0.1, 0.07), Vec3.make(0.0, 0.002, 0.0))
      |> Camera3D.clip(0.005, 10.0),
    Scene.group([
      Scene.plane() |> Scene.scale(0.5) |> Scene.lit(Color.rgb(0.55, 0.56, 0.6)),
      // Drawn per piece to preview the two-colour print: pink plate, white relief.
      Scene.group([
        Solid.toScene(plate) |> Scene.lit(Color.rgb(0.88, 0.36, 0.6)),
        Solid.toScene(relief) |> Scene.lit(Color.rgb(0.95, 0.95, 0.95)),
      ])
        |> Scene.rotateY(Angle.radians(tts * 0.4)),
    ]),
    [
      Light.ambient(Color.rgb(0.18, 0.18, 0.22)),
      Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.97, 0.92), 0.9),
    ])
