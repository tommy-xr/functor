// examples/atmosphere — distance fog + a cubemap skybox.
//
// A colonnade of identical pillars recedes into linear fog: every pillar has
// the same size and albedo, so any visible difference between near and far
// ones IS the fog. The fog color is tuned toward the sky's horizon band, so
// the far pillars dissolve into the sky. An emissive drifter weaves through
// the colonnade — fog occludes glow too (but not the sky: it IS the
// horizon). All animation derives from tts, so captures are deterministic
// under --fixed-time.
//
// The skybox faces are not checked in — run `npm run fetch:assets` first.
// While they load, the fog color shows in place of the sky.

let fog = Fog.linear(6.0, 26.0, Color.rgb(0.93, 0.95, 0.96))

let sky = Skybox.files(
  "TropicalSunnyDay_px.jpg", "TropicalSunnyDay_nx.jpg",
  "TropicalSunnyDay_py.jpg", "TropicalSunnyDay_ny.jpg",
  "TropicalSunnyDay_pz.jpg", "TropicalSunnyDay_nz.jpg")

// One pillar at depth i (world z = i * 3), staggered left/right.
let pillar = (i: float) =>
  Scene.cube()
    |> Scene.lit(Color.rgb(0.75, 0.55, 0.4))
    |> Scene.translate(Vec3.make(Math.sin(i * 1.7) * 4.0, 0.5, i * 3.0))

// An emissive glow weaving through the colonnade.
let drifter = (tts: float) =>
  Scene.sphere()
    |> Scene.scale(0.5)
    |> Scene.emissive(Color.rgb(1.0, 0.6, 0.2))
    |> Scene.translate(Vec3.make(Math.cos(tts * 0.6) * 3.0, 1.4, 8.0 + Math.sin(tts * 0.6) * 7.0))

let lights = () => [
  Light.ambient(Color.rgb(0.16, 0.17, 0.2)),
  Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.96, 0.88), 0.85) |> Light.castShadows,
]

let init = {}
let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 2.4, -6.0), Vec3.make(0.0, 1.0, 8.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(70.0) |> Scene.lit(Color.rgb(0.5, 0.55, 0.52)),
      Scene.group([0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0] |> List.map(pillar)),
      drifter(tts),
    ]),
    lights())
  |> Frame.withFog(fog)
  |> Frame.withSkybox(sky)
