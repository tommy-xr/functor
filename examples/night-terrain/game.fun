// A 4 km × 4 km terrain at night, crossed on foot by flashlight: the
// reference scene for a dark open-world VR game.
//
// One unshadowed spot light rides the camera and points where it looks (on
// XR, where the headset looks); a dim blue moon and ambient light the relief
// beyond its cone, out to a ridged, snow-capped range that reads against the
// sky. Trees, fallen logs, and boulders are scattered from a seed, stood on the
// ground by raycasting the same heightfield the terrain draws, and thinned by
// biome: dense on the low ground, sparse toward the treeline, none on cliffs.
//
//   node examples/night-terrain/generate-heightmap.mjs
//   functor -d examples/night-terrain run native

// ── Tunables (hot-reload: save and the scene updates in place) ──────────────

let fogDensity = 0.0006
let fogColor = Color.rgb(0.030, 0.038, 0.066)
// A touch brighter than the fogged distance, so ridgelines stand out dark.
let skyColor = Color.rgb(0.040, 0.050, 0.088)

let flashlightCone = Angle.degrees(20.0)
let flashlightRange = 60.0
let flashlightIntensity = 2.6
let flashlightColor = Color.rgb(1.0, 0.92, 0.78)

let moonIntensity = 0.55
let moonColor = Color.rgb(0.55, 0.66, 1.0)
let moonDirection = Vec3.make(0.62, -0.42, 0.66)
let ambientColor = Color.rgb(0.036, 0.042, 0.066)

