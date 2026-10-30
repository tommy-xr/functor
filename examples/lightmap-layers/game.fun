// Lightmap layers: a room whose two lamps are baked as switchable layers of
// one atlas. A flickering lamp's level follows the clock; Space toggles the
// other. The levels are model state, mixed in by `Texture.withLayers` and
// drawn with `Scene.withLightmap`, so scrubbing time replays them exactly.

let ambient = [0.06, 0.06, 0.08]
let color = (c) => match c with | [r, g, b] => Color.rgb(r, g, b) | _ => Color.rgb(0.0, 0.0, 0.0)

// Each lamp: a layer id, a place, a linear colour and a range.
let lamp = (id: float, at, color, range: float) => { id: id, at: at, color: color, range: range }
let flicker = lamp(1.0, Vec3.make(0.0 - 1.1, 2.5, 0.0 - 3.2), [1.0, 0.7, 0.4], 4.5)
let switched = lamp(2.0, Vec3.make(1.3, 1.6, 0.0 - 1.2), [0.5, 0.75, 1.0], 4.0)
let lamps = [flicker, switched]

// Each face: a corner, two edges (counter-clockwise seen from inside) and
// its inward normal. Floor first, then the walls and ceiling.
let face = (o, a, b, n) => { o: o, a: a, b: b, n: n }
let faces = [
  face(Vec3.make(0.0 - 2.0, 0.0, 0.0), Vec3.make(4.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 0.0 - 4.0), Vec3.make(0.0, 1.0, 0.0)),
  face(Vec3.make(0.0 - 2.0, 0.0, 0.0 - 4.0), Vec3.make(4.0, 0.0, 0.0), Vec3.make(0.0, 3.0, 0.0), Vec3.make(0.0, 0.0, 1.0)),
  face(Vec3.make(0.0 - 2.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 0.0 - 4.0), Vec3.make(0.0, 3.0, 0.0), Vec3.make(1.0, 0.0, 0.0)),
  face(Vec3.make(2.0, 0.0, 0.0 - 4.0), Vec3.make(0.0, 0.0, 4.0), Vec3.make(0.0, 3.0, 0.0), Vec3.make(0.0 - 1.0, 0.0, 0.0)),
  face(Vec3.make(0.0 - 2.0, 3.0, 0.0 - 4.0), Vec3.make(4.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 4.0), Vec3.make(0.0, 0.0 - 1.0, 0.0))
]

let at = (f, s: float, t: float) => f.o |> Vec3.add(Vec3.scale(s, f.a)) |> Vec3.add(Vec3.scale(t, f.b))

// Corners and edges darken over the last half metre.
let occlusion = (f, s: float, t: float): float =>
  let a = Vec3.length(f.a) in
  let b = Vec3.length(f.b) in
  let edge = Math.min(Math.min(s * a, (1.0 - s) * a), Math.min(t * b, (1.0 - t) * b)) in
  0.45 + 0.55 * Math.smoothstep(0.0, 0.5, edge)

// One lamp's linear light at a point: the engine's falloff (1 - d²/r²)²
// times Lambert, in each channel.
let lampLight = (l, p, n) =>
  let toLamp = l.at |> Vec3.sub(p) in
  let d = Vec3.length(toLamp) in
  let fall = Math.clamp01(1.0 - d * d / (l.range * l.range)) in
  let k = Math.max(0.0, Vec3.dot(n, Vec3.scale(1.0 / d, toLamp))) * fall * fall in
  l.color |> List.map((c) => c * k)

// A 16×16 linear image of `light(p, n)` over a face, darkened by occlusion.
let size = 16.0
let bake = (f, light) =>
  Bytes.fromU8(List.range(size) |> List.concatMap((j) =>
    List.range(size) |> List.concatMap((i) =>
      let s = i / (size - 1.0) in
      let t = j / (size - 1.0) in
      let ao = occlusion(f, s, t) in
      light(at(f, s, t), f.n) |> List.map((c) => Math.round(255.0 * Math.clamp01(ao * c))))))

// The base is ambient only; each lamp is a layer.
let image = (f) => {
  pixels: {
    width: size, height: size, format: Texture.Rgb8, colorSpace: Texture.Linear,
    pixels: bake(f, (p, n) => ambient)
  },
  layers: lamps |> List.map((l) => { id: l.id, pixels: bake(f, (p, n) => lampLight(l, p, n)) })
}

let lm = Texture.atlas({ images: List.map(image, faces), padding: 1.0, maxSize: 256.0 })

