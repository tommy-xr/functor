// Lightmap.unwrap on a room: an indexed box seen from inside, a turned
// crate and a 16-sided column, unwrapped at 16 texels a metre and drawn
// with a checker lightmap of 4-texel squares. Equal squares on every
// surface are equal texel density; each flat face is its own chart.

// A box's eight corners (index = x + 2y + 4z), centred on (cx, cz), turned
// `a` radians about Y.
let boxCorners = (cx: float, cz: float, w: float, h: float, d: float, a: float): List<float> =>
  List.range(8.0) |> List.concatMap((i) =>
    let x = (Math.mod(i, 2.0) - 0.5) * w in
    let y = Math.mod(Math.floor(i / 2.0), 2.0) * h in
    let z = (Math.floor(i / 4.0) - 0.5) * d in
    [cx + x * Math.cos(a) - z * Math.sin(a), y, cz + x * Math.sin(a) + z * Math.cos(a)])

// Its faces as corner quads, wound to face inward (a room) or outward.
let inward = [(0.0, 2.0, 6.0, 4.0), (1.0, 5.0, 7.0, 3.0), (0.0, 4.0, 5.0, 1.0), (2.0, 3.0, 7.0, 6.0), (0.0, 1.0, 3.0, 2.0), (4.0, 6.0, 7.0, 5.0)]
let outward = List.map((q) => let (a, b, c, d) = q in (d, c, b, a), inward)

// A floor-to-ceiling column: a ring of 16 at y = 0, then at y = 3.
let column = List.range(32.0) |> List.concatMap((k) =>
  let a = Math.mod(k, 16.0) / 16.0 * 2.0 * Math.pi in
  [0.0 - 1.6 + 0.4 * Math.cos(a), if k < 16.0 then 0.0 else 3.0, 0.0 - 1.2 + 0.4 * Math.sin(a)])
let columnQuads = List.range(16.0) |> List.map((i) => (i, 16.0 + i, 16.0 + Math.mod(i + 1.0, 16.0), Math.mod(i + 1.0, 16.0)))

let quadTris = (base: float, q): List<float> => let (a, b, c, d) = q in [a, b, c, a, c, d] |> List.map((v) => base + v)
let positions = List.flatten([boxCorners(0.0, 0.0, 6.0, 3.0, 6.0, 0.0), boxCorners(1.0, 0.0 - 1.0, 1.0, 1.0, 1.0, 0.5), column])
let triangles = List.flatten([
  List.concatMap((q) => quadTris(0.0, q), inward),
  List.concatMap((q) => quadTris(8.0, q), outward),
  List.concatMap((q) => quadTris(16.0, q), columnQuads)
])

// Shared corners: 48 vertices in, one per chart corner out.
let unwrapped = Lightmap.unwrap({
  positions: Bytes.fromF32le(positions),
  // Only the lightmap lights this room, so normals go unused.
  normals: Bytes.fromF32le(List.range(48.0) |> List.concatMap((i) => [0.0, 1.0, 0.0])),
  uvs: Bytes.fromF32le([]),
  uvs2: Bytes.fromF32le([]),
  triangles: Bytes.fromU32le(triangles),
  groups: [],
  lightmap: Option.None
}, { texelsPerMeter: 16.0, padding: 2.0, maxSize: 1024.0 })

// 4-texel squares (25 cm here) over the whole atlas.
let shade = (i: float, j: float): List<float> =>
  if Math.mod(Math.floor(i / 4.0) + Math.floor(j / 4.0), 2.0) == 0.0 then [230.0, 230.0, 220.0] else [60.0, 70.0, 90.0]
let checker = Texture.fromPixels({
  width: unwrapped.width, height: unwrapped.height, format: Texture.Rgb8, colorSpace: Texture.Linear,
  pixels: Bytes.fromU8(List.range(unwrapped.height) |> List.concatMap((j) => List.range(unwrapped.width) |> List.concatMap((i) => shade(i, j))))
})

let mesh = unwrapped.mesh
let room = Model.static({ mesh with lightmap: Option.Some({ texture: checker, scale: 1.0, floor: Color.rgb(0.0, 0.0, 0.0) }) })

let init = {}

let tick = (model, dt: float, tts: float) => model

let camera = Camera3D.lookAt(Vec3.make(2.8, 2.6, 2.9), Vec3.make(0.0 - 0.8, 0.5, 0.0 - 1.6))

let draw = (model, tts: float) =>
  Frame.createLit(camera, Scene.modelOf(room), [Light.ambient(Color.rgb(1.0, 1.0, 1.0)) |> Light.baked])
