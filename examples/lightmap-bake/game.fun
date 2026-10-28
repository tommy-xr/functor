// A Cornell box lit by `functor bake`: one lamp with hard shadows, plus
// ambient light darkened into corners and under the blocks (occlusion).
// `bakes` below is what the baker reads; it writes baked/room.png (light)
// and baked/room.mesh.bin (the mesh with lightmap UVs), and room-ao, ambient
// alone. Rebake after an edit:
//   functor -d examples/lightmap-bake bake --texels-per-meter 32
//
// The view cycles every 3 s: baked, occlusion only, realtime (no lightmap:
// the engine has no point-light shadows). 1, 2 and 3 pick one.

let v = Vec3.make

// A quad from corner `o` along `a`, then `b`; it faces `a × b`.
type quad = { o: Vec3.t, a: Vec3.t, b: Vec3.t }
let quad = (o: Vec3.t, a: Vec3.t, b: Vec3.t): quad => { o: o, a: a, b: b }

// The room's inside, open at z = 0 (the camera's side).
let floor = quad(v(0.0 - 2.0, 0.0, 0.0), v(4.0, 0.0, 0.0), v(0.0, 0.0, 0.0 - 4.0))
let ceiling = quad(v(0.0 - 2.0, 3.0, 0.0 - 4.0), v(4.0, 0.0, 0.0), v(0.0, 0.0, 4.0))
let back = quad(v(0.0 - 2.0, 0.0, 0.0 - 4.0), v(4.0, 0.0, 0.0), v(0.0, 3.0, 0.0))
let left = quad(v(0.0 - 2.0, 0.0, 0.0), v(0.0, 0.0, 0.0 - 4.0), v(0.0, 3.0, 0.0))
let right = quad(v(2.0, 0.0, 0.0 - 4.0), v(0.0, 0.0, 4.0), v(0.0, 3.0, 0.0))

// A `size` block standing at (x, z), turned `turn` radians about Y: its top
// and four sides, facing out.
let block = (x: float, z: float, size: Vec3.t, turn: float): List<quad> =>
  let ex = v(Math.cos(turn) * Vec3.x(size), 0.0, Math.sin(turn) * Vec3.x(size)) in
  let ez = v(0.0 - Math.sin(turn) * Vec3.z(size), 0.0, Math.cos(turn) * Vec3.z(size)) in
  let ey = v(0.0, Vec3.y(size), 0.0) in
  let c = v(x, 0.0, z) |> Vec3.sub(Vec3.scale(0.5, ex |> Vec3.add(ez))) in
  [quad(c |> Vec3.add(ey), ez, ex), quad(c, ez, ey), quad(c |> Vec3.add(ex), ey, ez),
   quad(c, ey, ex), quad(c |> Vec3.add(ez), ex, ey)]

// White surfaces first, then the red and green walls: one group each.
let white = List.flatten([[floor, ceiling, back],
  block(0.7, 0.0 - 1.4, v(1.1, 1.1, 1.1), 0.0 - 0.3),
  block(0.0 - 0.7, 0.0 - 2.6, v(1.1, 2.2, 1.1), 0.35)])
let quads = List.flatten([white, [left], [right]])

let components = (p: Vec3.t): List<float> => [Vec3.x(p), Vec3.y(p), Vec3.z(p)]
let corners = (q: quad): List<Vec3.t> =>
  [q.o, q.o |> Vec3.add(q.a), q.o |> Vec3.add(q.a) |> Vec3.add(q.b), q.o |> Vec3.add(q.b)]
let facing = (q: quad): Vec3.t => q.a |> Vec3.cross(q.b) |> Vec3.normalize

// Classic Cornell albedos, as sRGB.
let solid = (r: float, g: float, b: float) => Option.Some(Texture.fromPixels({
  width: 1.0, height: 1.0, format: Texture.Rgb8, colorSpace: Texture.Srgb, pixels: Bytes.fromU8([r, g, b]) }))
