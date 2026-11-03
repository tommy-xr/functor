// liminal-office — two floors of an empty office block, by night or day: a
// carpeted corridor under a drop ceiling and fluorescent panels, open empty
// offices (103 and 203 have windows), a sodium-lit stairwell, a neon
// entrance on an empty apron. B turns it into a block: four turned copies of
// the office around a crossroads (block.fun). G cycles grids of 4, 16 and 64
// copies, where at most K interiors draw and the rest are shells (shells.fun).
// H stands four towers there instead (tower.fun), climbed by dog-leg stairs.
// C stands a multiplex cinema on its parking lot (cinema.fun).
//
//   functor -d examples/liminal-office run native
//   functor -d examples/liminal-office run native --emulate-xr
//
// Desktop: click to capture the mouse, then mouse-look; WASD or the arrows
// walk. 1–8 jump to views: entrance, corridor, office, window; stair foot,
// landing, upstairs corridor, upstairs window. 9 and 0 jump into the block:
// the street, and a turned copy's window. T steps the hour; N jumps between
// midnight and noon; B toggles the block; G cycles the grids; K cycles how many
// interiors draw (4, 1, 2); O overlooks the roofs from 20 m up; H toggles the
// towers, where 1–0 jump to their own views (`towerViews`); C the cinema
// (`cinemaViews`); I puts the block
// on the island (island.fun), V cycles its biome; L toggles a flashlight. VR:
// the left thumbstick walks and strafes; the right turns and walks; X steps
// the hour, Y jumps midnight ⇄ noon; B toggles the block, A the island; the
// right trigger toggles the flashlight, held in the right hand.
//
// officeLayout.fun is the office's one layout authority (a layout.fun
// `Layout`); building.fun derives the mesh, colliders and bake jobs from it,
// portals.fun the culling cells, exterior.fun the street side, and
// archetypes.fun holds them, once per archetype. The interior's light is
// baked and committed under baked/office/; bake.mjs rebakes it and records
// its provenance, which provenance.fun checks at load.
// hours.fun maps the hour to the bake and the outdoor light.

let v = Vec3.make

// ---- Lights ------------------------------------------------------------------

// Every archetype's night and day bake jobs hold the panels' light and the
// daylight through openings; `functor bake` reads this list.
let bakes = Archetypes.bakes

// The frame's lights reach only unlightmapped geometry (the street, the
// facade, the lamps): all are `Light.baked`, so the lit interior ignores
// them, by day as by night. None casts shadows: a shadow pass would draw
// every hidden room. Each copy has a lobby spill and two street lamps; the
// frame takes those nearest the eye that fit the 8 forward-light slots.
// A copy's street lights in the world, as plain data: the lobby's spill and
// a sodium lamp under each head (`switched`: off by day). island.mjs paints
// the same lights into the island's night map; tests.fun checks they agree.
type StreetLight = { at: Vec3.t, r: float, g: float, b: float, intensity: float, range: float, switched: bool }
let streetLights = (c: Block.Copy): List<StreetLight> =>
  let lobby = Exterior.lobbyLight in
  let lamp = Exterior.sodiumLight in
  List.flatten([
    [{ at: Block.toWorld(c, Block.archetypeOf(c).street.lobbyLight), r: lobby.r, g: lobby.g, b: lobby.b,
       intensity: lobby.intensity, range: lobby.range, switched: false }],
    Block.archetypeOf(c).street.lampHeads |> List.map((p) =>
      { at: Block.toWorld(c, p |> Vec3.add(v(0.0, 0.0 - lamp.drop, 0.0))), r: lamp.r, g: lamp.g, b: lamp.b,
        intensity: lamp.intensity, range: lamp.range, switched: true }),
  ])
let pointLights = (o: Hours.Outdoor, c: Block.Copy): List<(Vec3.t, Light.t)> =>
  streetLights(c) |> List.map((s) =>
    (s.at, Light.point(s.at, Color.rgb(s.r, s.g, s.b), if s.switched then s.intensity * o.lamps else s.intensity, s.range)
             |> Light.baked))
// The flashlight (none, or one spot) goes third: the island's colormap
// terrain is lit only by the first unbaked spot within the first 8 lights,
// so it takes a point light's slot rather than queue behind them.
let frameLights = (o: Hours.Outdoor, copies: List<Block.Copy>, eye: Vec3.t, flashlight: List<Light.t>): List<Light.t> => List.flatten([
  [Light.ambient(Hours.color(o.ambient)) |> Light.baked,
   Light.directional(o.sunDirection, Hours.color(o.sun), o.sunIntensity) |> Light.baked],
  flashlight,
  Block.nearestLights(Block.pointSlots - List.length(flashlight), eye, copies |> List.concatMap((c) => pointLights(o, c))),
])

