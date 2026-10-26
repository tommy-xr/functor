// PartViewer — the default scene for a printable part: lit, on a bed, with an
// OrbitCamera. A file that defines `part` and none of `init`/`tick`/`draw`
// runs as if it ended with
//
//   let init = PartViewer.init
//   let tick = PartViewer.tick
//   let draw = (m, tts) => PartViewer.draw(part, m, tts)
//   let mouseMove = PartViewer.mouseMove
//   let mouseButton = PartViewer.mouseButton
//   let mouseWheel = PartViewer.mouseWheel
//
// Paste those lines into the file to change any of them.

let init = { orbit: OrbitCamera.init }

let tick = (m, dt: float, tts: float) => { m with orbit: OrbitCamera.tick(m.orbit, dt) }

let mouseMove = (m, x: float, y: float) => { m with orbit: OrbitCamera.move(m.orbit, x, y) }

let mouseButton = (m, button: Mouse.t, isDown: bool) =>
  if button == Mouse.Left then { m with orbit: OrbitCamera.press(m.orbit, isDown) } else m

let mouseWheel = (m, delta: float) => { m with orbit: OrbitCamera.scroll(m.orbit, delta) }

// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

// Part space (mm, Z up) to scene space (metres, Y up), like Solid.toScene.
let toScene = (v: Vec3.t) => Vec3.make(Vec3.x(v) / 1000.0, Vec3.z(v) / 1000.0, 0.0 - Vec3.y(v) / 1000.0)

let draw = (part: Solid.t, m, tts: float) =>
  let b = Solid.bounds(part) in
  let lo = toScene(b.min) in
  let hi = toScene(b.max) in
  let center = Vec3.scale(0.5, Vec3.add(lo, hi)) in
  let size = Vec3.length(Vec3.sub(hi, lo)) in
  Frame.createLit(
    OrbitCamera.camera(m.orbit, center, size),
    Scene.group([
      Scene.plane()
        |> Scene.scale(size * 4.0)
        |> Scene.translate(Vec3.make(Vec3.x(center), Vec3.y(lo), Vec3.z(center)))
        |> Scene.lit(Color.rgb(0.55, 0.56, 0.6)),
      Solid.toScene(part) |> Scene.lit(Color.rgb(0.25, 0.5, 0.85)),
    ]),
    [
      Light.ambient(Color.rgb(0.18, 0.18, 0.22)),
      Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.97, 0.92), 0.9),
      // A low fill from the other side, so walls aren't black.
      Light.directional(Vec3.make(-0.6, -0.2, -0.5), Color.rgb(0.75, 0.8, 1.0), 0.45),
    ])
