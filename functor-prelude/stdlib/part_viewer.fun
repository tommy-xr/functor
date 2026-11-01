// PartViewer — the default scene for a printable part: lit, on a bed, with an
// OrbitCamera; `V` toggles a PartViews sheet of eight orthographic views. A
// file that defines `part` and none of the entry points in
// `<stdlib>/PartViewerDefault.fun` runs with those seven lines; paste them
// into the file to change any of them.

let init = { orbit: OrbitCamera.init, sheet: false }

let tick = (m, dt: float, tts: float) => { m with orbit: OrbitCamera.tick(m.orbit, dt) }

let mouseMove = (m, x: float, y: float) => { m with orbit: OrbitCamera.move(m.orbit, x, y) }

let mouseButton = (m, button: Mouse.t, isDown: bool) =>
  if button == Mouse.Left then { m with orbit: OrbitCamera.press(m.orbit, isDown) } else m

let mouseWheel = (m, delta: float) => { m with orbit: OrbitCamera.scroll(m.orbit, delta) }

let input = (m, key: Key.t, isDown: bool) =>
  if key == Key.V && isDown then { m with sheet: not m.sheet } else m

// Top-level values here stay pure (no host calls): tools load this module
// without the engine.

let draw = (part: Solid.t, m, tts: float) =>
  if m.sheet then PartViews.sheet([{ solid: part, color: Color.rgb(0.25, 0.5, 0.85) }]) else
  let b = Solid.bounds(part) in
  let lo = PartViews.toScene(b.min) in
  let hi = PartViews.toScene(b.max) in
  let center = Vec3.scale(0.5, Vec3.add(lo, hi)) in
  let size = Vec3.distance(lo, hi) in
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
