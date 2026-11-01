// liminal-office — two floors of an empty office block, by night or day: a
// carpeted corridor under a drop ceiling and fluorescent panels, open empty
// offices (103 and 203 have windows), a sodium-lit stairwell, a neon
// entrance on an empty apron. B turns it into a block: four turned copies of
// the office around a crossroads (block.fun). G cycles grids of 4, 16 and 64
// copies, where at most K interiors draw and the rest are shells (shells.fun).
//
//   functor -d examples/liminal-office run native
//   functor -d examples/liminal-office run native --emulate-xr
//
// Desktop: click to capture the mouse, then mouse-look; WASD or the arrows
// walk. 1–8 jump to views: entrance, corridor, office, window; stair foot,
// landing, upstairs corridor, upstairs window. 9 and 0 jump into the block:
// the street, and a turned copy's window. T steps the hour; N jumps between
// midnight and noon; B toggles the block; G cycles the grids; K cycles how many
// interiors draw (4, 1, 2). VR: the left thumbstick walks and
// strafes; the right turns and walks; X steps the hour, Y jumps midnight ⇄
// noon; B toggles the block.
//
// layout.fun is the one layout authority; building.fun derives the mesh,
// colliders and bake jobs from it, portals.fun the culling cells, and
// exterior.fun the street side. The interior's light is baked
// (`Building.night`, `Building.day`) and committed under baked/; bake.mjs
// rebakes it and records its provenance, which provenance.fun checks at load.
// hours.fun maps the hour to the bake and the outdoor light.

let v = Vec3.make

// ---- Lights ------------------------------------------------------------------

// The bake jobs (`Building.night`, `Building.day`) hold the panels' light and
// the daylight through openings; `functor bake` reads this list.
let bakes = Building.bakes

// The frame's lights reach only unlightmapped geometry (the street, the
// facade, the lamps): all are `Light.baked`, so the lit interior ignores
// them, by day as by night. None casts shadows: a shadow pass would draw
// every hidden room. Each copy has a lobby spill and two street lamps; the
// frame takes those nearest the eye that fit the 8 forward-light slots.
let pointLights = (o: Hours.Outdoor, c: Block.Copy): List<(Vec3.t, Light.t)> =>
  let point = (at: Vec3.t, color: Color.t, intensity: float, range: float) =>
    let p = Block.toWorld(c, at) in (p, Light.point(p, color, intensity, range) |> Light.baked) in
  List.flatten([
    // The lobby's light spilling onto the apron.
    [point(v(10.0, 2.0, 0.9), Color.rgb(0.85, 0.95, 0.8), 1.2, 9.0)],
    Exterior.lampHeads |> List.map((p) => point(p |> Vec3.add(v(0.0, -0.2, 0.0)), Color.rgb(1.0, 0.68, 0.36), 0.9 * o.lamps, 14.0)),
  ])
let frameLights = (o: Hours.Outdoor, copies: List<Block.Copy>, eye: Vec3.t): List<Light.t> => List.flatten([
  [Light.ambient(Hours.color(o.ambient)) |> Light.baked,
   Light.directional(o.sunDirection, Hours.color(o.sun), o.sunIntensity) |> Light.baked],
  Block.nearestLights(eye, copies |> List.concatMap((c) => pointLights(o, c))),
])
// Without a bake, light the interior flatly so it stays walkable.
let unbakedLight = Light.ambient(Color.rgb(0.55, 0.57, 0.5))

// ---- The interior --------------------------------------------------------------

let interior: List<Building.CellModel> =
  match Provenance.status with
  | Provenance.Current =>
    let mesh = Provenance.meshPath |> File.bytes |> Lightmap.readMesh in
    Building.cellModels({ mesh with lightmap: Option.Some({
      texture: Texture.fromPng(File.bytes(Provenance.lightPath), Texture.Srgb),
      scale: Building.bakeScale, floor: Color.rgb(0.0, 0.0, 0.0) }) })
  | _ => Building.cellModels(Building.interiorData)

let baked = Provenance.status == Provenance.Current