let group = (first: float, count: float, texture) => { first: first, count: count, texture: texture, unlit: false }
let whiteCount = 2.0 * List.length(white)
let groups = [
  group(0.0, whiteCount, solid(223.0, 223.0, 223.0)),
  group(whiteCount, 2.0, solid(207.0, 72.0, 63.0)),
  group(whiteCount + 2.0, 2.0, solid(105.0, 179.0, 85.0))
]

let roomData = {
  positions: Bytes.fromF32le(quads |> List.concatMap((q) => corners(q) |> List.concatMap(components))),
  normals: Bytes.fromF32le(quads |> List.concatMap((q) => [0.0, 1.0, 2.0, 3.0] |> List.concatMap((i) => components(facing(q))))),
  // The textures are one texel each, so any UVs do.
  uvs: Bytes.fromF32le(quads |> List.concatMap((q) => [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])),
  uvs2: Bytes.fromF32le([]),
  triangles: Bytes.fromU32le(List.range(List.length(quads)) |> List.concatMap((k) =>
    [0.0, 1.0, 2.0, 0.0, 2.0, 3.0] |> List.map((i) => 4.0 * k + i))),
  groups: groups,
  lightmap: Option.None
}

// The lights, marked `Light.baked`: lightmapped surfaces already hold them.
let lampAt = v(0.0, 2.6, 0.0 - 2.0)
let ambient = Light.ambient(Color.rgb(0.35, 0.35, 0.35))
let lamp = Light.point(lampAt, Color.rgb(1.0, 0.92, 0.8), 1.6, 5.0) |> Light.castShadows
let bakedLights = [ambient |> Light.baked, lamp |> Light.baked]

let bakes = [
  { name: "room", mesh: roomData, lights: bakedLights },
  { name: "room-ao", mesh: roomData, lights: [Light.ambient(Color.rgb(1.0, 1.0, 1.0))] }
]

// The baked room, once `functor bake` has run.
let load = (name: string) =>
  let mesh = Lightmap.readMesh(File.bytes($"baked/{name}.mesh.bin")) in
  Model.static({ mesh with groups: groups, lightmap: Option.Some({
    texture: Texture.fromPng(File.bytes($"baked/{name}.png"), Texture.Linear),
    scale: 1.0, floor: Color.rgb(0.0, 0.0, 0.0) }) })
let baked = if File.exists("baked/room.png") && File.exists("baked/room-ao.png")
  then Option.Some((load("room"), load("room-ao")))
  else Option.None
let realtime = Model.static(roomData)

// 0: cycle, else the view picked with 1–3.
let init = { pick: 0.0 }

let input = (model, key, isDown: bool) =>
  if not isDown then model
  else match key with
    | Key.Num1 => { model with pick: 1.0 }
    | Key.Num2 => { model with pick: 2.0 }
    | Key.Num3 => { model with pick: 3.0 }
    | _ => model

let tick = (model, dt: float, tts: float) => model

let camera = Camera3D.lookAt(v(0.0, 1.5, 4.4), v(0.0, 1.4, 0.0 - 2.0))

let lampBall = Scene.sphere() |> Scene.scale(0.1) |> Scene.translate(lampAt) |> Scene.emissive(Color.rgb(1.0, 0.92, 0.8))
let unbaked = Frame.createLit(camera, Scene.group([Scene.modelOf(realtime), lampBall]), [ambient, lamp])

let draw = (model, tts: float) =>
  let view = if model.pick > 0.0 then model.pick else 1.0 + Math.mod(Math.floor(tts / 3.0), 3.0) in
  match baked with
    | Option.None => unbaked
    | Option.Some(models) =>
      let (lit, ao) = models in
      if view == 1.0 then Frame.createLit(camera, Scene.group([Scene.modelOf(lit), lampBall]), bakedLights)
      else if view == 2.0 then Frame.createLit(camera, Scene.modelOf(ao), bakedLights)
      else unbaked
