// orbit.fun — a small scene built to make the live values legible. A ring of
// glowing orbs circles a pulsing core; each orb's angle, height, and colour are
// computed from the model's clock, so pausing and inspecting shows exactly the
// numbers driving what you see on screen.

let count = 6.0

// One orb. Its angle sweeps around the ring (advancing with time), it bobs on a
// sine, and its hue steps around the circle — every value here shows up inline
// when the scene is paused.
let orb = (t: float, i: float) =>
  let angle = i / count * 6.2832 + t * 0.7 in
  let height = Math.sin(t * 1.6 + i * 1.1) * 1.3 in
  let hue = i / count in
  Scene.sphere()
    |> Scene.emissive(Color.rgb(0.35 + 0.55 * hue, 0.85 - 0.5 * hue, 0.95 - 0.3 * hue))
    |> Scene.scale(0.55)
    |> Scene.translate(Vec3.make(Math.cos(angle) * 3.2, height, Math.sin(angle) * 3.2))

let ring = (t) =>
  Scene.group(List.range(count) |> List.map((i) => orb(t, i)))

let core = (t) =>
  Scene.sphere()
    |> Scene.emissive(Color.rgb(1.0, 0.45 + 0.35 * Math.sin(t * 2.0), 0.75))
    |> Scene.scale(0.9 + 0.18 * Math.sin(t * 2.0))

let ground = () =>
  Scene.plane()
    |> Scene.color(Color.rgb(0.05, 0.02, 0.11))
    |> Scene.scale(40.0)
    |> Scene.translate(Vec3.make(0.0, -1.8, 0.0))

let init = { t: 0.0 }

let tick = (model, dt: float, tts: float) => { model with t: model.t + dt }

let draw = (model, tts: float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 2.6, -7.5), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([ground(), core(model.t), ring(model.t)]))
