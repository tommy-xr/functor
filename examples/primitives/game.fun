// examples/primitives — the Functor Lang port of examples/primitives (F#), the
// asset-free lit-pipeline scene: ground + three primitives under a shadow-
// casting sun, with two colored point lights orbiting (emissive markers at
// their positions). All animation derives from tts, so it renders
// deterministically under --fixed-time — golden-comparable against the F#
// original (docs/functor-lang.md C4b).
//
// Transform-order note: F#'s `x |> translateX a |> scale s` right-multiplies
// (scale hits the vertex first); Functor Lang transforms apply OUTERMOST-LAST, so the
// same composition reads `x |> Scene.scale(s) |> Scene.translate(a, …)`.

let tau = 6.2831853
let orbitRadius = 3.2

let pointPos = (i: float, tts: float) =>
  let a = tts * 0.6 + i * (tau / 2.0) in
  { x: Math.cos(a) * orbitRadius, y: 2.2, z: Math.sin(a) * orbitRadius }

// Near-white lit surfaces so the colored point lights tint them.
let whiteShapes = () =>
  Scene.group([
    Scene.sphere() |> Scene.scale(0.8) |> Scene.translate(-2.2, 0.8, 0.0),
    Scene.cube() |> Scene.translate(2.2, 0.5, 0.0),
    Scene.cylinder() |> Scene.translate(0.0, 0.5, 2.2),
  ]) |> Scene.lit(Color.rgb(0.9, 0.9, 0.9))

// An emissive marker sphere at a point light's position.
let marker = (i: float, tts: float, r: float, g: float, b: float) =>
  let p = pointPos(i, tts) in
  Scene.sphere()
    |> Scene.scale(0.15)
    |> Scene.emissive(Color.rgb(r, g, b))
    |> Scene.translate(p.x, p.y, p.z)

let pointLight = (i: float, tts: float, r: float, g: float, b: float) =>
  let p = pointPos(i, tts) in
  Light.point(p.x, p.y, p.z, Color.rgb(r, g, b), 1.4, 4.0)

let init = {}

let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera.firstPerson(
      0.0, 3.5, -8.0,
      Angle.radians(0.0), Angle.radians(-0.3), Angle.degrees(60.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(24.0) |> Scene.lit(Color.rgb(0.6, 0.6, 0.62)),
      whiteShapes(),
      marker(0.0, tts, 1.0, 0.3, 0.25),
      marker(1.0, tts, 0.35, 0.5, 1.0),
    ]),
    [
      Light.ambient(Color.rgb(0.1, 0.1, 0.13)),
      Light.directional(0.5, -1.0, 0.35, Color.rgb(1.0, 0.98, 0.95), 0.85) |> Light.castShadows,
      pointLight(0.0, tts, 1.0, 0.3, 0.25),
      pointLight(1.0, tts, 0.35, 0.5, 1.0),
    ])