// Each cell's mesh in its cell, under the night bake, and the same models under the
// day bake: one mesh, two resident lightmaps, picked per frame. Built once:
// every copy in the block draws these same values.
let roomOf = (p: Building.CellModel): Scene.t => Scene.modelOf(p.model) |> Scene.inCells([p.cell])
let nightRooms = interior |> List.map(roomOf)
let dayRooms =
  if not baked then nightRooms
  else
    let dayLightmap = Texture.fromPng(File.bytes(Provenance.dayLightPath), Texture.Srgb) in
    nightRooms |> List.map((room) => room |> Scene.withLightmap(dayLightmap))

// ---- The player ------------------------------------------------------------------

let playerTag = Physics.tag("player")
let centreAboveFeet = 0.92
let eyeAboveCentre = 0.73
let walkSpeed = 2.2
let turnRate = 1.6
let mouseRate = 0.0025
let pitchLimit = 1.3

type Look =
  | NoMouse
  | MouseAt(x: float, y: float)

// Views 1–8 (in the office, or the block's nearest copy) and 9, 0 (in the
// block, or a grid): in which copy (9 and 0 only), where in it, on which level, and which way. 4 faces office 103's window;
// 5 looks up the stairs from their foot, 6 down them from the landing; 7 is
// the upstairs corridor; 8 faces office 203's window. 9 looks up the street
// through the crossroads; 0 faces office 103's window in the copy turned
// 180°.
type View = { copy: float, x: float, level: float, z: float, yaw: float, pitch: float }
let views = [
  { copy: 0.0, x: Layout.spawn.x, level: 0.0, z: Layout.spawn.z, yaw: 0.0, pitch: 0.06 },
  { copy: 0.0, x: 10.0, level: 0.0, z: 6.8, yaw: 0.0, pitch: 0.0 },
  { copy: 0.0, x: 18.5, level: 0.0, z: 14.5, yaw: -1.3, pitch: 0.0 },
  { copy: 0.0, x: 7.6, level: 0.0, z: 7.0, yaw: -1.1, pitch: -0.2 },
  { copy: 0.0, x: 11.9, level: 0.0, z: 21.2, yaw: 1.75, pitch: 0.4 },
  { copy: 0.0, x: 18.9, level: 1.0, z: 20.8, yaw: -1.4, pitch: -0.45 },
  { copy: 0.0, x: 10.0, level: 1.0, z: 0.8, yaw: 0.0, pitch: 0.0 },
  { copy: 0.0, x: 7.6, level: 1.0, z: 7.0, yaw: -1.1, pitch: -0.2 },
  { copy: 0.0, x: -12.0, level: 0.0, z: -50.0, yaw: 0.0, pitch: 0.04 },
  { copy: 2.0, x: 7.6, level: 0.0, z: 7.0, yaw: -1.1, pitch: -0.2 },
]
// Views 9 and 0 are in the block (or the grid you're in).
let inBlock = (n: float): bool => n > 8.0

let spawnAt = v(Layout.spawn.x, centreAboveFeet, Layout.spawn.z)

let init = {
  yaw: 0.0, pitch: 0.06, look: NoMouse, vy: 0.0,
  // The hour, 0 ≤ hour < 24: midnight.
  hour: 0.0,
  // The left controller's X and Y and the right's B at the last sample: a
  // press is a change.
  padX: false, padY: false, padB: false,
  walk: 0.0, strafe: 0.0, turn: 0.0,
  // A requested view (1–10), applied by the next tick; 0 for none.
  goto: 0.0,
  // Which of `Block.towns`: the office alone, the block, a grid.
  town: 0.0,
  // At most this many interiors draw.
  k: Block.maxActive,
  // From the last tick: where the player stood and whether on the ground,
  // the nearest copy and the eye's cell in it (-1: outside every cell),
  // which copies' interiors draw and which have colliders (ids), and
  // whether the bake is current.
  at: { x: Layout.spawn.x, y: centreAboveFeet, z: Layout.spawn.z },
  grounded: false,
  copy: 0.0,
  cell: 0.0,
  active: [],
  solid: [0.0],
  // Every tick adds the active copies whose cells don't hold the eye: each
  // would have drawn its whole interior. Stays 0.
  uncovered: 0.0,
  baked: baked,
}

