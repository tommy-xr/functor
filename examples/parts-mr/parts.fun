// The printable parts the viewer cycles through, in millimetres with Z up
// (the 3D-printing convention). Each is a list of pieces, one per filament
// colour: the tag from examples/parts, the bookmark from examples/bookmark,
// and a 20 mm calibration cube. Any `Solid.*` works here.

type piece = { solid: Solid.t, color: Color.t }
type part = { pieces: List<piece> }

let pink = Color.rgb(0.88, 0.36, 0.6)
let white = Color.rgb(0.95, 0.95, 0.95)
let orange = Color.rgb(1.0, 0.55, 0.15)

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

// examples/parts: a 60 × 30 mm keychain tag with a raised name and heart.
let tag =
  let thickness = 3.0 in
  let embed = 0.2 in
  let font = Typeface.script() in
  let plate =
    Sketch.roundedRect(60.0, 30.0, 5.0)
      |> Sketch.subtract(Sketch.circle(3.0) |> Sketch.translate(-23.0, 0.0))
      |> Solid.extrude(thickness) in
  let relief =
    Sketch.union([
      Sketch.text(font, 6.5, "Love")
        |> Sketch.translate(4.0 - Sketch.textWidth(font, 6.5, "Love") / 2.0, 0.5),
      heart(5.0) |> Sketch.translate(4.0, -7.5),
    ])
      |> Solid.extrude(1.0 + embed)
      |> Solid.translate(Vec3.make(0.0, 0.0, thickness - embed)) in
  { pieces: [{ solid: plate, color: pink }, { solid: relief, color: white }] }

// examples/bookmark: a 180 × 58 mm two-colour bookmark.
let bookmark =
  let length = 180.0 in
  let width = 58.0 in
  let r = 3.0 in
  let baseHeight = 1.2 in
  let holeX = 17.0 in
  let typeface = Typeface.script() in
  let scallops = (ax: float, ay: float, bx: float, by: float) =>
    let n = Math.round(Math.sqrt((bx - ax) * (bx - ax) + (by - ay) * (by - ay)) / 5.0) in
    List.range(n)
      |> List.map((i) =>
        Sketch.circle(r) |> Sketch.translate(ax + (bx - ax) * i / n, ay + (by - ay) * i / n)) in
  let x1 = length - r in
  let y1 = width - r in
  let outline =
    Sketch.union(List.flatten([
      [Sketch.rect(length - 2.0 * r, width - 2.0 * r) |> Sketch.translate(length / 2.0, width / 2.0)],
      scallops(r, r, x1, r),
      scallops(x1, r, x1, y1),
      scallops(x1, y1, r, y1),
      scallops(r, y1, r, r),
    ])) in
  let base =
    outline
      |> Sketch.subtract(heart(5.0) |> Sketch.translate(holeX, width / 2.0))
      |> Solid.extrude(baseHeight) in
  let line = (text: string, x: float, y: float, size: float) =>
    Sketch.text(typeface, size, text) |> Sketch.offset(0.12) |> Sketch.translate(x, y) in
  let frame =
    Sketch.roundedRect(length - 9.0, width - 9.0, 1.5)
      |> Sketch.translate(length / 2.0, width / 2.0) in
  let signatureRight = length - 11.0 in
  let relief =
    Sketch.union([
      frame |> Sketch.subtract(frame |> Sketch.offset(-1.0)),
      heart(5.0)
        |> Sketch.offset(1.2)
        |> Sketch.subtract(heart(5.0))
        |> Sketch.translate(holeX, width / 2.0),
      line("Wife,", 31.0, 41.0, 8.0),
      line("you are my favorite chapter, and", 35.0, 32.5, 4.8),
      line("you, Kid 1, and Kid 2 are my favorite story.", 35.0, 25.0, 4.8),
      line("I love you now and always.", 35.0, 17.5, 4.8),
      line("Husband", signatureRight - 9.0 - Sketch.textWidth(typeface, 6.0, "Husband"), 9.0, 6.0),
      heart(2.6) |> Sketch.translate(signatureRight - 3.5, 11.5),
    ])
      |> Solid.extrude(0.6)
      |> Solid.translate(Vec3.make(0.0, 0.0, baseHeight)) in
  { pieces: [{ solid: base, color: pink }, { solid: relief, color: white }] }

// A 20 mm calibration cube with a raised X (front), Y (right) and Z (top).
let calibrationCube =
  let half = 10.0 in
  let raise = 0.6 in
  let embed = 0.2 in
  let font = Typeface.sans() in
  // A letter centered on the top face, standing `raise` proud of it.
  let letter = (c: string) =>
    Sketch.text(font, 9.0, c)
      |> Sketch.translate(0.0 - Sketch.textWidth(font, 9.0, c) / 2.0, -4.5)
      |> Solid.extrude(raise + embed)
      |> Solid.translate(Vec3.make(0.0, 0.0, half - embed)) in
  let letters =
    Solid.group([
      letter("Z"),
      // Tip the top face's letter onto the front (-Y), reading upright...
      letter("X") |> Solid.rotateX(Angle.degrees(90.0)),
      // ...then turn the front onto the right (+X).
      letter("Y") |> Solid.rotateX(Angle.degrees(90.0)) |> Solid.rotateZ(Angle.degrees(90.0)),
    ]) in
  {
    pieces: [{ solid: Solid.box(20.0, 20.0, 20.0), color: orange }, { solid: letters, color: white }]
  }
