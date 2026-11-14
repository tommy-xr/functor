// city-lights — a grid of instanced towers lit by waves of light. Point at a
// tower to light it; click (or pull the trigger in VR) to pick it.
//
// The towers are ONE `Scene.instancedPaint` node over a lit cube. Their
// placements are a top-level list built once; the paint bytes (16 per tower:
// tint r, g, b and glow) are rebuilt only when the light changes, so the GPU
// re-uploads 16 bytes per tower and never the transforms. Picking is one
// `Instance.raycast` against that same list — no physics bodies.
//
// Space freezes the waves: the city is then the same `Scene.memo` value
// until the hover or pick changes, which costs nothing to convert or upload.
//
//   functor -d examples/city-lights run native
//   functor -d examples/city-lights run vr
//   functor -d examples/city-lights test

// Towers per side: 64 is 4,096 towers; 100 is 10,000.
let side = 64.0
let spacing = 1.2

// ---- The city (built once) ---------------------------------------------------

let cells = List.range(side * side)

let column = (i: float): float => Math.mod(i, side)
let row = (i: float): float => Math.floor(i / side)

// A stable pseudo-random 0–1 per tower.
let hash = (i: float): float =>
  let s = Math.sin(i * 12.9898) * 43758.5453 in
  s - Math.floor(s)

// Taller toward the middle, with noise.
let distance = (i: float): float =>
  let x = column(i) - (side - 1.0) / 2.0 in
  let z = row(i) - (side - 1.0) / 2.0 in
  Math.sqrt(x * x + z * z)

let height = (i: float): float =>
  let centre = Math.max(0.0, 1.0 - distance(i) / (side * 0.6)) in
  0.3 + (0.4 + 3.6 * centre * centre) * (0.35 + 0.65 * hash(i))

let towers = cells |> List.map((i) =>
  let h = height(i) in
  Instance.trs(
    Vec3.make((column(i) - (side - 1.0) / 2.0) * spacing, h / 2.0,
      (row(i) - (side - 1.0) / 2.0) * spacing),
    Angle.degrees(0.0), 0.9, h, 0.9))

// (distance from the middle, index) per tower.
let lots = List.zip(cells, cells |> List.map(distance))

let camera =
  Camera3D.lookAt(Vec3.make(0.0, side * spacing * 0.5, side * spacing * 0.85),
    Vec3.make(0.0, 0.0, 0.0))
  |> Camera3D.clip(0.5, 400.0)

// ---- The light ---------------------------------------------------------------

// Two rings run outward from the middle; each tower glows as one passes.
let glowAt = (d: float, tts: float): float =>
  let wave = 0.5 + 0.5 * Math.sin(d * 0.45 - tts * 2.4) in
  wave * wave * wave * wave

// A glowing tower turns from cool blue to amber and stops taking the light;
// the hovered one burns white, the picked one cyan.
let paintFor = (d: float, i: float, tts: float, hovered: float, picked: float): List<float> =>
  if i == picked then [0.35, 1.0, 1.0, 1.0]
  else if i == hovered then [1.0, 1.0, 1.0, 1.0]
  else
    let g = glowAt(d, tts) in
    [Math.lerp(1.0, g, 0.45), Math.lerp(0.72, g, 0.55), Math.lerp(0.3, g, 1.0), g]

let paintsAt = (tts: float, hovered: float, picked: float): Bytes.t =>
  Bytes.fromF32le(lots |> List.concatMap((lot) =>
    match lot with
    | (d, i) => paintFor(d, i, tts, hovered, picked)))

// ---- Pointing ----------------------------------------------------------------

let point = (p: Input.point3) => Vec3.make(p.x, p.y, p.z)

// The right controller's aim in VR, else the mouse.
let aimRay = (snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.Some(xr) =>
    xr.right.aim |> Option.map((aim) =>
      let pose = Camera3D.mapTrackedPose(camera, aim) in
      { origin: point(pose.position), direction: point(pose.forward) })
  | Option.None => Camera3D.toWorldRay(snapshot.mouse, camera)

// The tower under the aim, or -1.
let hoveredBy = (snapshot: Input.snapshot): float =>
  aimRay(snapshot)
  |> Option.bind((ray) => Instance.raycast(ray, towers))
  |> Option.map((hit) => hit.index)
  |> Option.defaultValue(-1.0)

let triggerHeld = (snapshot: Input.snapshot): bool =>
  match snapshot.xr with
  | Option.Some(xr) => xr.right.trigger > 0.5
  | Option.None => false

// ---- The game ----------------------------------------------------------------

type Model = { now: float, frozen: bool, hovered: float, picked: float, trigger: bool }

let init: Model = { now: 0.0, frozen: false, hovered: -1.0, picked: -1.0, trigger: false }

let tick = (model: Model, dt: float, tts: float) =>
  if model.frozen then model else { model with now: tts }

let sampledInput = (model: Model, snapshot: Input.snapshot) =>
  let hovered = hoveredBy(snapshot) in
  let trigger = triggerHeld(snapshot) in
  let clicked = snapshot.mouse.pressed.left || (trigger && not model.trigger) in
  let picked = if clicked then (if hovered == model.picked then -1.0 else hovered) else model.picked in
  { model with hovered: hovered, picked: picked, trigger: trigger }

let input = (model: Model, key, isDown: bool) =>
  if isDown && key == Key.Space then { model with frozen: not model.frozen } else model

// The towers for one light: rebuilt only when the wave time, hover or pick
// changes (`Scene.memo`), so frozen they convert and upload nothing.
type Light = { t: float, hovered: float, picked: float }

let lit = (light: Light) =>
  Scene.cube()
  |> Scene.lit(Color.rgb(0.85, 0.85, 0.9))
  |> Scene.instancedPaint(towers, paintsAt(light.t, light.hovered, light.picked))

let draw = (model: Model, tts: float) =>
  let city =
    Scene.memo("city", { t: model.now, hovered: model.hovered, picked: model.picked }, lit) in
  let ground =
    Scene.plane()
    |> Scene.scaleXYZ(side * spacing + 4.0, 1.0, side * spacing + 4.0)
    |> Scene.lit(Color.rgb(0.05, 0.05, 0.08)) in
  Frame.createLit(
    camera,
    Scene.group([ground, city]),
    [
      Light.ambient(Color.rgb(0.12, 0.12, 0.16)),
      Light.directional(Vec3.make(0.4, -1.0, 0.25), Color.rgb(0.8, 0.85, 1.0), 0.6)
        |> Light.castShadows,
    ])
  |> Frame.withClearColor(Color.rgb(0.02, 0.02, 0.05))

// ---- Tests -------------------------------------------------------------------

let down = { origin: Vec3.make(spacing / 2.0, 50.0, spacing / 2.0), direction: Vec3.make(0.0, -1.0, 0.0) }
let centre = (side / 2.0) * side + side / 2.0

expect List.length(towers) == side * side
expect Bytes.length(paintsAt(1.5, -1.0, -1.0)) == 16.0 * side * side
expect List.all((lot) => match lot with | (d, _) => glowAt(d, 0.7) >= 0.0 && glowAt(d, 0.7) <= 1.0, lots)
// Straight down onto the middle picks the tower there, from its roof.
expect (Instance.raycast(down, towers) |> Option.map((hit) => hit.index)) == Option.Some(centre)
expect (Instance.raycast(down, towers) |> Option.map((hit) => Vec3.y(hit.normal))) == Option.Some(1.0)
// Off the edge of the city: nothing.
expect Instance.raycast({ down with origin: Vec3.make(200.0, 50.0, 0.0) }, towers) == Option.None
