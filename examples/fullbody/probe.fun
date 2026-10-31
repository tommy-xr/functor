// Reads solved joints back out of a pose, for tests and the `/state` readout.
//
// `Model.pose` skins `Model.skinned` models only, so this builds a stand-in
// from Xbot's own bind skeleton (`skeleton.fun`): four points per joint, all
// weighted to it, at its origin and 10 cm along its local X, Y and Z axes.
// Posing it with the avatar's `Anim.t` runs the engine's evaluator on the
// same joints, so the points land on the solved joint frames. Only the
// clip differs: the stand-in's `idle` is the bind pose, and the avatar masks
// the real one to the legs, which no upper-body joint depends on.

let joints = Skeleton.joints

let axisPoints = (b: Skeleton.Bind): List<Input.point3> =>
  [Quat.v3(0.0, 0.0, 0.0), Quat.v3(0.1, 0.0, 0.0), Quat.v3(0.0, 0.1, 0.0), Quat.v3(0.0, 0.0, 0.1)]
    |> List.map((a) => Quat.add(b.position, Quat.rotate(b.rotation, a)))

let points = Skeleton.bind |> List.concatMap(axisPoints)

let standIn = Model.skinned({
  positions: Bytes.fromF32le(points |> List.concatMap((p) => [p.x, p.y, p.z])),
  normals: Bytes.fromF32le(points |> List.concatMap((p) => [0.0, 1.0, 0.0])),
  uvs: Bytes.fromF32le([]),
  joints: Bytes.fromU8(List.range(List.length(joints)) |> List.concatMap((i) => [i, 0.0, 0.0, 0.0, i, 0.0, 0.0, 0.0, i, 0.0, 0.0, 0.0, i, 0.0, 0.0, 0.0])),
  weights: Bytes.fromF32le(points |> List.concatMap((p) => [1.0, 0.0, 0.0, 0.0])),
  triangles: Bytes.fromU32le([0.0, 1.0, 2.0]),
  skeleton: joints,
  clips: [{ name: "idle", fps: 30.0, frames: 1.0, tracks: [] }],
  texture: Option.None
})

/// A solved joint: its model-space origin and unit local axes.
type Frame = { position: Input.point3, x: Input.point3, y: Input.point3, z: Input.point3 }

let indexOf = (name: string): float =>
  joints
    |> List.indexedMap((i, j) => (i, j.name))
    |> List.find((p) => let (_, n) = p in n == name)
    |> Option.map((p) => let (i, _) = p in i)
    |> Option.defaultValue(0.0 - 1.0)

let point = (posed: Bytes.t, k: float): Input.point3 =>
  let f = (o: float) => Bytes.f32le(k * 12.0 + o, posed) |> Result.defaultValue(0.0) in
  Quat.v3(f(0.0), f(4.0), f(8.0))

let frameAt = (posed: Bytes.t, i: float): Frame =>
  let o = point(posed, i * 4.0) in
  let axis = (k: float) => Quat.sub(point(posed, i * 4.0 + k), o) |> Quat.scale(10.0) in
  { position: o, x: axis(1.0), y: axis(2.0), z: axis(3.0) }

/// Pose the stand-in once.
let pose = (anim: Anim.t): Bytes.t => Model.pose(anim, standIn)

/// One named joint's solved frame from a posed stand-in.
let frame = (posed: Bytes.t, name: string): Frame => frameAt(posed, indexOf(name))

// The stand-in's rest pose reproduces the generated bind positions.
expect (
  let posed = pose(Anim.rest()) in
  Skeleton.bind |> List.all((b) =>
    Quat.length(Quat.sub(frame(posed, b.name).position, b.position)) < 0.00001)
)
