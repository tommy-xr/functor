// examples/loading — a loading screen over `Sub.assets`: models stream from
// the BabylonJS CDN while a progress bar tracks {loaded, total, failed}; when
// everything settles the bar disappears. On a fast machine (or once the disk
// cache is warm) loads are near-instant — simulate a slow network to actually
// watch it (native only; the value is KB/s):
//
//   FUNCTOR_THROTTLE_ASSETS=200 functor -d examples/loading run native
//
// Remote assets disk-cache at ~/.functor/cache, so point FUNCTOR_ASSET_CACHE
// at a scratch dir (or wipe the cache) to re-see the first-load experience.
//
// Loading is DECLARATIVE: an asset starts loading when `draw` references it.
// So the scene is drawn from frame one — models pop in as they arrive — and
// the bar overlays until `loaded + failed == total`. Failed loads (a bad URL)
// count toward settling, and the HUD shows them: a loading screen must end
// even when the CDN doesn't cooperate.

let init = { loaded: 0.0, failed: 0.0, total: 0.0, done: false }

let update = (model, p) =>
  let failedCount = List.length(p.failed) in
  { loaded: p.loaded,
    failed: failedCount,
    total: p.total,
    done: p.total > 0.0 && p.loaded + failedCount == p.total }

let subscriptions = (model) => Sub.assets((p) => p)

let tick = (model, dt, tts) => model

let fraction = (model) =>
  if model.total == 0.0 then 0.0
  else (model.loaded + model.failed) / model.total

let models = () =>
  Scene.group([
    Scene.model("https://assets.babylonjs.com/meshes/ExplodingBarrel.glb")
      |> Scene.scale(0.35)
      |> Scene.translate(0.0, -1.2, 0.0),
    Scene.model("https://assets.babylonjs.com/meshes/shark.glb")
      |> Scene.scale(0.18)
      |> Scene.rotateY(Angle.degrees(200.0))
      |> Scene.translate(-2.2, 0.7, 0.0),
    // Deliberately mixed sizes (2KB box, 2.9MB barrel, 15MB shark): assets
    // settle in size order, so the staggered pop-in is visible.
    Scene.model("https://assets.babylonjs.com/meshes/box.glb")
      |> Scene.scale(0.8)
      |> Scene.rotateY(Angle.degrees(30.0))
      |> Scene.translate(2.4, 0.2, 0.0),
  ])

// A backdrop quad and a left-anchored fill quad (quads are centered, so the
// fill shifts left by half its missing width). Quad fronts face +Z — toward
// the camera below.
let barWidth = 3.0

let bar = (model) =>
  let f = fraction(model) in
  Scene.group([
    Scene.quad()
      |> Scene.scaleXYZ(barWidth + 0.15, 0.35, 1.0)
      |> Scene.color(0.12, 0.12, 0.18),
    Scene.quad()
      |> Scene.scaleXYZ(barWidth * f, 0.22, 1.0)
      |> Scene.translate(barWidth * (f - 1.0) * 0.5, 0.0, 0.05)
      |> Scene.emissive(0.25, 0.9, 0.55),
  ])
    |> Scene.translate(0.0, -2.3, 0.0)

let draw = (model, tts) =>
  let scene =
    if model.done then models()
    else Scene.group([models(), bar(model)])
  in
  Frame.createLit(
    Camera.lookAt(0.0, 0.8, 6.0, 0.0, -0.2, 0.0),
    scene,
    [
      Light.ambient(0.4, 0.4, 0.45),
      Light.directional(-0.4, -1.0, -0.5, 1.0, 0.95, 0.9, 1.1),
    ])

let ui = (model) =>
  let status =
    if model.done then "Ready!"
    else
      Text.concat(
        "Loading ",
        Text.concat(
          Text.fixed(model.loaded + model.failed, 0.0),
          Text.concat(
            " / ",
            Text.concat(
              Text.fixed(model.total, 0.0),
              if model.failed > 0.0
              then Text.concat("  failed: ", Text.fixed(model.failed, 0.0))
              else ""))))
  in
  Ui.column([Ui.text(status)]) |> Ui.panel(Ui.center())
