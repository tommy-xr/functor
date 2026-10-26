// A 4 km × 4 km terrain at night, crossed on foot by flashlight: the
// reference scene for a dark open-world VR game.
//
// One unshadowed spot light rides the camera and points where it looks (on
// XR, where the headset looks); a dim blue moon and ambient light the relief
// beyond its cone until a dense night fog swallows everything by `fogEnd`.
// The fog also bounds the work: the camera's far plane sits at `fogEnd`, the
// renderer skips terrain past it, and props past it are never submitted.
// Trees, fallen logs, and boulders are scattered from a seed, stood on the
// ground by raycasting the same heightfield the terrain draws, and thinned by
// biome: dense on the low ground, sparse toward the treeline, none on cliffs.
//
//   npm run fetch:assets                                # the Poly Haven props
//   node examples/night-terrain/generate-heightmap.mjs  # checked in; to regenerate
//   node examples/night-terrain/generate-impostors.mjs  # checked in; to regenerate
//   functor -d examples/night-terrain run native

// ── Tunables (hot-reload: save and the scene updates in place) ──────────────

// Clear to `fogStart`, fully black by `fogEnd`. The background clears to the
// same black, so nothing past `fogEnd` can be told apart from it.
let fogStart = 20.0
let fogEnd = 200.0
let fogColor = Color.rgb(0.0, 0.0, 0.0)

let flashlightCone = Angle.degrees(20.0)
let flashlightRange = 60.0
let flashlightIntensity = 2.6
let flashlightColor = Color.rgb(1.0, 0.92, 0.78)

let moonIntensity = 0.55
let moonColor = Color.rgb(0.55, 0.66, 1.0)
let moonDirection = Vec3.make(0.62, -0.42, 0.66)
let ambientColor = Color.rgb(0.036, 0.042, 0.066)

let candidateCount = 900.0
let propSeed = 23.0
// Candidates spread over this square, centred on the view below the spawn.
let scatterSize = 900.0
let groveCount = 16.0
let groveRadius = 55.0
// Trees nearer than this draw as a real trunk-and-branch mesh under a
// three-plane foliage card; farther ones as a light two-plane card alone.
let meshDistance = 40.0
// No prop within this radius of the spawn, so the first frame is unobstructed.
let spawnClearance = 8.0
// Scattered props also keep out of this wider clearing, leaving the opening
// view to the hand-placed landmarks.
let clearingRadius = 15.0

// ── World ───────────────────────────────────────────────────────────────────

let worldSize = 4000.0
let minHeight = 0.0
let maxHeight = 600.0
let snowHeight = 430.0
// Above this fraction of the height range, trees give way to rock and snow.
let treeline = 0.5
let walkSpeed = 6.0
let sensitivity = 0.003
let pitchLimit = 1.45
let eyeHeight = 1.7
// A hillside looking north, down across the hills to the mountains.
let spawnX = 50.0
let spawnZ = 0.0 - 400.0
let scatterX = spawnX
let scatterZ = spawnZ + 190.0

// The renderer blends low → high by height, rock by slope, and snow above
// `snowHeight`; the heightmap is shaped so those bands ARE the biomes.
let world =
  Terrain.heightmap(Assets.night_heightmap, worldSize, worldSize, minHeight, maxHeight)
  // 4 px fits the Quest vertex budget; 8 swims in stereo.
  |> Terrain.maxPixelError(4.0)
  |> Terrain.layered(
       Color.rgb(0.20, 0.27, 0.14),
       Color.rgb(0.36, 0.33, 0.22),
       Color.rgb(0.34, 0.33, 0.32),
       Color.rgb(0.86, 0.89, 0.93),
       snowHeight)
  // 4 m tiles: fine enough that the flashlight shows structure at 1–10 m.
  |> Terrain.textured(
       Assets.night_detail_valley,
       Assets.night_detail_hills,
       Assets.night_detail_rock,
       Assets.night_detail_snow,
       4.0)

let terrainBody = Physics.tag("terrain")

let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0),
                [Physics.heightfield(terrainBody, world)])

// Straight down through the whole height range. Misses off the map, and
// everywhere before the heightfield's first physics step.
let groundAt = (x, z) =>
  Physics.cast(Vec3.make(x, maxHeight + 10.0, z),
               Vec3.make(0.0, -1.0, 0.0),
               maxHeight - minHeight + 20.0)

