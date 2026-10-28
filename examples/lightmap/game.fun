// A lightmapped room built from data: `Model.static` walls in two texture
// groups, a lightmap baked here in Functor (one ceiling lamp with the
// engine's own falloff, plus soft occlusion into the corners, which no
// realtime light gives) packed with `Texture.atlas`, and a moving unbaked
// light that adds on top.

// The baked lights. `Light.baked` keeps them off the lightmapped walls,
// which already hold them, while still lighting the dynamic cube.
let ambient = Color.rgb(0.25, 0.25, 0.3)
let lampAt = Vec3.make(0.0, 2.8, 0.0 - 2.0)
let lampColor = Color.rgb(1.0, 0.85, 0.6)
let lampIntensity = 1.6
let lampRange = 6.0

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

// The point on a face at (s, t) in [0, 1]².
let at = (f, s: float, t: float) => f.o |> Vec3.add(Vec3.scale(s, f.a)) |> Vec3.add(Vec3.scale(t, f.b))

// Baked illumination at a point: ambient plus the lamp, with the engine's
// light falloff (1 - d²/r²)² and Lambert, darkened by `ao`. Linear, 0–1.
let srgbToLinear = (c: float): float => Math.pow(c, 2.2)
let lit = (p, n, ao: float, channel: float, lamp: float): float =>
  let toLamp = lampAt |> Vec3.sub(p) in
  let d = Vec3.length(toLamp) in
  let fall = Math.clamp01(1.0 - d * d / (lampRange * lampRange)) in
  let ndotl = Math.max(0.0, Vec3.dot(n, Vec3.scale(1.0 / d, toLamp))) in
  Math.clamp01(ao * (srgbToLinear(channel) + srgbToLinear(lamp) * lampIntensity * ndotl * fall * fall))

// Corners and edges darken over the last half metre.
let occlusion = (f, s: float, t: float): float =>
  let a = Vec3.length(f.a) in
  let b = Vec3.length(f.b) in
  let edge = Math.min(Math.min(s * a, (1.0 - s) * a), Math.min(t * b, (1.0 - t) * b)) in
  0.45 + 0.55 * Math.smoothstep(0.0, 0.5, edge)

// One 16×16 lightmap image per face: texel (i, j) sits at (i, j) / 15, the
// same points the atlas rect's inset UVs reach.
let size = 16.0
let image = (f) => {
  pixels: {
    width: size, height: size, format: Texture.Rgb8, colorSpace: Texture.Linear,
    pixels: Bytes.fromU8(List.range(size) |> List.concatMap((j) =>
      List.range(size) |> List.concatMap((i) =>
        let s = i / (size - 1.0) in
        let t = j / (size - 1.0) in
        let p = at(f, s, t) in
        let ao = occlusion(f, s, t) in
        [255.0 * lit(p, f.n, ao, 0.25, 1.0), 255.0 * lit(p, f.n, ao, 0.25, 0.85), 255.0 * lit(p, f.n, ao, 0.3, 0.6)]
          |> List.map(Math.round))))
  },
  layers: []
}

let baked = Texture.atlas({ images: List.map(image, faces), padding: 1.0, maxSize: 256.0 })

// Per vertex, four corners per face.
let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
let perCorner = (f) => (List.zip(baked.rects, faces) |> List.concatMap((pair) =>
  let (face, rect) = pair in
  corners |> List.concatMap((c) => f(face, rect, c))))

let positions = perCorner((face, rect, c) => let (s, t) = c in let p = at(face, s, t) in [Vec3.x(p), Vec3.y(p), Vec3.z(p)])
let normals = perCorner((face, rect, c) => [Vec3.x(face.n), Vec3.y(face.n), Vec3.z(face.n)])
// Base UVs repeat once per metre.
let uvs = perCorner((face, rect, c) => let (s, t) = c in [s * Vec3.length(face.a), t * Vec3.length(face.b)])
let uvs2 = perCorner((face, rect, c) => let (s, t) = c in [rect.u0 |> Math.lerp(rect.u1, s), rect.v0 |> Math.lerp(rect.v1, t)])
let triangles = List.range(5.0) |> List.concatMap((k) => [0.0, 1.0, 2.0, 0.0, 2.0, 3.0] |> List.map((i) => 4.0 * k + i))

// A checker floor and plain plaster, both tiny sRGB textures.
let rgb = (w, h, bytes) => Texture.fromPixels({ width: w, height: h, format: Texture.Rgb8, colorSpace: Texture.Srgb, pixels: Bytes.fromU8(bytes) })
let checker = rgb(2.0, 2.0, [200.0, 200.0, 190.0, 90.0, 90.0, 100.0, 90.0, 90.0, 100.0, 200.0, 200.0, 190.0])
let plaster = rgb(1.0, 1.0, [225.0, 215.0, 200.0])

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
  lightmap: Option.Some({ texture: baked.texture, scale: 1.0, floor: Color.rgb(0.02, 0.02, 0.03) })
})

let init = {}

let tick = (model, dt: float, tts: float) => model

let camera = Camera3D.lookAt(Vec3.make(0.0, 1.6, 2.2), Vec3.make(0.0, 1.1, 0.0 - 2.0))

// A blue unbaked light sweeps along the back wall.
let orbAt = (tts: float) => Vec3.make(1.4 * Math.sin(tts), 1.0, 0.0 - 3.3)

let draw = (model, tts: float) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.modelOf(room),
      Scene.cube() |> Scene.scale(0.6) |> Scene.translate(Vec3.make(0.6, 0.3, 0.0 - 2.2)) |> Scene.lit(Color.rgb(0.8, 0.3, 0.25)),
      Scene.sphere() |> Scene.scale(0.12) |> Scene.translate(orbAt(tts)) |> Scene.emissive(Color.rgb(0.4, 0.6, 1.0)),
      Scene.sphere() |> Scene.scale(0.15) |> Scene.translate(lampAt) |> Scene.emissive(lampColor)
    ]),
    [
      Light.ambient(ambient) |> Light.baked,
      Light.point(lampAt, lampColor, lampIntensity, lampRange) |> Light.baked,
      Light.point(orbAt(tts), Color.rgb(0.4, 0.6, 1.0), 1.5, 2.5)
    ])
