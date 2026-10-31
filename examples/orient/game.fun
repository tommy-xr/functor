// Anim.orient: an arm built with `Model.skinned` reaches one target three
// ways. Left: `reach` alone, so the hand follows the forearm. Middle: `orient`
// after `reach` holds the hand upright, palm to the camera, at the same
// point. Right: the oriented hand waves about the wrist.

// The cross-section: a pentagon tube up to the wrist (y = 2), then a flat
// mitten whose thumb sticks out on +X near its base, so the hand's roll and
// side read at a glance.
let corners = List.range(5.0)

let armCorner = (i: float) =>
  let a = 2.0 * Math.pi * i / 5.0 in
  { x: 0.09 * Math.cos(a), z: 0.09 * Math.sin(a) }

let handCorner = (y: float, i: float) =>
  if i == 0.0 then { x: if y < 2.45 then 0.26 else 0.1, z: 0.0 }
  else if i == 1.0 then { x: 0.1, z: 0.04 }
  else if i == 2.0 then { x: 0.0 - 0.15, z: 0.03 }
  else if i == 3.0 then { x: 0.0 - 0.15, z: 0.0 - 0.03 }
  else { x: 0.1, z: 0.0 - 0.04 }

let corner = (y: float, i: float) => if y > 2.05 then handCorner(y, i) else armCorner(i)

let levels = List.range(15.0) |> List.map((k) => k * 0.2)

let ring = (y: float, f) => corners |> List.concatMap((i) => f(corner(y, i)))

let positions = levels |> List.concatMap((y) => ring(y, (c) => [c.x, y, c.z]))

let normals =
  levels |> List.concatMap((y) => ring(y, (c) =>
    let l = Math.sqrt(c.x * c.x + c.z * c.z) in
    [c.x / l, 0.0, c.z / l]))

// Upper arm to y = 1, forearm to y = 2, hand above; rings near each joint
// blend the two bones so the bends stay smooth.
let blend = (y: float, at: float): float => Math.clamp01((y - at + 0.2) / 0.4)

let weights =
  levels |> List.concatMap((y) => ring(y, (c) =>
    let lower = blend(y, 1.0) in
    let hand = blend(y, 2.0) in
    [1.0 - lower, lower - hand, hand, 0.0]))

let joints = levels |> List.concatMap((y) => ring(y, (c) => [0.0, 1.0, 2.0, 0.0]))

// Two triangles per side per segment, counter-clockwise from outside.
let triangles =
  List.range(14.0) |> List.concatMap((k) =>
    corners |> List.concatMap((i) =>
      let a = k * 5.0 + i in
      let b = k * 5.0 + Math.mod(i + 1.0, 5.0) in
      [a, b + 5.0, a + 5.0, a, b, b + 5.0]))

let identity = { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }

let arm = Model.skinned({
  positions: Bytes.fromF32le(positions),
  normals: Bytes.fromF32le(normals),
  uvs: Bytes.fromF32le([]),
  joints: Bytes.fromU8(joints),
  weights: Bytes.fromF32le(weights),
  triangles: Bytes.fromU32le(triangles),
  skeleton: [
    { name: "upper", parent: Option.None, translation: Vec3.make(0.0, 0.0, 0.0), rotation: identity },
    { name: "lower", parent: Option.Some(0.0), translation: Vec3.make(0.0, 1.0, 0.0), rotation: identity },
    { name: "hand", parent: Option.Some(1.0), translation: Vec3.make(0.0, 1.0, 0.0), rotation: identity }
  ],
  clips: [],
  texture: Option.None
})

let init = {}

let tick = (model, dt: float, tts: float) => model

// A model-space reach target up and toward the camera.
let target = Vec3.make(0.0 - 0.5, 1.4, 0.0 - 0.35)

// A slight elbow bend seeds the side `reach` bends to.
let reached =
  Anim.rest()
    |> Anim.rotate("lower", 0deg, 0deg, 25deg)
    |> Anim.reach("upper", "lower", "hand", target, 1.0)

// A wave: up to 60° either way about the camera axis.
let wave = (tts: float) =>
  let half = Math.pi / 6.0 * Math.sin(Math.pi * tts) in
  { x: 0.0, y: 0.0, z: Math.sin(half), w: Math.cos(half) }

let shoulder = (x: float, anim: Anim.t) =>
  Scene.group([
    Scene.modelOf(arm) |> Scene.animate(anim),
    Scene.sphere() |> Scene.scale(0.08) |> Scene.translate(target) |> Scene.lit(Color.rgb(1.0, 0.45, 0.2))
  ]) |> Scene.translate(Vec3.make(x, 0.0, 0.0))

let camera = Camera3D.lookAt(Vec3.make(0.0 - 0.4, 1.8, 0.0 - 5.6), Vec3.make(0.0 - 0.4, 1.3, 0.0))

let draw = (model, tts: float) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.plane() |> Scene.scale(7.0) |> Scene.lit(Color.rgb(0.42, 0.47, 0.55)),
      shoulder(1.7, reached),
      shoulder(0.0, reached |> Anim.orient("hand", identity, 1.0)),
      shoulder(0.0 - 1.7, reached |> Anim.orient("hand", wave(tts), 1.0))
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.25, 0.3)),
      Light.directional(Vec3.make(0.0 - 0.5, 0.0 - 1.0, 0.4), Color.rgb(1.0, 0.96, 0.88), 1.0) |> Light.castShadows
    ])
