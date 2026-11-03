// examples/world-texture — `Scene.litWorldTexture`: a lit texture box-mapped
// by world position, so boxes of any size show the same square bricks. The
// three boxes on the right are ONE instanced cube draw, scaled and tinted per
// copy; the one on the left is an ordinary box. The paving is the same
// mapping on a plane's top face.

type Rgb = { r: float, g: float, b: float }

// A `w`-texel-square brick texture: four courses, each offset half a brick,
// with `mortar` texels of joint at each brick's top and left.
let bricks = (w: float, mortar: float, joint: Rgb, brick: Rgb): Texture.pixels =>
  let course = w / 4.0 in
  { width: w, height: w, format: Texture.Rgb8, colorSpace: Texture.Srgb,
    pixels: Bytes.fromU8(List.range(w * w) |> List.concatMap((i) =>
      let x = Math.mod(i, w) in
      let y = Math.floor(i / w) in
      let shift = if Math.mod(Math.floor(y / course), 2.0) == 0.0 then 0.0 else w / 4.0 in
      let c = if Math.mod(y, course) < mortar || Math.mod(x + shift, w / 2.0) < mortar then joint else brick in
      [c.r, c.g, c.b])) }

let wall = Texture.fromPixels(bricks(64.0, 2.0, { r: 190.0, g: 188.0, b: 178.0 }, { r: 158.0, g: 76.0, b: 56.0 }))
let paving = Texture.fromPixels(bricks(32.0, 1.0, { r: 90.0, g: 90.0, b: 92.0 }, { r: 152.0, g: 152.0, b: 148.0 }))

// A box spanning [x0, x1] × [0, h] × [z0, z1], as an instance of the unit cube.
let box = (x0: float, z0: float, x1: float, z1: float, h: float, tint: Color.t): Instance.t =>
  Instance.trs(Vec3.make((x0 + x1) / 2.0, h / 2.0, (z0 + z1) / 2.0), Angle.degrees(0.0), x1 - x0, h, z1 - z0)
    |> Instance.tint(tint)

let instancedBoxes =
  Scene.cube() |> Scene.litWorldTexture(wall, 1.0) |> Scene.instanced([
    box(0.0 - 5.0, 1.0, 0.0 - 1.0, 1.5, 2.0, Color.rgb(1.0, 1.0, 1.0)),
    box(0.0 - 4.5, 2.5, 0.0 - 3.0, 4.0, 4.5, Color.rgb(0.8, 0.9, 1.0)),
    box(0.0 - 0.5, 0.0 - 1.0, 0.5, 0.0, 1.0, Color.rgb(1.0, 0.95, 0.7)),
  ])

let stampedBox =
  Scene.cube() |> Scene.scaleXYZ(3.0, 3.0, 2.0) |> Scene.translate(Vec3.make(3.0, 1.5, 1.0))
    |> Scene.litWorldTexture(wall, 1.0)

let init = {}

let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera3D.firstPerson(Vec3.make(0.0, 3.0, 0.0 - 8.0), Angle.radians(0.0), Angle.radians(0.0 - 0.25), Angle.degrees(60.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(24.0) |> Scene.litWorldTexture(paving, 0.5),
      instancedBoxes,
      stampedBox,
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
      Light.directional(Vec3.make(0.4, 0.0 - 1.0, 0.6), Color.rgb(1.0, 0.97, 0.9), 0.9),
    ])
