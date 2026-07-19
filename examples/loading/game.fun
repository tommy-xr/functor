// examples/loading — a loading screen over `Sub.assets`: models stream from
// the BabylonJS CDN while a progress bar tracks {loaded, total, failed}; when
// everything settles the bar disappears. Press SPACE to request a SECOND
// batch mid-game — the demo of late asset requests. On a fast machine (or a
// warm disk cache) loads are near-instant — simulate a slow network to
// actually watch it (native only; the value is KB/s):
//
//   FUNCTOR_THROTTLE_ASSETS=600 functor -d examples/loading run native
//
// Remote assets disk-cache at ~/.functor/cache, so point FUNCTOR_ASSET_CACHE
// at a scratch dir (or wipe the cache) to re-see the first-load experience.
//
// Loading is DECLARATIVE: an asset starts loading when `draw` references it.
// So the scene is drawn from frame one — models pop in as they arrive — and
// the bar overlays until `loaded + failed == total`. Failed loads (a bad URL)
// count toward settling, and the HUD shows them: a loading screen must end
// even when the CDN doesn't cooperate.
//
// The SPACE transition is the `Effect.preload` demo (B.5): the new batch is
// WARMED imperatively — phase 1 keeps playing untouched while the models
// stream, and `Effect.preloadThen` delivers Phase2Warmed through `update`
// when the anchor load settles, so phase 2 appears fully loaded (the
// no-loading-screen transition; draw's later references are cache hits).
// One late-request idiom remains: progress is CUMULATIVE (total never
// resets), so the warming bar subtracts the baseline captured at SPACE
// (baseLoaded / baseTotal). The old second idiom — inferring "did the
// snapshot see my new assets yet?" from totals via a transitioning flag —
// is gone: the game KNOWS it is warming (its own phase state) and is TOLD
// when it's done (the preloadThen message). (Pending effect messages reset
// on hot reload — the HTTP-tagger rule — so a reload mid-warming strands
// phase 1.5 until restart; the dev-loop tradeoff the engine documents.)

// phase: 1.0 = playing phase 1; 1.5 = SPACE pressed, phase-2 batch warming
// via Effect.preload (phase 1 still on screen); 2.0 = warmed, phase 2 drawn.
// warmed counts the batch's settlements — phase 2 draws when all three land.
let init = {
  loaded: 0.0, failed: 0.0, total: 0.0, done: false,
  phase: 1.0, warmed: 0.0,
  baseLoaded: 0.0, baseTotal: 0.0,
}

// The phase-2 batch as Asset values (the preload surface takes values, not
// strings) — another size ladder (1.1MB gull, 4.6MB plane, 11MB boombox).
let gull = Asset.model("https://assets.babylonjs.com/meshes/seagulf.glb")
let aeroplane = Asset.model("https://assets.babylonjs.com/meshes/aerobatic_plane.glb")
let boombox = Asset.model("https://assets.babylonjs.com/meshes/boombox.glb")