// ── Props ───────────────────────────────────────────────────────────────────

type PropKind =
  | Tree
  | Log
  | Boulder

// Every prop gathers around one of the groves. Grove 0 is pinned around the
// spawn — the clearance turns it into a clearing.
let groveCentre = (g) =>
  if g == 0.0 then { x: spawnX, z: spawnZ + 12.0 }
  else
    let limit = scatterSize / 2.0 - 2.0 * groveRadius in
    let s0 = Random.seed(propSeed) |> Random.fork(0.0 - 1.0 - g) in
    let (x, s1) = Random.range(0.0 - limit, limit, s0) in
    let (z, _) = Random.range(0.0 - limit, limit, s1) in
    { x: scatterX + x, z: scatterZ + z }

// One candidate, drawn from its own forked stream so changing the count adds
// or removes candidates without reshuffling the others. `keep` is compared
// against the biome density once the ground under it is known.
let scatterProp = (i) =>
  let s0 = Random.seed(propSeed) |> Random.fork(i) in
  let (pick, s1) = Random.step(s0) in
  let kind = if pick < 0.88 then Tree else if pick < 0.94 then Log else Boulder in
  let centre = groveCentre(Math.mod(i, groveCount)) in
  // Boulders spill twice as far, so groves sit among scattered rock.
  let spread = if kind == Boulder then 2.0 * groveRadius else groveRadius in
  // Uniform over a disc: radius ∝ √u.
  let (angle, s2) = Random.range(0.0, 2.0 * Math.pi, s1) in
  let (u, s3) = Random.step(s2) in
  let (yaw, s4) = Random.range(0.0, 2.0 * Math.pi, s3) in
  let (size, s5) = Random.range(0.8, 1.5, s4) in
  let (keep, s6) = Random.step(s5) in
  // Which model of its kind (see `variantOf`).
  let (variant, _) = Random.step(s6) in
  { kind: kind,
    x: centre.x + Math.cos(angle) * spread * Math.sqrt(u),
    z: centre.z + Math.sin(angle) * spread * Math.sqrt(u),
    yaw: yaw, size: size, keep: keep, variant: variant }

// Index of a prop's model among `count` variants of its kind.
let variantOf = (count, p) => Math.min(count - 1.0, Math.floor(p.variant * count))

let near = (a, b, d) => (a.x - b.x) * (a.x - b.x) + (a.z - b.z) * (a.z - b.z) < d * d
let spawn = { x: spawnX, z: spawnZ }

// Hand-placed alongside the scatter: a fallen log and a boulder in the
// flashlight's opening view, the way a level designer dresses a spawn.
// `keep: 0.0` survives any density; they still sit on the raycast ground.
let landmarks = [
  { kind: Log, x: spawnX - 1.5, z: spawnZ + 9.5, yaw: 0.35, size: 1.6, keep: 0.0, variant: 0.0 },
  { kind: Boulder, x: spawnX + 4.0, z: spawnZ + 12.5, yaw: 0.6, size: 1.0, keep: 0.0, variant: 0.0 },
]

// Top-level, so it is computed once per load — and again on every hot reload,
// which is what lets a seed or count edit re-scatter a running game.
let scattered =
  List.range(candidateCount)
  |> List.map(scatterProp)
  |> List.filter((p) => not near(p, spawn, clearingRadius))
  |> List.append(landmarks)

expect scattered |> List.all((p) => not near(p, spawn, spawnClearance))
expect List.length(scattered) > candidateCount * 0.9
expect scattered |> List.all((p) => variantOf(3.0, p) >= 0.0 && variantOf(3.0, p) <= 2.0)
expect scattered |> List.all((p) =>
  Math.abs(p.x - scatterX) <= scatterSize / 2.0 && Math.abs(p.z - scatterZ) <= scatterSize / 2.0)

// How likely a candidate survives at normalized height `h` on ground whose
// normal has vertical component `up`: forest thins toward the treeline and
// never grows on cliffs; logs lie flat, so only on near-level ground (under
// ~12°); boulders lie everywhere, most thickly up high.
let density = (kind, h, up) =>
  let belowTreeline = 1.0 - Math.smoothstep(treeline * 0.6, treeline, h) in
  match kind with
  | Tree => 0.8 * belowTreeline * Math.smoothstep(0.82, 0.92, up)
  | Log => 0.7 * belowTreeline * Math.smoothstep(0.96, 0.98, up)
  | Boulder => 0.55 + 0.4 * Math.smoothstep(0.2, 0.5, h)

