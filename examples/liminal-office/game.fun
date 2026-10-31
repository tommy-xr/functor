// liminal-office — one floor of an empty office block at night: a carpeted
// corridor under a drop ceiling and fluorescent panels, open empty offices,
// a neon entrance on an empty apron.
//
//   functor -d examples/liminal-office run native
//   functor -d examples/liminal-office run native --emulate-xr
//
// Desktop: click to capture the mouse, then mouse-look; WASD or the arrows
// walk. 1, 2, 3 jump to the entrance, corridor and office views. VR: the left
// thumbstick walks and strafes; the right turns and walks.
//
// layout.fun is the one layout authority; building.fun derives the mesh,
// cells, colliders and exterior from it. The interior's light is baked
// (`Building.night`) and committed under baked/; bake.mjs rebakes it and records
// its provenance, which provenance.fun checks at load.

let v = Vec3.make

// ---- Lights ------------------------------------------------------------------

// The bake job (`Building.night`) holds the panels' light; `functor bake`
// reads this list.
let bakes = [Building.night]

// The frame's lights reach only unlightmapped geometry (the street, the
// facade, the lamps): all are `Light.baked`, so the lit interior ignores
// them. None casts shadows: a shadow pass would draw every hidden room.
let nightSky = Color.rgb(22.0 / 255.0, 17.0 / 255.0, 34.0 / 255.0)
let frameLights = List.flatten([
  [Light.ambient(Color.rgb(0.09, 0.08, 0.14)) |> Light.baked,
   Light.directional(v(0.3, -0.8, 0.5), Color.rgb(0.35, 0.38, 0.55), 0.6) |> Light.baked,
   // The lobby's light spilling onto the apron.
   Light.point(v(10.0, 2.0, 0.9), Color.rgb(0.85, 0.95, 0.8), 1.2, 9.0) |> Light.baked],
  Exterior.lampHeads |> List.map((p) =>
    Light.point(p |> Vec3.add(v(0.0, -0.2, 0.0)), Color.rgb(1.0, 0.68, 0.36), 0.9, 14.0) |> Light.baked),
])
// Without a bake, light the interior flatly so it stays walkable.
let unbakedLight = Light.ambient(Color.rgb(0.55, 0.57, 0.5))

// ---- The interior --------------------------------------------------------------

let interior: List<Building.PartModel> =
  match Provenance.status with
  | Provenance.Current =>
    let mesh = Provenance.meshPath |> File.bytes |> Lightmap.readMesh in
    Building.partModels({ mesh with lightmap: Option.Some({
      texture: Texture.fromPng(File.bytes(Provenance.lightPath), Texture.Srgb),
      scale: Building.bakeScale, floor: Color.rgb(0.0, 0.0, 0.0) }) })
  | _ => Building.partModels(Building.interiorData)

let baked = Provenance.status == Provenance.Current

// ---- The player ------------------------------------------------------------------

let playerTag = Physics.tag("player")
let centreAboveFeet = 0.92
let eyeAboveCentre = 0.73
let walkSpeed = 2.2
let turnRate = 1.6
let mouseRate = 0.0025
let pitchLimit = 1.3
// Interiors further than this draw nothing: `Scene.cells` draws every cell
// when the eye is outside them all. The fog's far end, so nothing pops.
let interiorRange = 140.0

type Look =
  | NoMouse
  | MouseAt(x: float, y: float)

// Views 1–3: where, and which way.
type View = { x: float, z: float, yaw: float, pitch: float }
let views = [
  { x: Layout.spawn.x, z: Layout.spawn.z, yaw: 0.0, pitch: 0.06 },
  { x: 10.0, z: 6.8, yaw: 0.0, pitch: 0.0 },
  { x: 18.5, z: 14.5, yaw: -1.3, pitch: 0.0 },
]

let spawnAt = v(Layout.spawn.x, centreAboveFeet, Layout.spawn.z)

let init = {
  yaw: 0.0, pitch: 0.06, look: NoMouse, vy: 0.0,
  walk: 0.0, strafe: 0.0, turn: 0.0,
  // A requested view (1–3), applied by the next tick; 0 for none.
  goto: 0.0,
  // From the last tick: where the player stood, its cell (-1: outside every
  // cell), whether the interior draws, and whether its bake is current.
  at: { x: Layout.spawn.x, y: centreAboveFeet, z: Layout.spawn.z },
  cell: 0.0,
  near: true,
  baked: baked,
}

let held = (k, snap: Input.snapshot) => snap.heldKeys |> List.any((h) => h == k)
let axis = (neg, pos, snap) =>
  (if held(pos, snap) then 1.0 else 0.0) - (if held(neg, snap) then 1.0 else 0.0)
let deadzone = (x: float): float => if Math.abs(x) < 0.15 then 0.0 else x
let unit = (x: float): float => Math.clamp(-1.0, 1.0, x)
let still = { x: 0.0, y: 0.0 }

