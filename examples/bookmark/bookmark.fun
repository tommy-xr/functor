// examples/bookmark — a two-colour bookmark: a scalloped pink base with a
// heart-shaped ribbon hole, and a white raised frame and message. Export it
// ready to slice, each part on its own filament:
//
//   functor -d examples/bookmark export -o bookmark.3mf --part base --part relief
//
// Units are millimetres, Z up; the origin is the bookmark's lower-left corner.

let length = 180.0
let width = 58.0
let corner = 6.0
let scallopRadius = 3.0
let scallopSpacing = 5.0
let baseHeight = 1.2
let reliefHeight = 0.6
// Thickens strokes so thin script lines survive a 0.4 mm nozzle.
let fatten = 0.12
let holeX = 17.0
let holeSize = 5.0
let borderInset = 4.5
let borderWidth = 1.0

let typeface = Typeface.script()

// A heart pointing down, centered on the origin: a square with a circle on
// two adjacent edges, turned 45°. About 1.71 × 1.56 times `size`.
let heart = (size: float) =>
  let r = size / 2.0 in
  Sketch.union([
    Sketch.rect(size, size) |> Sketch.translate(r, r),
    Sketch.circle(r) |> Sketch.translate(size, r),
    Sketch.circle(r) |> Sketch.translate(r, size),
  ])
    |> Sketch.rotate(Angle.degrees(45.0))
    |> Sketch.translate(0.0, 0.0 - 0.78 * size)

// Overlapping circles stepping from (ax, ay) toward (bx, by): one scalloped edge.
let scallops = (ax: float, ay: float, bx: float, by: float) =>
  let n = Math.round(Math.sqrt((bx - ax) * (bx - ax) + (by - ay) * (by - ay)) / scallopSpacing) in
  List.range(n)
    |> List.map((i) =>
      Sketch.circle(scallopRadius)
        |> Sketch.translate(ax + (bx - ax) * i / n, ay + (by - ay) * i / n))

let outline =
  let r = scallopRadius in
  let x1 = length - r in
  let y1 = width - r in
  Sketch.union(List.flatten([
    [Sketch.rect(length - 2.0 * r, width - 2.0 * r) |> Sketch.translate(length / 2.0, width / 2.0)],
    scallops(r, r, x1, r),
    scallops(x1, r, x1, y1),
    scallops(x1, y1, r, y1),
    scallops(r, y1, r, r),
  ]))

let hole = heart(holeSize) |> Sketch.translate(holeX, width / 2.0)

let base = outline |> Sketch.subtract(hole) |> Solid.extrude(baseHeight)

// One line of the message, fattened; `x` is its left edge.
let line = (text: string, x: float, y: float, size: float) =>
  Sketch.text(typeface, size, text)
    |> Sketch.offset(fatten)
    |> Sketch.translate(x, y)

let frame =
  Sketch.roundedRect(length - 2.0 * borderInset, width - 2.0 * borderInset, corner - borderInset)
    |> Sketch.translate(length / 2.0, width / 2.0)

let textLeft = 31.0
let signatureRight = length - 11.0

let relief =
  Sketch.union([
    frame |> Sketch.subtract(frame |> Sketch.offset(0.0 - borderWidth)),
    heart(holeSize)
      |> Sketch.offset(1.2)
      |> Sketch.subtract(heart(holeSize))
      |> Sketch.translate(holeX, width / 2.0),
    line("Wife,", textLeft, 41.0, 8.0),
    line("you are my favorite chapter, and", textLeft + 4.0, 32.5, 4.8),
    line("you, Kid 1, and Kid 2 are my favorite story.", textLeft + 4.0, 25.0, 4.8),
    line("I love you now and always.", textLeft + 4.0, 17.5, 4.8),
    line("Husband", signatureRight - 9.0 - Sketch.textWidth(typeface, 6.0, "Husband"), 9.0, 6.0),
    heart(2.6) |> Sketch.translate(signatureRight - 3.5, 11.5),
  ])
    |> Solid.extrude(reliefHeight)
    |> Solid.translate(Vec3.make(0.0, 0.0, baseHeight))

expect Vec3.x(Solid.bounds(base).max) == length
expect Vec3.y(Solid.bounds(base).max) == width
expect Math.abs(Vec3.z(Solid.bounds(relief).max) - (baseHeight + reliefHeight)) < 0.0001
expect Vec3.x(Solid.bounds(relief).max) < length - borderInset + 0.01

// The viewer: the bookmark at true size, tilting gently. Scene units are
// metres, so it is 0.18 long.
let init = {}

let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 0.2, 0.14), Vec3.make(0.0, 0.0, 0.0))
      |> Camera3D.clip(0.01, 10.0),
    Scene.group([
      Scene.plane() |> Scene.scale(1.0) |> Scene.lit(Color.rgb(0.55, 0.56, 0.6)),
      Scene.group([
        Solid.toScene(base) |> Scene.lit(Color.rgb(0.88, 0.36, 0.6)),
        Solid.toScene(relief) |> Scene.lit(Color.rgb(0.95, 0.95, 0.95)),
      ])
        |> Scene.translate(Vec3.make(0.0 - length / 2000.0, 0.0, width / 2000.0))
        |> Scene.rotateY(Angle.radians(Math.sin(tts * 0.5) * 0.3)),
    ]),
    [
      Light.ambient(Color.rgb(0.2, 0.2, 0.24)),
      Light.directional(Vec3.make(0.3, -1.0, -0.4), Color.rgb(1.0, 0.97, 0.92), 0.9),
    ])