expect density(Tree, 0.1, 1.0) > density(Tree, 0.4, 1.0)
expect density(Tree, 0.1, 0.6) == 0.0
expect density(Tree, treeline, 1.0) == 0.0

// Everything placement reads, so a hot reload that edits any of these
// re-settles the props. (An edit to `density`'s body needs a restart.)
let placementKey = {
  seed: propSeed, count: candidateCount, size: scatterSize, groves: groveCount,
  radius: groveRadius, clearing: clearingRadius, treeline: treeline,
  low: minHeight, high: maxHeight, landmarks: landmarks,
}

// Ground heights need the physics world, so props settle in `tick` once per
// placement key, as soon as the heightfield exists (the spawn probe hits).
let settle = (model) =>
  let ready = groundAt(spawnX, spawnZ) in
  if model.settledFor == Option.Some(placementKey) || not ready.hit then model
  else
    let placed =
      scattered
      |> List.map((p) =>
           let hit = groundAt(p.x, p.z) in
           let h = (hit.y - minHeight) / (maxHeight - minHeight) in
           { kind: p.kind, x: p.x, y: hit.y, z: p.z, yaw: p.yaw, size: p.size, variant: p.variant,
             keep: hit.hit && p.keep < density(p.kind, h, hit.ny) })
      |> List.filter((p) => p.keep) in
    let trees = placed |> List.filter((p) => p.kind == Tree) in
    { model with
        settledFor: Option.Some(placementKey),
        trees: trees,
        forestAt: Option.None,
        // A log within its half-length (~2.1 m per unit size) of a trunk would
        // pass through it.
        logs: placed |> List.filter((p) =>
          p.kind == Log && not (trees |> List.any((t) => near(p, t, 2.2 * p.size)))),
        boulders: placed |> List.filter((p) => p.kind == Boulder) }

// ── Input and movement ──────────────────────────────────────────────────────

type MousePos =
  | NoMouse
  | MouseAt(x: float, y: float)

let init = {
  held: { up: false, down: false, left: false, right: false },
  yaw: 0.2,
  pitch: 0.0 - 0.12,
  lastMouse: NoMouse,
  eye: { x: spawnX, y: 150.0, z: spawnZ },
  head: Option.None,
  settledFor: Option.None,
  trees: [],
  logs: [],
  boulders: [],
  // The trees split by fir variant and mesh/card distance, the logs and
  // boulders short of the fog, and where the eye stood when they were split
  // (see `regroup`).
  forest: [],
  shownLogs: [],
  shownBoulders: [],
  forestAt: Option.None,
}

let setHeld = (held, key, isDown) =>
  match key with
  | Key.W => { held with up: isDown }
  | Key.Up => { held with up: isDown }
  | Key.S => { held with down: isDown }
  | Key.Down => { held with down: isDown }
  | Key.A => { held with left: isDown }
  | Key.Left => { held with left: isDown }
  | Key.D => { held with right: isDown }
  | Key.Right => { held with right: isDown }
  | _ => held

let input = (model, key, isDown) => { model with held: setHeld(model.held, key, isDown) }