type Msg<'p> =
  | Progress(p: 'p)
  | Phase2Warmed

let update = (model, msg) =>
  match msg with
  // One batch member settled (loaded or failed — Sub.assets says which).
  // Phase 2 first DRAWS when all three have landed: every reference below
  // is then a cache hit, so the batch appears at once, fully loaded.
  | Phase2Warmed =>
    let warmed = model.warmed + 1.0 in
    { model with
        warmed: warmed,
        phase: (if warmed == 3.0 then 2.0 else model.phase) }
  | Progress(p) =>
    let failedCount = List.length(p.failed) in
    let settledAll = p.total > 0.0 && p.loaded + failedCount == p.total in
    { model with
        loaded: p.loaded,
        failed: failedCount,
        total: p.total,
        done: settledAll }

let subscriptions = (model) => Sub.assets(Progress)

let input = (model, key, isDown) =>
  match key with
  | Key.Space =>
    if isDown && model.phase == 1.0
    then
      // Warm the batch imperatively; each member reports its settlement,
      // and the count gates the transition. The baseline (remaining idiom)
      // gives the warming bar a fresh 0..1 range.
      ({ model with
           phase: 1.5,
           baseLoaded: model.loaded + model.failed,
           baseTotal: model.total },
       Effect.batch([
         Effect.preloadThen(gull, Phase2Warmed),
         Effect.preloadThen(aeroplane, Phase2Warmed),
         Effect.preloadThen(boombox, Phase2Warmed),
       ]))
    else model
  | _ => model

let tick = (model, dt, tts) => model

// Per-phase fraction (idiom 1): progress since the captured baseline.
let fraction = (model) =>
  if model.total == model.baseTotal then 0.0
  else
    (model.loaded + model.failed - model.baseLoaded)
      / (model.total - model.baseTotal)

// The 15MB shark shows the `Asset.whilePending` placeholder pattern: the
// 2KB box stands in (streams near-instantly) until the shark is decoded —
// under FUNCTOR_THROTTLE_ASSETS you can watch the swap. The placeholder is
// just another asset value; a FAILED shark would show the empty fallback
// (failure is not pending) and still count in Sub.assets' `failed`.
let shark =
  Asset.model("https://assets.babylonjs.com/meshes/shark.glb")
    |> Asset.whilePending(Asset.model("https://assets.babylonjs.com/meshes/box.glb"))

let models = () =>
  Scene.group([
    Scene.model("https://assets.babylonjs.com/meshes/ExplodingBarrel.glb")
      |> Scene.scale(0.35)
      |> Scene.translate(Vec3.make(0.0, -1.2, 0.0)),
    Scene.model(shark)
      |> Scene.scale(0.18)
      |> Scene.rotateY(Angle.degrees(200.0))
      |> Scene.translate(Vec3.make(-2.2, 0.7, 0.0)),
    // Deliberately mixed sizes (2KB box, 2.9MB barrel, 15MB shark): assets
    // settle in size order, so the staggered pop-in is visible.
    Scene.model("https://assets.babylonjs.com/meshes/box.glb")
      |> Scene.scale(0.8)
      |> Scene.rotateY(Angle.degrees(30.0))
      |> Scene.translate(Vec3.make(2.4, 0.2, 0.0)),
  ])

// The SPACE batch — another size ladder (1.1MB gull, 4.6MB plane, 11MB
// boombox), placed above the phase-1 row.
let phase2Models = () =>
  Scene.group([
    Scene.model(gull)
      |> Scene.scale(0.001)
      |> Scene.translate(Vec3.make(-3.0, -1.6, 0.0)),
    Scene.model(aeroplane)
      |> Scene.scale(3.0)
      |> Scene.rotateY(Angle.degrees(150.0))
      |> Scene.translate(Vec3.make(1.7, 2.0, 0.0)),
    // The glTF-sample BoomBox is authored at centimeter scale (~1cm tall).
    Scene.model(boombox)
      |> Scene.scale(22.0)
      |> Scene.rotateY(Angle.degrees(180.0))
      |> Scene.translate(Vec3.make(0.0, 1.6, 0.0)),
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
      |> Scene.color(Color.rgb(0.12, 0.12, 0.18)),
    Scene.quad()
      |> Scene.scaleXYZ(barWidth * f, 0.22, 1.0)
      |> Scene.translate(Vec3.make(barWidth * (f - 1.0) * 0.5, 0.0, 0.05))
      |> Scene.emissive(Color.rgb(0.25, 0.9, 0.55)),
  ])
    |> Scene.translate(Vec3.make(0.0, -2.3, 0.0))

let draw = (model, tts) =>
  let base = models() in
  let scene =
    if model.phase == 2.0 then Scene.group([base, phase2Models()])
    else base
  in
  // The bar shows while anything is unsettled OR the game is warming — its
  // own phase state, no snapshot inference needed.
  let withBar =
    if model.phase == 1.5 || not model.done
    then Scene.group([scene, bar(model)])
    else scene
  in
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 0.8, 6.0), Vec3.make(0.0, 0.2, 0.0)),
    withBar,
    [
      Light.ambient(Color.rgb(0.4, 0.4, 0.45)),
      Light.directional(Vec3.make(-0.4, -1.0, -0.5), Color.rgb(1.0, 0.95, 0.9), 1.1),
    ])

let ui = (model) =>
  let status =
    if model.phase == 1.5 && model.total == model.baseTotal
    then "Warming phase 2 (Effect.preload)..."
    else if model.phase == 1.5 || not model.done
    then
      Text.concat(
        "Loading ",
        Text.concat(
          Text.fixed(model.loaded + model.failed - model.baseLoaded, 0.0),
          Text.concat(
            " / ",
            Text.concat(
              Text.fixed(model.total - model.baseTotal, 0.0),
              if model.failed > 0.0
              then Text.concat("  failed: ", Text.fixed(model.failed, 0.0))
              else ""))))
    else if model.phase == 1.0
    then "Ready!  [Space: load more]"
    else "Ready!"
  in
  Ui.column([Ui.text(status)]) |> Ui.panel(Ui.center())
