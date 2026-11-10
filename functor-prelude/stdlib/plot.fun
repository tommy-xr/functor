// Plot — points and lines in world space, for trails, paths and point clouds.
// Each call is one `Scene.instanced` node (a camera-facing square per point,
// a stretched box per segment), unlit in one color and casting no shadow, so
// thousands stay cheap to draw. Building the list is the cost: wrap a plot
// that rarely changes in `Scene.memo`. Sizes are world units.
//
//   Plot.polyline(0.01, Color.rgb(0.25, 0.85, 0.9), cameraPath)
//   Plot.points(0.02, Color.rgb(1.0, 1.0, 1.0), cloud)

// A camera-facing square `size` across at each point.
let points = (size: float, color: Color.t, at: List<Vec3.t>): Scene.t =>
  Scene.billboard()
  |> Scene.emissive(color)
  |> Scene.instanced(at |> List.map((p) => Instance.at(p) |> Instance.scale(size)))

// The box from `a` to `b`: unit Z stretched to their distance, pitched then
// yawed to point along them.
let segment = (width: float, ends: (Vec3.t, Vec3.t)): Instance.t =>
  let (a, b) = ends in
  let d = Vec3.sub(a, b) in
  let flat = Math.sqrt(Vec3.x(d) * Vec3.x(d) + Vec3.z(d) * Vec3.z(d)) in
  Instance.at(Vec3.lerp(a, 0.5, b))
  |> Instance.scaleXYZ(width, width, Vec3.length(d))
  |> Instance.rotateX(Angle.radians(0.0 - Math.atan2(Vec3.y(d), flat)))
  |> Instance.rotateY(Angle.radians(Math.atan2(Vec3.x(d), Vec3.z(d))))

// A square bar `width` thick between each pair of points.
let segments = (width: float, color: Color.t, pairs: List<(Vec3.t, Vec3.t)>): Scene.t =>
  Scene.cube()
  |> Scene.emissive(color)
  |> Scene.instanced(pairs |> List.map((ends) => segment(width, ends)))
  |> Scene.castShadows(false)

// `segments` joining each point to the next.
let polyline = (width: float, color: Color.t, path: List<Vec3.t>): Scene.t =>
  segments(width, color, path |> List.zip(List.drop(1.0, path)))