let sampledInput = (model, snap: Input.snapshot) =>
  let pad = (match snap.xr with
    | Option.Some(xr) => { stick: xr.left.thumbstick, look: xr.right.thumbstick }
    | Option.None => { stick: still, look: still }) in
  { model with
    // In a headset the head pitches; a tilted camera would tilt the horizon.
    pitch: if snap.xr == Option.None then model.pitch else 0.0,
    walk: unit(axis(Key.S, Key.W, snap) + axis(Key.Down, Key.Up, snap) + deadzone(pad.stick.y) + deadzone(pad.look.y)),
    strafe: unit(axis(Key.A, Key.D, snap) + deadzone(pad.stick.x)),
    turn: unit(axis(Key.Right, Key.Left, snap) - deadzone(pad.look.x)) }

// Mouse positions are absolute: look by the change since the last one.
let mouseMove = (model, x: float, y: float) =>
  match model.look with
  | NoMouse => { model with look: MouseAt(x, y) }
  | MouseAt(lx, ly) =>
    { model with
      yaw: model.yaw - (x - lx) * mouseRate,
      pitch: Math.clamp(0.0 - pitchLimit, pitchLimit, model.pitch - (y - ly) * mouseRate),
      look: MouseAt(x, y) }

let input = (model, key, isDown: bool) =>
  if not isDown then model
  else match key with
    | Key.Num1 => { model with goto: 1.0 }
    | Key.Num2 => { model with goto: 2.0 }
    | Key.Num3 => { model with goto: 3.0 }
    | _ => model

let eyeOf = (p: Physics.position): Vec3.t => v(p.x, p.y + eyeAboveCentre, p.z)

// How far a point is from the building's footprint (0 inside it).
let fromBuilding = (x: float, z: float): float =>
  let f = Layout.footprint in
  let dx = Math.max(0.0, Math.max(f.x0 - x, x - f.x1)) in
  let dz = Math.max(0.0, Math.max(f.z0 - z, z - f.z1)) in
  Math.sqrt(dx * dx + dz * dz)

let gravity = 9.81

let tick = (model, dt: float, tts: float) =>
  let p = Physics.position(playerTag) in
  let observed = { model with
    at: p,
    cell: Cells.locate(eyeOf(p), Building.level) |> Option.defaultValue(-1.0),
    near: fromBuilding(p.x, p.z) < interiorRange,
    baked: baked } in
  match List.nth(model.goto - 1.0, views) with
  | Option.Some(view) =>
    ({ observed with goto: 0.0, vy: 0.0, yaw: view.yaw, pitch: view.pitch },
     Physics.teleport(playerTag, v(view.x, centreAboveFeet, view.z)))
  | Option.None =>
    let yaw = model.yaw + model.turn * turnRate * dt in
    // Forward is (sin yaw, cos yaw); right is (-cos yaw, sin yaw).
    let vx = walkSpeed * (model.walk * Math.sin(yaw) - model.strafe * Math.cos(yaw)) in
    let vz = walkSpeed * (model.walk * Math.cos(yaw) + model.strafe * Math.sin(yaw)) in
    let vy = (if Physics.grounded(playerTag) && model.vy <= 0.0 then 0.0 else model.vy) - gravity * dt in
    ({ observed with goto: 0.0, yaw: yaw, vy: vy }, Physics.move(playerTag, v(vx * dt, vy * dt, vz * dt)))

let physics = (model) =>
  Physics.scene(v(0.0, -9.81, 0.0), [
    Physics.memo("building", 0.0, Building.buildingBodies),
    Physics.memo("street", 0.0, Exterior.streetBodies),
    Physics.character(playerTag, Physics.capsule(0.6, 0.3),
                      { stepHeight: 0.2, maxSlope: 40deg, snap: 0.2, offset: 0.02 })
      |> Physics.at(spawnAt),
  ])

// ---- Drawing ---------------------------------------------------------------------

let rooms = interior |> List.map((p) => Scene.modelOf(p.model) |> Scene.inCells(p.cells))

let draw = (model, tts: float) =>
  let p = Physics.position(playerTag) in
  let camera = Camera3D.firstPerson(eyeOf(p), Angle.radians(model.yaw), Angle.radians(model.pitch), 70deg)
    |> Camera3D.clip(0.05, 160.0) in
  let scene = Scene.cells(Building.level, Scene.group([
    Exterior.exterior,
    if model.near then Scene.group(rooms) else Scene.group([]),
  ])) in
  Frame.createLit(camera, scene, if baked then frameLights else frameLights |> List.append([unbakedLight]))
    |> Frame.withFog(Fog.linear(35.0, 140.0, nightSky))
    |> Frame.withClearColor(nightSky)

// Only shown when the committed bake doesn't match the layout.
let ui = (model) =>
  match Provenance.status with
  | Provenance.Current => Ui.column([])
  | Provenance.Missing => Ui.text("no bake: lighting is flat. Run node examples/liminal-office/bake.mjs") |> Ui.panel(Ui.topLeft())
  | Provenance.Stale(why) => Ui.text($"stale bake ({why}): lighting is flat. Run node examples/liminal-office/bake.mjs") |> Ui.panel(Ui.topLeft())

// Quest: 120 Hz with foveation off, the liminal-city perf baseline.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120))
let settings = (model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))