let mouseMove = (model, x, y) =>
  match model.lastMouse with
  | NoMouse => { model with lastMouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    { model with
        yaw: model.yaw - (x - lastX) * sensitivity,
        pitch: Math.clamp(0.0 - pitchLimit, pitchLimit, model.pitch - (y - lastY) * sensitivity),
        lastMouse: MouseAt(x, y) }

// The headset's rig-local head pose (None off XR). On XR the camera composes
// head motion itself; the flashlight needs the pose to follow it.
let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.None => { model with head: Option.None }
  | Option.Some(xr) => { model with head: xr.head }

// Where the camera points: yaw 0 / pitch 0 looks down +Z, positive pitch up.
let lookDir = (yaw, pitch) =>
  let flat = Math.cos(pitch) in
  Vec3.make(Math.sin(yaw) * flat, Math.sin(pitch), Math.cos(yaw) * flat)

let axis = (negative, positive) =>
  (if positive then 1.0 else 0.0) - (if negative then 1.0 else 0.0)

// A kinematic walker: steer in XZ, then snap the eye to the ground below.
let tick = (model, dt, tts) =>
  let forward = axis(model.held.down, model.held.up) in
  let right = axis(model.held.left, model.held.right) in
  let step =
    Vec3.make(forward * Math.sin(model.yaw) - right * Math.cos(model.yaw),
              0.0,
              forward * Math.cos(model.yaw) + right * Math.sin(model.yaw))
    |> Vec3.normalize
    |> Vec3.scale(walkSpeed * dt) in
  // Stop a metre short of the map edge, where the ground probe would miss.
  let edge = worldSize / 2.0 - 1.0 in
  let x = Math.clamp(0.0 - edge, edge, model.eye.x + Vec3.x(step)) in
  let z = Math.clamp(0.0 - edge, edge, model.eye.z + Vec3.z(step)) in
  let ground = groundAt(x, z) in
  let y = if ground.hit then ground.y + eyeHeight else model.eye.y in
  regroup(settle({ model with eye: { x: x, y: y, z: z } }))

// ── Rendering ───────────────────────────────────────────────────────────────

// Every prop is a `Scene.model` template, which the renderer hardware-
// instances: one draw per mesh primitive, however many copies. The models are
// Poly Haven photoscans fetched by `npm run fetch:assets`; the fir cards are
// baked from the same firs by `generate-impostors.mjs` (see ASSETS.md).
//
// A fir is three models deep. Its needles are dense geometry no Quest budget
// survives, and the renderer has no alpha cutout for cards, so the foliage is
// a card whose silhouette is cut out in geometry. Near, the real trunk and
// branches stand inside a three-plane card; far, a two-plane card alone.
let firs = [
  { mesh: Assets.fir_a, near: Assets.impostor_fir_a_near, far: Assets.impostor_fir_a_far },
  { mesh: Assets.fir_b, near: Assets.impostor_fir_b_near, far: Assets.impostor_fir_b_far },
  { mesh: Assets.fir_c, near: Assets.impostor_fir_c_near, far: Assets.impostor_fir_c_far },
]
let logs = [Assets.log, Assets.stump]
let rocks = [Assets.rock_a, Assets.rock_b, Assets.rock_c, Assets.rock_d]

// Draw calls per copy-set: the fir meshes carry two primitives (live and dead
// branches); every other model, one.
let firMeshPrimitives = 2.0

let placeAt = (p, lift, scale) =>
  Instance.trs(Vec3.make(p.x, p.y + lift, p.z), Angle.radians(p.yaw), scale, scale, scale)

// One instanced node per model, holding the props that drew that variant.
let byVariant = (models, props, place) =>
  models
  |> List.indexedMap((i, m) =>
       Scene.model(m)
       |> Scene.instanced(props
                          |> List.filter((p) => variantOf(List.length(models), p) == i)
                          |> List.map(place)))

// Trees sink a little so slopes never show a gap under the trunk.
let treeAt = (p) => placeAt(p, 0.0 - 0.15, p.size)
// The log is 3 m and the stump half a metre: both read better a size up.
let logAt = (p) => placeAt(p, 0.0 - 0.05, 1.4 * p.size)
// Rock origins sit mid-height, so a little of each stays buried.
let rockAt = (p) => placeAt(p, 0.15, p.size)

let isNear = (eye, p) => near(p, eye, meshDistance)

// Split the trees by variant and by mesh/card distance, and drop every prop
// past the fog. Splitting walks every prop, so `tick` redoes it only after the
// eye moves `regroupStep` metres — near trees switch to cards up to that much
// late.
let regroupStep = 4.0
// No prop reaches further than this from its origin.
let propRadius = 4.0
// Short of the fog from anywhere within `regroupStep` of the split's eye.
let isShown = (eye, p) => near(p, eye, fogEnd + regroupStep + propRadius)
let regroup = (model) =>
  let fresh = match model.forestAt with
    // The tunables key the split too, so hot-reloading either applies.
    | Option.Some(at) =>
      at.distance == meshDistance && at.fogEnd == fogEnd && near(at, model.eye, regroupStep)
    | Option.None => false in
  if fresh then model
  else
    let shown = (props) => props |> List.filter((p) => isShown(model.eye, p)) in
    let trees = shown(model.trees) in
    { model with
        forestAt: Option.Some({ x: model.eye.x, z: model.eye.z, distance: meshDistance, fogEnd: fogEnd }),
        // One { near, far } per entry of `firs`, in order.
        forest: List.range(List.length(firs)) |> List.map((i) =>
          let mine = trees |> List.filter((t) => variantOf(List.length(firs), t) == i) in
          { near: mine |> List.filter((t) => isNear(model.eye, t)),
            far: mine |> List.filter((t) => not isNear(model.eye, t)) }),
        shownLogs: shown(model.logs),
        shownBoulders: shown(model.boulders) }

let forest = (groups) =>
  groups
  |> List.zip(firs)
  |> List.concatMap((pair) =>
       let (group, fir) = pair in
       let close = group.near |> List.map(treeAt) in
       [ Scene.model(fir.mesh) |> Scene.instanced(close),
         Scene.model(fir.near) |> Scene.instanced(close),
         Scene.model(fir.far) |> Scene.instanced(group.far |> List.map(treeAt)) ])

let draw = (model, tts) =>
  let eye = Vec3.make(model.eye.x, model.eye.y, model.eye.z) in
  let camera =
    Camera3D.firstPerson(eye, Angle.radians(model.yaw), Angle.radians(model.pitch), Angle.degrees(68.0))
    // Nothing past the fog's end can show, so the far plane stops there.
    |> Camera3D.clip(0.3, fogEnd) in
  // On XR the beam follows the tracked head, mapped through the same camera.
  let (from, dir) =
    match model.head with
    | Option.None => (eye, lookDir(model.yaw, model.pitch))
    | Option.Some(pose) =>
      let m = Camera3D.mapTrackedPose(camera, pose) in
      (Vec3.make(m.position.x, m.position.y, m.position.z),
       Vec3.make(m.forward.x, m.forward.y, m.forward.z)) in
  // Held just below the eye, so the beam's falloff reads across the ground.
  let flashlight =
    Light.spot(from |> Vec3.add(Vec3.make(0.0, -0.25, 0.0)), dir,
               flashlightColor, flashlightIntensity, flashlightRange, flashlightCone) in
  Frame.createLit(
    camera,
    // The props rebuild only when they re-settle or regroup, or on a hot
    // reload — every other frame reuses last frame's nodes and GPU buffers.
    Scene.group([
      Scene.terrain(world),
      Scene.memo("forest", model.forest, (groups) => Scene.group(forest(groups))),
      Scene.memo("logs", model.shownLogs, (props) => Scene.group(byVariant(logs, props, logAt))),
      Scene.memo("rocks", model.shownBoulders, (props) => Scene.group(byVariant(rocks, props, rockAt))),
    ]),
    [
      Light.ambient(ambientColor),
      Light.directional(moonDirection, moonColor, moonIntensity),
      flashlight,
    ])
  |> Frame.withFog(Fog.linear(fogStart, fogEnd, fogColor))

// How many instanced props use each of `count` variants.
let perVariant = (count, props) =>
  List.range(count) |> List.map((i) =>
    props |> List.filter((p) => variantOf(count, p) == i) |> List.length)

let nonEmpty = (counts) => counts |> List.filter((n) => n > 0.0) |> List.length

// The cost gauge: copies drawn, and the draw calls they take (terrain aside).
let ui = (model) =>
  let close = model.forest |> List.map((g) => List.length(g.near)) in
  let distant = model.forest |> List.map((g) => List.length(g.far)) in
  let draws =
    nonEmpty(close) * (firMeshPrimitives + 1.0)
    + nonEmpty(distant)
    + nonEmpty(perVariant(List.length(logs), model.shownLogs))
    + nonEmpty(perVariant(List.length(rocks), model.shownBoulders)) in
  let instances =
    2.0 * List.sum(close) + List.sum(distant) + List.length(model.shownLogs) + List.length(model.shownBoulders) in
  Ui.text($"Night terrain · WASD + mouse · {List.sum(close) + List.sum(distant)}/{List.length(model.trees)} trees ({List.sum(close)} mesh, {List.sum(distant)} card) · {List.length(model.shownLogs)} logs · {List.length(model.shownBoulders)} rocks · {instances} instances · {draws} draws")
  |> Ui.panel(Ui.topLeft())
