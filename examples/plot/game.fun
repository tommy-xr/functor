// examples/plot — points and lines in world space with the `Plot` stdlib
// module: a rising spiral trail, the same path as points beside it, and the
// three axes. Everything derives from tts, so it renders deterministically
// under --fixed-time.

let spiral = (turns: float, tts: float): List<Vec3.t> =>
  List.range(120.0) |> List.map((i) =>
    let t = i / 119.0 in
    let a = t * turns * 6.2831853 + tts * 0.5 in
    Vec3.make(Math.cos(a) * (0.4 + t), t * 2.0, Math.sin(a) * (0.4 + t)))

let axis = (to: Vec3.t, color: Color.t) =>
  Plot.segments(0.02, color, [(Vec3.make(0.0, 0.0, 0.0), to)])

let init = {}
let tick = (m, dts, tts) => m

let draw = (m, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(3.5, 3.0, 5.0), Vec3.make(-0.8, 1.0, 0.0)),
    Scene.group([
      axis(Vec3.make(2.0, 0.0, 0.0), Color.rgb(0.9, 0.2, 0.2)),
      axis(Vec3.make(0.0, 2.0, 0.0), Color.rgb(0.2, 0.9, 0.2)),
      axis(Vec3.make(0.0, 0.0, 2.0), Color.rgb(0.2, 0.4, 0.9)),
      Plot.polyline(0.03, Color.rgb(0.25, 0.85, 0.9), spiral(3.0, tts)),
      Plot.points(0.06, Color.rgb(1.0, 0.8, 0.3), spiral(3.0, tts))
        |> Scene.translate(Vec3.make(-2.5, 0.0, 0.0)),
    ]))
