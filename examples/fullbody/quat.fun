// Plain-data vectors (`Input.point3`) and quaternions (`Input.quaternion`,
// x, y, z, w), so the body state stays inspectable and replayable.

let v3 = (x: float, y: float, z: float): Input.point3 => { x: x, y: y, z: z }
let add = (a: Input.point3, b: Input.point3): Input.point3 => v3(a.x + b.x, a.y + b.y, a.z + b.z)
let sub = (a: Input.point3, b: Input.point3): Input.point3 => v3(a.x - b.x, a.y - b.y, a.z - b.z)
let scale = (k: float, a: Input.point3): Input.point3 => v3(a.x * k, a.y * k, a.z * k)
let lerp = (a: Input.point3, b: Input.point3, t: float): Input.point3 => add(a, scale(t, sub(b, a)))
let dot = (a: Input.point3, b: Input.point3): float => a.x * b.x + a.y * b.y + a.z * b.z
let length = (a: Input.point3): float => Math.sqrt(dot(a, a))
let toVec3 = (a: Input.point3) => Vec3.make(a.x, a.y, a.z)

let identity: Input.quaternion = { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }

// The Hamilton product: `mul(a, b)` applies `b` first, then `a`.
let mul = (a: Input.quaternion, b: Input.quaternion): Input.quaternion => {
  x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
  y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
  z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
  w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z
}

let conj = (q: Input.quaternion): Input.quaternion => { x: 0.0 - q.x, y: 0.0 - q.y, z: 0.0 - q.z, w: q.w }

let axisAngle = (axis: Input.point3, radians: float): Input.quaternion =>
  let s = Math.sin(radians / 2.0) in
  { x: axis.x * s, y: axis.y * s, z: axis.z * s, w: Math.cos(radians / 2.0) }

let yaw = (radians: float): Input.quaternion => axisAngle(v3(0.0, 1.0, 0.0), radians)

// v' = v + 2w(q × v) + 2 q × (q × v)
let rotate = (q: Input.quaternion, v: Input.point3): Input.point3 =>
  let tx = 2.0 * (q.y * v.z - q.z * v.y) in
  let ty = 2.0 * (q.z * v.x - q.x * v.z) in
  let tz = 2.0 * (q.x * v.y - q.y * v.x) in
  v3(
    v.x + q.w * tx + (q.y * tz - q.z * ty),
    v.y + q.w * ty + (q.z * tx - q.x * tz),
    v.z + q.w * tz + (q.x * ty - q.y * tx))

// Shortest-path normalized lerp: plenty for blending nearby rotations.
let nlerp = (a: Input.quaternion, b: Input.quaternion, t: float): Input.quaternion =>
  let s = if a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w < 0.0 then 0.0 - 1.0 else 1.0 in
  let x = a.x + (s * b.x - a.x) * t in
  let y = a.y + (s * b.y - a.y) * t in
  let z = a.z + (s * b.z - a.z) * t in
  let w = a.w + (s * b.w - a.w) * t in
  let n = Math.sqrt(x * x + y * y + z * z + w * w) in
  { x: x / n, y: y / n, z: z / n, w: w / n }

// The heading (radians about +Y, 0 = facing +Z) of a frame whose forward is
// +Z. Exact for any pitch without roll, so looking straight down still has
// one: `forward.xz * up.y - up.xz * forward.y` is the unpitched forward.
let heading = (q: Input.quaternion): float =>
  let f = rotate(q, v3(0.0, 0.0, 1.0)) in
  let u = rotate(q, v3(0.0, 1.0, 0.0)) in
  Math.atan2(f.x * u.y - u.x * f.y, f.z * u.y - u.z * f.y)

// An angle wrapped to (-pi, pi].
let wrap = (a: float): float =>
  let t = Math.mod(a + Math.pi, 2.0 * Math.pi) in
  (if t < 0.0 then t + 2.0 * Math.pi else t) - Math.pi

let near = (a: float, b: float): bool => Math.abs(a - b) < 0.0001

expect near(heading(yaw(0.7)), 0.7)
expect near(heading(mul(yaw(0.0 - 2.0), axisAngle(v3(1.0, 0.0, 0.0), 1.4))), 0.0 - 2.0)
expect near(heading(mul(yaw(2.5), axisAngle(v3(1.0, 0.0, 0.0), 0.0 - 1.5))), 2.5)
expect near(wrap(3.0 * Math.pi / 2.0), 0.0 - Math.pi / 2.0)
expect (let p = rotate(yaw(Math.pi / 2.0), v3(0.0, 0.0, 1.0)) in near(p.x, 1.0) && near(p.z, 0.0))