let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
let perCorner = (f) => (List.zip(lm.rects, faces) |> List.concatMap((pair) =>
  let (face, rect) = pair in
  corners |> List.concatMap((c) => f(face, rect, c))))

let positions = perCorner((face, rect, c) => let (s, t) = c in let p = at(face, s, t) in [Vec3.x(p), Vec3.y(p), Vec3.z(p)])
let normals = perCorner((face, rect, c) => [Vec3.x(face.n), Vec3.y(face.n), Vec3.z(face.n)])
let uvs = perCorner((face, rect, c) => let (s, t) = c in [s * Vec3.length(face.a), t * Vec3.length(face.b)])
let uvs2 = perCorner((face, rect, c) => let (s, t) = c in [rect.u0 |> Math.lerp(rect.u1, s), rect.v0 |> Math.lerp(rect.v1, t)])
let triangles = List.range(5.0) |> List.concatMap((k) => [0.0, 1.0, 2.0, 0.0, 2.0, 3.0] |> List.map((i) => 4.0 * k + i))

let rgb = (w, h, bytes) => Texture.fromPixels({ width: w, height: h, format: Texture.Rgb8, colorSpace: Texture.Srgb, pixels: Bytes.fromU8(bytes) })
let tile = (i: float, j: float): List<float> =>
  if Math.mod(Math.floor(i / 4.0) + Math.floor(j / 4.0), 2.0) == 0.0 then [200.0, 200.0, 190.0] else [90.0, 90.0, 100.0]
let checker = rgb(8.0, 8.0, List.range(8.0) |> List.concatMap((j) => List.range(8.0) |> List.concatMap((i) => tile(i, j))))
let plaster = rgb(1.0, 1.0, [225.0, 215.0, 200.0])

// The model's own lightmap is the atlas as is: every layer off.
let room = Model.static({
  positions: Bytes.fromF32le(positions),
  normals: Bytes.fromF32le(normals),
  uvs: Bytes.fromF32le(uvs),
  uvs2: Bytes.fromF32le(uvs2),
  triangles: Bytes.fromU32le(triangles),
  groups: [
    { first: 0.0, count: 2.0, texture: Option.Some(checker), unlit: false },
    { first: 2.0, count: 8.0, texture: Option.Some(plaster), unlit: false }
  ],
  lightmap: Option.Some({ texture: lm.texture, scale: 1.0, floor: Color.rgb(0.0, 0.0, 0.0) })
})

// A failing tube: a hum plus short dropouts at deterministic moments.
let hash = (x: float): float =>
  let h = Math.sin(x * 12.9898) * 43758.5453 in
  h - Math.floor(h)
let flickerLevel = (tts: float): float =>
  if hash(Math.floor(tts * 9.0)) < 0.18 then 0.12 else 0.85 + 0.1 * Math.sin(tts * 31.0)

let levels = (on: bool, tts: float) =>
  Map.fromList([(flicker.id, flickerLevel(tts)), (switched.id, if on then 1.0 else 0.0)])

let init = { on: true, lights: levels(true, 0.0) }

let tick = (model, dt: float, tts: float) => { model with lights: levels(model.on, tts) }

let input = (model, key: Key.t, isDown: bool) =>
  if isDown && key == Key.Space then { model with on: if model.on then false else true } else model

let camera = Camera3D.lookAt(Vec3.make(0.0, 1.6, 2.2), Vec3.make(0.0, 1.1, 0.0 - 2.0))

let level = (model, l): float =>
  match Map.get(l.id, model.lights) with
  | Option.Some(v) => v
  | Option.None => 0.0

let bulb = (model, l) =>
  Scene.sphere() |> Scene.scale(0.12) |> Scene.translate(l.at)
    |> Scene.emissive(color(l.color |> List.map((c) => c * Math.max(0.15, level(model, l)))))

let draw = (model, tts: float) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.modelOf(room) |> Scene.withLightmap(Texture.withLayers(model.lights, lm.texture)),
      Scene.cube() |> Scene.scale(0.6) |> Scene.translate(Vec3.make(0.5, 0.3, 0.0 - 1.8)) |> Scene.lit(Color.rgb(0.8, 0.3, 0.25)),
      bulb(model, flicker),
      bulb(model, switched)
    ]),
    // In the lightmap already (`Light.baked`), but they still light the cube.
    [Light.ambient(color(ambient)) |> Light.baked]
      |> List.append(lamps |> List.map((l) => Light.point(l.at, color(l.color), 1.6 * level(model, l), l.range) |> Light.baked)))
