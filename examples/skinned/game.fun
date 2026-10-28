// Skinned models from data: a two-bone tube built in Functor with
// `Model.skinned`, drawn three times from one GPU upload and posed by `Anim`.

// Nine square rings from y = 0 to y = 2; the elbow joint sits at y = 1.
let levels = List.range(9.0) |> List.map((k) => k * 0.25)

let corners = List.range(4.0) |> List.map((i) => Math.pi * (0.25 + 0.5 * i))

let ring = (y: float, f: (float) => List<float>): List<float> => corners |> List.concatMap(f)

let positions = levels |> List.concatMap((y) => ring(y, (a) => [0.14 * Math.cos(a), y, 0.14 * Math.sin(a)]))

let normals = levels |> List.concatMap((y) => ring(y, (a) => [Math.cos(a), 0.0, Math.sin(a)]))

// Rings near the elbow blend the two bones, so the bend stays smooth.
let lowerWeight = (y: float): float => Math.clamp01((y - 0.75) / 0.5)

let weights = levels |> List.concatMap((y) => ring(y, (a) => [1.0 - lowerWeight(y), lowerWeight(y), 0.0, 0.0]))

let joints = levels |> List.concatMap((y) => ring(y, (a) => [0.0, 1.0, 0.0, 0.0]))

// Two triangles per side per segment, counter-clockwise from outside.
let triangles =
  List.range(8.0) |> List.concatMap((k) =>
    List.range(4.0) |> List.concatMap((i) =>
      let a = k * 4.0 + i in
      let b = k * 4.0 + Math.mod(i + 1.0, 4.0) in
      [a, a + 4.0, b + 4.0, a, b + 4.0, b]))

let identity = { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }

// A 2 s loop: the lower bone swings ±60° about Z.
let frames = 16.0

let swing =
  List.range(frames) |> List.concatMap((k) =>
    let half = Math.pi / 6.0 * Math.sin(2.0 * Math.pi * k / frames) in
    [0.0, 0.0, Math.sin(half), Math.cos(half)])

let arm = Model.skinned({
  positions: Bytes.fromF32le(positions),
  normals: Bytes.fromF32le(normals),
  uvs: Bytes.fromF32le([]),
  joints: Bytes.fromU8(joints),
  weights: Bytes.fromF32le(weights),
  triangles: Bytes.fromU32le(triangles),
  skeleton: [
    { name: "upper", parent: Option.None, translation: Vec3.make(0.0, 0.0, 0.0), rotation: identity },
    { name: "lower", parent: Option.Some(0.0), translation: Vec3.make(0.0, 1.0, 0.0), rotation: identity }
  ],
  clips: [
    { name: "swing", fps: 8.0, frames: frames, tracks: [
      { joint: "lower", rotations: Bytes.fromF32le(swing), translations: Bytes.fromF32le([]) }
    ] }
  ],
  texture: Option.None
})

let init = {}

let tick = (model, dt: float, tts: float) => model

let camera = Camera3D.lookAt(Vec3.make(0.0, 1.4, 0.0 - 4.0), Vec3.make(0.0, 1.0, 0.0))

// Rest, the clip, and a blend of the clip (a second later) with a leaning
// shoulder.
let draw = (model, tts: float) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.plane() |> Scene.scale(6.0) |> Scene.lit(Color.rgb(0.42, 0.47, 0.55)),
      Scene.modelOf(arm) |> Scene.animate(Anim.rest()) |> Scene.translate(Vec3.make(1.2, 0.0, 0.0)),
      Scene.modelOf(arm) |> Scene.animate(Anim.clip("swing", tts)),
      Scene.modelOf(arm)
        |> Scene.animate(Anim.blend([
             (Anim.clip("swing", tts + 1.0), 1.0),
             (Anim.rotate("upper", 0deg, 0deg, 40deg, Anim.rest()), 1.0)
           ]))
        |> Scene.translate(Vec3.make(0.0 - 1.2, 0.0, 0.0))
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
      Light.directional(Vec3.make(0.0 - 0.5, 0.0 - 1.0, 0.4), Color.rgb(1.0, 0.96, 0.88), 1.0) |> Light.castShadows
    ])