let candidateCount = 150.0
let propSeed = 23.0
// Candidates spread over this square, centred on the view below the spawn.
let scatterSize = 480.0
let groveCount = 7.0
let groveRadius = 38.0
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
  Terrain.heightmap(Asset.texture("night-heightmap.png"), worldSize, worldSize, minHeight, maxHeight)
  |> Terrain.maxPixelError(2.0)
  |> Terrain.layered(
       Color.rgb(0.20, 0.27, 0.14),
       Color.rgb(0.36, 0.33, 0.22),
       Color.rgb(0.34, 0.33, 0.32),
       Color.rgb(0.86, 0.89, 0.93),
       snowHeight)
  // 4 m tiles: fine enough that the flashlight shows structure at 1–10 m.
  |> Terrain.textured(
       Asset.texture("night-detail-valley.jpg"),
       Asset.texture("night-detail-hills.jpg"),
       Asset.texture("night-detail-rock.jpg"),
       Asset.texture("night-detail-snow.jpg"),
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
  let kind = if pick < 0.66 then Tree else if pick < 0.76 then Log else Boulder in
  let centre = groveCentre(Math.mod(i, groveCount)) in
  // Boulders spill twice as far, so groves sit among scattered rock.
  let spread = if kind == Boulder then 2.0 * groveRadius else groveRadius in
  // Uniform over a disc: radius ∝ √u.
  let (angle, s2) = Random.range(0.0, 2.0 * Math.pi, s1) in
  let (u, s3) = Random.step(s2) in
  let (yaw, s4) = Random.range(0.0, 2.0 * Math.pi, s3) in
  let (size, s5) = Random.range(0.7, 1.4, s4) in
  let (keep, s6) = Random.step(s5) in
  let (shade, _) = Random.range(0.75, 1.15, s6) in
  { kind: kind,
    x: centre.x + Math.cos(angle) * spread * Math.sqrt(u),
    z: centre.z + Math.sin(angle) * spread * Math.sqrt(u),
    yaw: yaw, size: size, keep: keep, shade: shade }

let near = (a, b, d) => (a.x - b.x) * (a.x - b.x) + (a.z - b.z) * (a.z - b.z) < d * d
let spawn = { x: spawnX, z: spawnZ }

// Hand-placed alongside the scatter: a fallen log and a boulder in the
// flashlight's opening view, the way a level designer dresses a spawn.
// `keep: 0.0` survives any density; they still sit on the raycast ground.
let landmarks = [
  { kind: Log, x: spawnX - 1.5, z: spawnZ + 9.5, yaw: 0.35, size: 0.8, keep: 0.0, shade: 0.9 },
  { kind: Boulder, x: spawnX + 4.0, z: spawnZ + 12.5, yaw: 0.6, size: 0.8, keep: 0.0, shade: 1.0 },
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
           { kind: p.kind, x: p.x, y: hit.y, z: p.z, yaw: p.yaw, size: p.size, shade: p.shade,
             keep: hit.hit && p.keep < density(p.kind, h, hit.ny) })
      |> List.filter((p) => p.keep) in
    let trees = placed |> List.filter((p) => p.kind == Tree) in
    { model with
        settledFor: Option.Some(placementKey),
        trees: trees,
        // A log within its half-length of a trunk would pass through it.
        logs: placed |> List.filter((p) =>
          p.kind == Log && not (trees |> List.any((t) => near(p, t, 3.0 * p.size)))),
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
  settle({ model with eye: { x: x, y: y, z: z } })

// ── Rendering ───────────────────────────────────────────────────────────────

// A unit cube stood on a corner: its body diagonal is vertical, so it is
// pointed top and bottom. Stretched tall it is the nearest thing to a cone the
// primitives offer; squashed it is a faceted rock. Still one leaf under
// transforms, so it hardware-instances.
let facet = (color) =>
  Scene.cube()
  |> Scene.lit(color)
  |> Scene.rotateX(Angle.degrees(45.0))
  |> Scene.rotateZ(Angle.degrees(35.26))

// Three templates, three instanced draws: wood (standing trunks and fallen
// logs), needles (every canopy tier), and stone. Per-copy tints vary them.
// Wood is a box, not a cylinder: lit materials carry a fixed specular, and a
// smooth cylinder catches it along its whole length as a polished-pipe streak.
let wood = Scene.cube() |> Scene.lit(Color.rgb(0.11, 0.08, 0.06))
let needles = facet(Color.rgb(0.09, 0.17, 0.11))
let stone = facet(Color.rgb(0.34, 0.34, 0.35))

let grey = (v) => Color.rgb(v, v, v)

// Unit cube: 1 on a side, centred.
let trunkAt = (p) =>
  Instance.trs(Vec3.make(p.x, p.y + 2.5 * p.size, p.z), Angle.radians(p.yaw),
               0.35 * p.size, 5.6 * p.size, 0.35 * p.size)

// A spruce: four pointed tiers, each sunk well into the one below so the
// silhouette steps in to a tip, the lowest hiding most of the trunk. Each
// tier is twisted a little against the last so the facets don't line up.
let firTiers = [
  { radius: 2.7, height: 1.8, lift: 2.3, twist: 0.0 },
  { radius: 2.1, height: 1.7, lift: 3.5, twist: 0.35 },
  { radius: 1.5, height: 1.6, lift: 4.7, twist: 0.7 },
  { radius: 0.85, height: 1.5, lift: 5.9, twist: 1.05 },
]

let tiersAt = (p) =>
  firTiers
  |> List.map((t) =>
       Instance.trs(Vec3.make(p.x, p.y + t.lift * p.size, p.z), Angle.radians(p.yaw + t.twist),
                    t.radius * p.size, t.height * p.size, t.radius * p.size)
       |> Instance.tint(Color.rgb(p.shade, p.shade * 1.05, p.shade)))

let logAt = (p) =>
  Instance.at(Vec3.make(p.x, p.y + 0.25 * p.size, p.z))
  |> Instance.scaleXYZ(0.45 * p.size, 5.0 * p.size, 0.45 * p.size)
  // An edge up along its length, then laid down and turned.
  |> Instance.rotateY(Angle.degrees(45.0))
  |> Instance.rotateZ(Angle.degrees(90.0))
  |> Instance.rotateY(Angle.radians(p.yaw))
  |> Instance.tint(grey(p.shade * 0.8))

let boulderAt = (p) =>
  Instance.at(Vec3.make(p.x, p.y + 0.2 * p.size, p.z))
  |> Instance.scaleXYZ(2.0 * p.size, 1.1 * p.size * p.shade, 1.6 * p.size)
  // Squashed and tipped at its own angles, so no two sit alike.
  |> Instance.rotateZ(Angle.radians((p.shade - 0.95) * 2.5))
  |> Instance.rotateX(Angle.radians(0.5 - p.size * 0.4))
  |> Instance.rotateY(Angle.radians(p.yaw))
  |> Instance.tint(grey(p.shade))

let draw = (model, tts) =>
  let eye = Vec3.make(model.eye.x, model.eye.y, model.eye.z) in
  let camera =
    Camera3D.firstPerson(eye, Angle.radians(model.yaw), Angle.radians(model.pitch), Angle.degrees(68.0))
    |> Camera3D.clip(0.3, 6000.0) in
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
    Scene.group([
      Scene.terrain(world),
      wood |> Scene.instanced(
                (model.trees |> List.map(trunkAt)) |> List.append(model.logs |> List.map(logAt))),
      needles |> Scene.instanced(model.trees |> List.concatMap(tiersAt)),
      stone |> Scene.instanced(model.boulders |> List.map(boulderAt)),
    ]),
    [
      Light.ambient(ambientColor),
      Light.directional(moonDirection, moonColor, moonIntensity),
      flashlight,
    ])
  |> Frame.withFog(Fog.exp(fogDensity, fogColor))
  |> Frame.withClearColor(skyColor)

let ui = (model) =>
  let trees = List.length(model.trees) in
  let instances =
    trees * (1.0 + List.length(firTiers)) + List.length(model.logs) + List.length(model.boulders) in
  Ui.text($"Night terrain · WASD + mouse · {trees} trees · {List.length(model.logs)} logs · {List.length(model.boulders)} boulders · {instances} instances")
  |> Ui.panel(Ui.topLeft())