// The flashlight: L on the desktop, held just below the eye and aimed where
// you look; in VR the right trigger, aimed with the right controller.
let flashlightCone = Angle.degrees(20.0)
let flashlightRange = 60.0
let flashlightIntensity = 1.6
let flashlightColor = Color.rgb(1.0, 0.92, 0.78)
// Where the camera points: yaw 0 / pitch 0 looks down +Z, positive pitch up.
let lookDir = (yaw: float, pitch: float): Vec3.t =>
  v(Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), Math.cos(yaw) * Math.cos(pitch))
// Without a bake, light the interior flatly so it stays walkable.
let unbakedLight = Light.ambient(Color.rgb(0.55, 0.57, 0.5))

// ---- The interior --------------------------------------------------------------

let baked = Archetypes.baked

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

// The office's spawn, on its apron.
let spawn = OfficeLayout.layout.spawn

// Views 1–8 (in the office, or the block's nearest copy) and 9, 0 (in the
// block, or a grid): in which copy (9 and 0 only), where in it, on which level, and which way. 4 faces office 103's window;
// 5 looks up the stairs from their foot, 6 down them from the landing; 7 is
// the upstairs corridor; 8 faces office 203's window. 9 looks up the street
// through the crossroads; 0 faces office 103's window in the copy turned
// 180°.
type View = { copy: float, x: float, level: float, z: float, yaw: float, pitch: float }
let views = [
  { copy: 0.0, x: spawn.x, level: 0.0, z: spawn.z, yaw: 0.0, pitch: 0.06 },
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
// The towers' views, in the nearest tower: 1 the lobby, 2 a lift lobby
// (floor 3), 3 down its corridor, 4 its west office's window; 5 up the
// well from floor 1's landing, 6 down the well from floor 6's
// landing, 7 up a flight (floor 2), 8 floor 8's west window. 9 the 8-floor
// tower's roof terrace, facing the taller ones; 0 the street between them.
let towerViews = [
  { copy: 0.0, x: 12.0, level: 0.0, z: 1.0, yaw: 0.0, pitch: 0.05 },
  { copy: 0.0, x: 13.6, level: 3.0, z: 7.0, yaw: 0.0, pitch: 0.05 },
  { copy: 0.0, x: 18.2, level: 3.0, z: 7.6, yaw: -1.5708, pitch: 0.0 },
  { copy: 0.0, x: 3.2, level: 3.0, z: 12.0, yaw: -1.5708, pitch: -0.1 },
  { copy: 0.0, x: 9.2, level: 1.0, z: 10.6, yaw: 0.0, pitch: 1.0 },
  { copy: 0.0, x: 9.2, level: 6.0, z: 10.2, yaw: 0.0, pitch: -1.2 },
  { copy: 0.0, x: 8.3, level: 2.0, z: 9.8, yaw: 0.0, pitch: 0.3 },
  { copy: 0.0, x: 3.2, level: 8.0, z: 12.0, yaw: -1.5708, pitch: -0.1 },
  { copy: 0.0, x: 6.0, level: 9.0, z: 3.0, yaw: -2.7, pitch: 0.25 },
  { copy: 0.0, x: -12.0, level: 0.0, z: -36.0, yaw: 0.0, pitch: 0.35 },
]
// The cinema's views: 1 the lobby; 2 down the numbered corridor (B) by
// night; 3 auditorium 3 from the screen, up the rake; 4 down it from the top
// row; 5 up corridor A to its bend; 6 the service corridor; 7 the lobby's
// posters; 8 auditorium 4's playset from its aisle; 9 the parking lot,
// facing the entrance; 0 the lot's far corner, where the cinema is a shell.
let cinemaViews = [
  { copy: 0.0, x: 30.0, level: 0.0, z: 2.2, yaw: 0.0, pitch: 0.08 },
  { copy: 0.0, x: 47.9, level: 0.0, z: 19.75, yaw: -1.5708, pitch: 0.03 },
  { copy: 0.0, x: 41.9, level: 0.0, z: 22.3, yaw: -0.45, pitch: 0.2 },
  { copy: 0.0, x: 29.75, level: CinemaBuilding.rakeTop / CinemaLayout.roofline, z: 38.6, yaw: 3.1416, pitch: -0.18 },
  { copy: 0.0, x: 46.75, level: 0.0, z: 8.8, yaw: 0.0, pitch: 0.05 },
  { copy: 0.0, x: 13.8, level: 0.0, z: 19.0, yaw: 0.0, pitch: 0.0 },
  { copy: 0.0, x: 33.0, level: 0.0, z: 12.5, yaw: -2.2, pitch: 0.05 },
  { copy: 0.0, x: 28.25, level: CinemaBuilding.treadY(5.0) / CinemaLayout.roofline, z: 31.0, yaw: -2.6, pitch: -0.2 },
  { copy: 0.0, x: 40.0, level: 0.0, z: -26.0, yaw: 0.1, pitch: 0.22 },
  { copy: 0.0, x: 65.0, level: 0.0, z: -49.0, yaw: -0.7, pitch: 0.06 },
]
let viewsOf = (town: float): List<View> =>
  if town == Block.towersTown then towerViews else if town == Block.cinemaTown then cinemaViews else views
// Views 9 and 0 are in the block (or the grid you're in).
let inBlock = (n: float): bool => n > 8.0

let spawnAt = v(spawn.x, centreAboveFeet, spawn.z)

let init = {
  yaw: 0.0, pitch: 0.06, look: NoMouse, vy: 0.0,
  // The hour, 0 ≤ hour < 24: midnight.
  hour: 0.0,
  // The left controller's X and Y and the right's A and B at the last
  // sample: a press is a change.
  padX: false, padY: false, padA: false, padB: false,
  walk: 0.0, strafe: 0.0, turn: 0.0,
  // A requested view (1–10), applied by the next tick; 0 for none.
  goto: 0.0,
  // Which of `Block.towns`: the office alone, the block, a grid.
  town: 0.0,
  // At most this many interiors draw.
  k: Block.maxActive,
  // On the flat ground or the island (the office alone or the block only).
  island: false,
  // The island's props drawn and solid near the eye (`Island.regroup`).
  props: Island.noProps,
  // The eye overlooking the roofs (O).
  overlook: false,
  // The island's biome (an index into `Island.biomes`).
  biome: 0.0,
  // The flashlight; whether L or the right trigger was down at the last
  // event (a press is a change), and the right controller's aim in VR.
  flashlight: false,
  flashKey: false,
  padTrigger: false,
  hand: Option.None,
  // From the last tick: where the player stood and whether on the ground,
  // the nearest copy and the eye's cell in it (-1: outside every cell),
  // which copies' interiors draw and which have colliders (ids), and
  // whether the bake is current.
  at: { x: spawn.x, y: centreAboveFeet, z: spawn.z },
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

// I and A: onto the island, with the block; or back to the flat ground.
// Only the pad is level on both grounds: from anywhere else, or from the
// towers (other buildings than the block's), the walker goes to the street
// (view 9) rather than end up under the other ground or inside a wall.
// On the island B and VR B keep the block: its night map has every copy's
// lamp pools painted in.
let toggleIsland = (model) =>
  { model with island: not model.island, town: if model.island then model.town else 1.0,
               goto: if Island.onPad(model.at.x, model.at.z) && model.town != Block.towersTown && model.town != Block.cinemaTown then model.goto else 9.0 }

// T and X: the next hour. N and Y: midnight ⇄ noon.
let nextHour = (hour: float): float => Math.mod(hour + 1.0, 24.0)
let dayOrNight = (hour: float): float => if Hours.isDay(hour) then 0.0 else 12.0
// B: the office alone ⇄ the block (from a grid, the office alone). G: the
// next grid. K: 4 → 1 → 2 → 4 interiors at most.
let toggleBlock = (town: float): float => if town == 0.0 then 1.0 else 0.0
let nextGrid = (town: float): float => if town < 2.0 || town >= 4.0 then 2.0 else town + 1.0
let nextK = (k: float): float => if k >= Block.maxActive then 1.0 else k * 2.0
// A grid doesn't fit on the island's pad: G goes back to the flat ground.
let toGrid = (model) => { (if model.island then toggleIsland(model) else model) with town: nextGrid(model.town) }

let sampledInput = (model, snap: Input.snapshot) =>
  let pad = (match snap.xr with
    | Option.Some(xr) => { stick: xr.left.thumbstick, look: xr.right.thumbstick, x: xr.left.primaryPressed, y: xr.left.secondaryPressed, a: xr.right.primaryPressed, b: xr.right.secondaryPressed,
                           trigger: xr.right.trigger > 0.5, hand: xr.right.aim }
    | Option.None => { stick: still, look: still, x: false, y: false, a: false, b: false, trigger: false, hand: Option.None }) in
  let stepped = if pad.x && not model.padX then nextHour(model.hour) else model.hour in
  let toggled = { model with town: if model.island || not (pad.b && not model.padB) then model.town else toggleBlock(model.town) } in
  { (if pad.a && not model.padA then toggleIsland(toggled) else toggled) with
    hour: if pad.y && not model.padY then dayOrNight(stepped) else stepped,
    padX: pad.x, padY: pad.y, padA: pad.a, padB: pad.b, padTrigger: pad.trigger, hand: pad.hand,
    flashlight: model.flashlight != (pad.trigger && not model.padTrigger),
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
  // L toggles once per press: a held key repeats.
  if key == Key.L then { model with flashlight: model.flashlight != (isDown && not model.flashKey), flashKey: isDown }
  else if not isDown then model
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
    | Key.B => { model with town: if model.island then model.town else toggleBlock(model.town) }
    | Key.G => toGrid(model)
    // Into the other town's view 1: its building where the player stands
    // differs. The towers stand on the flat ground.
    | Key.H => { model with island: false, town: if model.town == Block.towersTown then 0.0 else Block.towersTown, goto: 1.0 }
    // Onto the cinema's parking lot (view 9), or back to the office's entrance.
    | Key.C =>
      let leaving = model.town == Block.cinemaTown in
      { model with island: false, town: if leaving then 0.0 else Block.cinemaTown, goto: if leaving then 1.0 else 9.0 }
    | Key.K => { model with k: nextK(model.k) }
    | Key.I => toggleIsland(model)
    | Key.V => { model with biome: Math.mod(model.biome + 1.0, List.length(Island.biomes)) }
    | Key.O => { model with overlook: not model.overlook, pitch: if model.overlook then 0.0 else overlookPitch }
    | _ => model

// O: overlook. The eye rises `overlookLift` above the player and looks down
// over the roofs; no interior draws from up there.
let overlookLift = 20.0
let overlookPitch = -0.5
let eyeOf = (model, p: Physics.position): Vec3.t =>
  v(p.x, p.y + eyeAboveCentre + (if model.overlook then overlookLift else 0.0), p.z)
let interiorsOf = (model): float => if model.overlook then 0.0 else model.k

let gravity = 9.81

let tick = (model, dt: float, tts: float) =>
  let p = Physics.position(playerTag) in
  let eye = eyeOf(model, p) in
  // The copies the last frame drew: that draw ran after the physics step
  // that left the player at `p`, from the model this tick started with.
  let town = Block.townAt(model.town) in
  let active = Block.activeAt(town, interiorsOf(model), model.active, eye) in
  let copy = Block.nearest(town, eye) in
  let observed = { model with
    at: p,
    grounded: Physics.grounded(playerTag),
    copy: copy,
    cell: (let c = Block.copyIn(town, copy) in Cells.locate(Block.toLocal(c, eye), Block.archetypeOf(c).level)) |> Option.defaultValue(-1.0),
    active: active,
    solid: Block.solidAt(town, model.solid, eye),
    uncovered: model.uncovered + Block.uncovered(town, active, eye),
    baked: baked,
    props: Island.regroup(model.props, model.island, p.x, p.z, Hours.reachAt(Island.reach, Hours.daylight(model.hour))) } in
  match List.nth(model.goto - 1.0, viewsOf(model.town)) with
  | Option.Some(view) =>
    let t = if inBlock(model.goto) && model.town == 0.0 then 1.0 else model.town in
    let c = Block.copyIn(Block.townAt(t), if inBlock(model.goto) then view.copy else copy) in
    let to = Block.toWorld(c, v(view.x, view.level * Block.archetypeOf(c).layout.storey + centreAboveFeet, view.z)) in
    // Solid where the player lands, before the teleport lands.
    ({ observed with goto: 0.0, vy: 0.0, overlook: false, yaw: Block.headingToWorld(c, view.yaw), pitch: view.pitch,
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
    if model.island then List.append([Physics.memo("island-props", model.props.solid, Island.propBodies)], Island.bodies)
    else [Block.townAt(model.town).groundBody],
    model.solid |> List.map((id) => let c = Block.copyIn(Block.townAt(model.town), id) in Physics.memo(c.name, c, Block.bodiesOf)),
    [Physics.character(playerTag, Physics.capsule(0.6, 0.3),
                       { stepHeight: 0.2, maxSlope: 40deg, snap: 0.2, offset: 0.02 })
       |> Physics.at(spawnAt)],
  ]))

// ---- Drawing ---------------------------------------------------------------------

// What a copy draws: its archetype's street side at the hour's neon and
// lamps, and its interior when active, under the day or night bake.
type CopyLook = { copy: Block.Copy, neon: float, lamps: float, day: bool, active: bool }
let copyScene = (look: CopyLook): Scene.t =>
  let a = Block.archetypeOf(look.copy) in
  let interiorScene =
    if not look.active then Scene.group([])
    else Scene.cells(a.level, Scene.group(if look.day then a.day else a.night)) in
  Block.sceneOf(look.copy, Scene.group([Exterior.exteriorAt(a.street, look.neon, look.lamps, look.day), interiorScene]))

// The near plane. Depth precision at a distance goes as 1/near: at 0.05 m,
// faces 2 cm apart z-fought from about 60 m. At 0.1 m its corners sit about
// 0.17 m from the eye, inside the capsule's 0.3 m radius.
let nearClip = 0.1

let draw = (model, tts: float) =>
  let p = Physics.position(playerTag) in
  let eye = eyeOf(model, p) in
  let daylight = Hours.daylight(model.hour) in
  let o = if model.island then Hours.islandOutdoorAt(daylight) else Hours.outdoorAt(daylight) in
  let town = Block.townAt(model.town) in
  // On the island, the island's reach.
  let far = Hours.reachAt(if model.island then Island.reach else town.reach, daylight) in
  let camera = Camera3D.firstPerson(eye, Angle.radians(model.yaw), Angle.radians(model.pitch), 70deg)
    |> Camera3D.clip(nearClip, far) in
  let active = Block.activeAt(town, interiorsOf(model), model.active, eye) in
  let day = Hours.isDay(model.hour) in
  // Each copy rebuilds only when the hour or its activation changes. In a
  // grid only the active copies draw as themselves; the rest are shells.
  let drawn = if town.shells then active else List.range(List.length(town.copies)) in
  let copies = drawn |> List.map((id) =>
    let c = Block.copyIn(town, id) in
    Scene.memo(c.name, { copy: c, neon: o.neon, lamps: o.lamps, day: day, active: Block.isIn(id, active) }, copyScene)) in
  let shells =
    if not town.shells then []
    else [Scene.memo($"{town.name}/shells", { town: model.town, active: active, neon: o.neon, lamps: o.lamps, day: day }, Shells.shellsOf)] in
  let buildings = Scene.group(List.append(shells, copies)) in
  // On the island the props draw after the buildings and the terrain last,
  // so early-Z rejects what's behind them.
  let scene = if model.island
    then Scene.group([buildings, Scene.memo("island-props", model.props, Island.propsScene),
                      Scene.memo("island-ground", { biome: model.biome, night: 1.0 - daylight }, Island.ground), Archetypes.resident])
    else Scene.group([town.ground, buildings, Archetypes.resident]) in
  // On XR the beam follows the right controller, mapped through the camera.
  let (from, dir) = match model.hand with
    | Option.None => (eye |> Vec3.add(v(0.0, -0.25, 0.0)), lookDir(model.yaw, model.pitch))
    | Option.Some(pose) =>
      let m = Camera3D.mapTrackedPose(camera, pose) in
      (v(m.position.x, m.position.y, m.position.z), v(m.forward.x, m.forward.y, m.forward.z)) in
  let flashlight = if model.flashlight
    then [Light.spot(from, dir, flashlightColor, flashlightIntensity, flashlightRange, flashlightCone)] else [] in
  let lights = frameLights(o, Block.near(town, town.lightReach, eye) |> List.map((e) => let (id, d) = e in Block.copyIn(town, id)), eye, flashlight) in
  Frame.createLit(camera, scene, if baked then lights else lights |> List.append([unbakedLight]))
    |> Frame.withFog(Fog.linear(o.fogNear, far, Hours.color(o.sky)))
    |> Frame.withClearColor(Hours.color(o.sky))

// Only shown when an archetype's committed bake doesn't match its layout.
let ui = (model) =>
  match Archetypes.stale |> Option.map((a) => a.status) |> Option.defaultValue(Provenance.Current) with
  | Provenance.Current => Ui.column([])
  | Provenance.Missing => Ui.text("no bake: lighting is flat. Run node examples/liminal-office/bake.mjs") |> Ui.panel(Ui.topLeft())
  | Provenance.Stale(why) => Ui.text($"stale bake ({why}): lighting is flat. Run node examples/liminal-office/bake.mjs") |> Ui.panel(Ui.topLeft())

// Quest: 120 Hz with foveation off, the liminal-city perf baseline.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120))
let settings = (model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))
