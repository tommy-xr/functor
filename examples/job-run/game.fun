// examples/job-run — a Mandelbrot height field computed on a job worker.
// `field` is ~100k interpreted calls; `Job.run` runs it off the frame, so the
// spinner keeps turning while it computes, and its bar fills with the job's
// `Job.report`s (watched with `Job.progress`). `f` must be a top-level function:
// the worker compiles this project and calls it by name. Press N for the next
// view (cancel-then-run replaces the job in flight). Scrub back and the
// recorded field and progress come back with the model; the job never runs
// again.

type view = { cx: float, cy: float, span: float }
type Msg = | Computed(view: float, r: Result.t<List<float>, Job.error>) | Progressed(p: float)

let size = 32.0
let maxIter = 40.0
let views = [
  { cx: -0.6, cy: 0.0, span: 3.0 },
  { cx: -0.745, cy: 0.11, span: 0.25 },
  { cx: -1.25, cy: 0.02, span: 0.12 }
]

// How many steps until `z² + c` escapes, capped at `maxIter`.
let escape = (cr, ci) =>
  let start = { zr: 0.0, zi: 0.0, n: 0.0, out: false } in
  let step = (s, i) =>
    if s.out then s
    else
      let zr = s.zr * s.zr - s.zi * s.zi + cr in
      let zi = 2.0 * s.zr * s.zi + ci in
      if zr * zr + zi * zi > 4.0 then { s with out: true }
      else { zr: zr, zi: zi, n: s.n + 1.0, out: false } in
  List.fold(step, start, List.range(maxIter)).n

// The job: every cell's escape count, row by row, reporting the share done.
let field = (v: view): List<float> =>
  List.range(size * size)
  |> List.map((i) =>
       let x = Math.mod(i, size) in
       let y = Math.floor(i / size) in
       Job.report((i + 1.0) / (size * size),
         escape(v.cx + (x / size - 0.5) * v.span, v.cy + (y / size - 0.5) * v.span)))

let viewAt = (n) =>
  match List.nth(n, views) with
  | Option.Some(v) => v
  | Option.None => { cx: -0.6, cy: 0.0, span: 3.0 }

// `asked` is the view last asked for; `shown` the one on screen.
let init = { view: 0.0, asked: -1.0, shown: -1.0, heights: [], status: "computing", spin: 0.0, done: 0.0 }

// Ask once per view. Cancel first, so a new view replaces one in flight.
let tick = (m, dt, tts) =>
  let next = { m with spin: m.spin + dt } in
  if m.asked == m.view then next
  else
    let ask = Job.run("field", field, viewAt(m.view), (r) => Computed(m.view, r)) in
    ({ next with asked: m.view, status: "computing", done: 0.0 }, Effect.batch([Job.cancel("field"), ask]))

// The job's progress, while one computes.
let subscriptions = (m) =>
  if m.status == "computing" then Job.progress("field", Progressed) else Sub.none()

let input = (m, key, isDown) =>
  match key with
  | Key.N => if isDown then { m with view: Math.mod(m.view + 1.0, List.length(views)) } else m
  | _ => m

let update = (m, msg) =>
  match msg with
  | Computed(view, r) =>
    (match r with
     | Result.Ok(heights) => { m with shown: view, heights: heights, status: $"view {view + 1.0}" }
     | Result.Error(e) => { m with shown: view, heights: [], status: Job.describe(e) })
  | Progressed(p) => { m with done: p }

let columns = (heights) =>
  heights
  |> List.indexedMap((i, h) =>
       let x = Math.mod(i, size) in
       let z = Math.floor(i / size) in
       Scene.cube()
       |> Scene.scaleXYZ(0.9, 0.2 + h * 0.15, 0.9)
       |> Scene.translate(Vec3.make(x - size / 2.0, (0.2 + h * 0.15) / 2.0, z - size / 2.0)))

// A turning cube over a bar that fills as the job reports.
let spinner = (m) =>
  if m.status == "computing" then
    [Scene.cube() |> Scene.rotateY(Angle.radians(m.spin * 4.0)) |> Scene.translate(Vec3.make(0.0, 8.0, 0.0)),
     Scene.cube()
     |> Scene.scaleXYZ(0.1 + m.done * 20.0, 0.4, 0.4)
     |> Scene.translate(Vec3.make((0.1 + m.done * 20.0) / 2.0 - 10.0, 6.0, 0.0))]
  else []

let draw = (m, tts) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 26.0, -30.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group(List.append(columns(m.heights), spinner(m))),
    [Light.ambient(Color.rgb(0.3, 0.3, 0.35)),
     Light.directional(Vec3.make(-0.4, -1.0, 0.3), Color.rgb(0.9, 0.85, 0.8), 1.0)])