let held = (k, snap: Input.snapshot) => snap.heldKeys |> List.any((h) => h == k)
let axis = (neg, pos, snap) =>
  (if held(pos, snap) then 1.0 else 0.0) - (if held(neg, snap) then 1.0 else 0.0)
let deadzone = (x: float): float => if Math.abs(x) < 0.15 then 0.0 else x
let unit = (x: float): float => Math.clamp(-1.0, 1.0, x)
let still = { x: 0.0, y: 0.0 }

// T and X: the next hour. N and Y: midnight ⇄ noon.
let nextHour = (hour: float): float => Math.mod(hour + 1.0, 24.0)
let dayOrNight = (hour: float): float => if Hours.isDay(hour) then 0.0 else 12.0
// B: the office alone ⇄ the block (from a grid, the office alone). G: the
// next grid. K: 4 → 1 → 2 → 4 interiors at most.
let toggleBlock = (town: float): float => if town == 0.0 then 1.0 else 0.0
let nextGrid = (town: float): float => if town < 2.0 || town >= 4.0 then 2.0 else town + 1.0
let nextK = (k: float): float => if k >= Block.maxActive then 1.0 else k * 2.0

let sampledInput = (model, snap: Input.snapshot) =>
  let pad = (match snap.xr with
    | Option.Some(xr) => { stick: xr.left.thumbstick, look: xr.right.thumbstick, x: xr.left.primaryPressed, y: xr.left.secondaryPressed, b: xr.right.secondaryPressed }
    | Option.None => { stick: still, look: still, x: false, y: false, b: false }) in
  let stepped = if pad.x && not model.padX then nextHour(model.hour) else model.hour in
  { model with
    hour: if pad.y && not model.padY then dayOrNight(stepped) else stepped,
    town: if pad.b && not model.padB then toggleBlock(model.town) else model.town,
    padX: pad.x, padY: pad.y, padB: pad.b,
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
    | Key.Num4 => { model with goto: 4.0 }
    | Key.Num5 => { model with goto: 5.0 }
    | Key.Num6 => { model with goto: 6.0 }
    | Key.Num7 => { model with goto: 7.0 }
    | Key.Num8 => { model with goto: 8.0 }
    | Key.Num9 => { model with goto: 9.0 }
    | Key.Num0 => { model with goto: 10.0 }
    | Key.T => { model with hour: nextHour(model.hour) }
    | Key.N => { model with hour: dayOrNight(model.hour) }
    | Key.B => { model with town: toggleBlock(model.town) }
    | Key.G => { model with town: nextGrid(model.town) }
    | Key.K => { model with k: nextK(model.k) }
    | _ => model

let eyeOf = (p: Physics.position): Vec3.t => v(p.x, p.y + eyeAboveCentre, p.z)

let gravity = 9.81

let tick = (model, dt: float, tts: float) =>
  let p = Physics.position(playerTag) in
  let eye = eyeOf(p) in
  // The copies the last frame drew: that draw ran after the physics step
  // that left the player at `p`, from the model this tick started with.
  let town = Block.townAt(model.town) in
  let active = Block.activeAt(town, model.k, model.active, eye) in
  let copy = Block.nearest(town, eye) in
  let observed = { model with
    at: p,
    grounded: Physics.grounded(playerTag),
    copy: copy,
    cell: Cells.locate(Block.toLocal(Block.copyAt(copy), eye), Portals.level) |> Option.defaultValue(-1.0),
    active: active,
    solid: Block.solidAt(town, model.solid, eye),
    uncovered: model.uncovered + Block.uncovered(town, active, eye),
    baked: baked } in
  match List.nth(model.goto - 1.0, views) with
  | Option.Some(view) =>
    let c = Block.copyAt(if inBlock(model.goto) then view.copy else copy) in
    let to = Block.toWorld(c, v(view.x, view.level * Layout.storey + centreAboveFeet, view.z)) in
    let t = if inBlock(model.goto) && model.town == 0.0 then 1.0 else model.town in
    // Solid where the player lands, before the teleport lands.
    ({ observed with goto: 0.0, vy: 0.0, yaw: Block.headingToWorld(c, view.yaw), pitch: view.pitch,
                     town: t, solid: Block.solidAt(Block.townAt(t), observed.solid, to) },
     Physics.teleport(playerTag, to))
  | Option.None =>
    let yaw = model.yaw + model.turn * turnRate * dt in
    // Forward is (sin yaw, cos yaw); right is (-cos yaw, sin yaw).
    let vx = walkSpeed * (model.walk * Math.sin(yaw) - model.strafe * Math.cos(yaw)) in
    let vz = walkSpeed * (model.walk * Math.cos(yaw) + model.strafe * Math.sin(yaw)) in
    let vy = (if Physics.grounded(playerTag) && model.vy <= 0.0 then 0.0 else model.vy) - gravity * dt in
    ({ observed with goto: 0.0, yaw: yaw, vy: vy }, Physics.move(playerTag, v(vx * dt, vy * dt, vz * dt)))

// Each solid copy's colliders are built once, under its own key and tags.
let physics = (model) =>
  Physics.scene(v(0.0, -9.81, 0.0), List.flatten([
    [Block.townAt(model.town).groundBody],
    model.solid |> List.map((id) => let c = Block.copyAt(id) in Physics.memo(c.name, c, Block.bodiesOf)),
    [Physics.character(playerTag, Physics.capsule(0.6, 0.3),
                       { stepHeight: 0.2, maxSlope: 40deg, snap: 0.2, offset: 0.02 })
       |> Physics.at(spawnAt)],
  ]))

// ---- Drawing ---------------------------------------------------------------------

// What a copy draws: its street side at the hour's neon and lamps, and its
// interior when active, under the day or night bake.
type CopyLook = { copy: Block.Copy, neon: float, lamps: float, day: bool, active: bool }
let copyScene = (look: CopyLook): Scene.t =>
  let interiorScene =
    if not look.active then Scene.group([])
    else Scene.cells(Portals.level, Scene.group(if look.day then dayRooms else nightRooms)) in
  Block.sceneOf(look.copy, Scene.group([Exterior.exteriorAt(look.neon, look.lamps), interiorScene]))

let draw = (model, tts: float) =>
  let p = Physics.position(playerTag) in
  let eye = eyeOf(p) in
  let o = Hours.outdoorAt(Hours.daylight(model.hour)) in
  let camera = Camera3D.firstPerson(eye, Angle.radians(model.yaw), Angle.radians(model.pitch), 70deg)
    |> Camera3D.clip(0.05, 160.0) in
  let town = Block.townAt(model.town) in
  let active = Block.activeAt(town, model.k, model.active, eye) in
  let day = Hours.isDay(model.hour) in
  // Each copy rebuilds only when the hour or its activation changes. In a
  // grid only the active copies draw as themselves; the rest are shells.
  let drawn = if town.shells then active else List.range(List.length(town.copies)) in
  let copies = drawn |> List.map((id) =>
    let c = Block.copyAt(id) in
    Scene.memo(c.name, { copy: c, neon: o.neon, lamps: o.lamps, day: day, active: Block.isIn(id, active) }, copyScene)) in
  let shells =
    if not town.shells then []
    else [Scene.memo($"{town.name}/shells", { town: model.town, active: active, neon: o.neon, lamps: o.lamps, day: day }, Shells.shellsOf)] in
  let scene = Scene.group([town.ground, Scene.group(List.append(shells, copies))]) in
  let lights = frameLights(o, Block.near(town, Block.lightReach, eye) |> List.map((e) => let (id, d) = e in Block.copyAt(id)), eye) in
  Frame.createLit(camera, scene, if baked then lights else lights |> List.append([unbakedLight]))
    |> Frame.withFog(Fog.linear(o.fogNear, Hours.fogFar, Hours.color(o.sky)))
    |> Frame.withClearColor(Hours.color(o.sky))

// Only shown when the committed bake doesn't match the layout.
let ui = (model) =>
  match Provenance.status with
  | Provenance.Current => Ui.column([])
  | Provenance.Missing => Ui.text("no bake: lighting is flat. Run node examples/liminal-office/bake.mjs") |> Ui.panel(Ui.topLeft())
  | Provenance.Stale(why) => Ui.text($"stale bake ({why}): lighting is flat. Run node examples/liminal-office/bake.mjs") |> Ui.panel(Ui.topLeft())

// Quest: 120 Hz with foveation off, the liminal-city perf baseline.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120))
let settings = (model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))
