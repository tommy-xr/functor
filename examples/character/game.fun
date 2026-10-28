// character — a capsule player walking a small trimesh level on
// `Physics.character`: stairs, a ledge too tall to step onto, two ramps (one
// too steep to stand on) and a wall.
//
//   functor -d examples/character run native
//   functor -d examples/character run vr
//
// Desktop: W/S or Up/Down walk, A/D strafe, Left/Right turn, Space jumps.
// VR: the left thumbstick walks and strafes, the right one turns, A jumps.
//
// The level is ONE `Physics.trimesh`, built once from the pieces the scene
// draws. `tick` reads `Physics.grounded`, keeps the vertical speed in the
// model, and returns one `Physics.move` per frame. Sliding along walls,
// climbing stairs, staying on a ramp going down and sliding off a steep one
// all come from the engine.

let playerTag = Physics.tag("player")
let levelTag = Physics.tag("level")

// A level piece: a box by centre and full extents in metres, tilted `tilt`
// degrees about Z (a ramp rises toward +x).
type Piece = { x: float, y: float, z: float, w: float, h: float, d: float, tilt: float }

let block = (x, y, z, w, h, d): Piece => { x: x, y: y, z: z, w: w, h: h, d: d, tilt: 0.0 }

// A 0.2 m thick, 3 m wide ramp `run` long, rising at `angle` degrees along +x
// from the floor at x0.
let ramp = (x0, z, run, angle): Piece =>
  let a = angle * Math.pi / 180.0 in
  { x: x0 + run / 2.0 * Math.cos(a), y: run / 2.0 * Math.sin(a) - 0.1 * Math.cos(a), z: z,
    w: run, h: 0.2, d: 3.0, tilt: angle }

let floor = block(0.0, -0.5, 0.0, 24.0, 1.0, 24.0)

let pieces = [
  floor,
  // Two 0.25 m stairs up to a 0.75 m landing: each rise is under the 0.35 m
  // step height.
  block(2.3, 0.125, -5.0, 0.6, 0.25, 3.0),
  block(2.9, 0.25, -5.0, 0.6, 0.5, 3.0),
  block(4.7, 0.375, -5.0, 3.0, 0.75, 3.0),
  // A 0.6 m ledge: too tall to step onto, so it blocks.
  block(4.0, 0.3, -1.0, 2.0, 0.6, 2.0),
  // A walkable 20° ramp up to a platform, and a 55° one that slides you off.
  ramp(1.0, 3.0, 6.0, 20.0),
  block(8.1, 1.025, 3.0, 3.0, 2.05, 3.0),
  ramp(1.0, 7.0, 2.5, 55.0),
  // A wall to slide along.
  block(-6.0, 1.5, 0.0, 0.4, 3.0, 10.0),
]

// A box's 8 corners (x varies fastest, then y, then z) and its 12 triangles,
// wound counter-clockwise seen from outside: trimesh faces collide from the
// front.
let signs = [
  { x: -1.0, y: -1.0, z: -1.0 }, { x: 1.0, y: -1.0, z: -1.0 },
  { x: -1.0, y: 1.0, z: -1.0 }, { x: 1.0, y: 1.0, z: -1.0 },
  { x: -1.0, y: -1.0, z: 1.0 }, { x: 1.0, y: -1.0, z: 1.0 },
  { x: -1.0, y: 1.0, z: 1.0 }, { x: 1.0, y: 1.0, z: 1.0 },
]
let boxTriangles = [
  0.0, 3.0, 1.0, 0.0, 2.0, 3.0, 4.0, 7.0, 6.0, 4.0, 5.0, 7.0,
  0.0, 5.0, 4.0, 0.0, 1.0, 5.0, 2.0, 7.0, 3.0, 2.0, 6.0, 7.0,
  0.0, 6.0, 2.0, 0.0, 4.0, 6.0, 1.0, 7.0, 5.0, 1.0, 3.0, 7.0,
]

let corners = (p: Piece) =>
  let a = p.tilt * Math.pi / 180.0 in
  signs |> List.concatMap((k) =>
    let lx = k.x * p.w / 2.0 in
    let ly = k.y * p.h / 2.0 in
    [p.x + lx * Math.cos(a) - ly * Math.sin(a),
     p.y + lx * Math.sin(a) + ly * Math.cos(a),
     p.z + k.z * p.d / 2.0])

// Built once, here at the top level: the hook re-declares this value every
// frame, and its collision structure is built only for the first.
let levelShape = Physics.trimesh({
  positions: Bytes.fromF32le(pieces |> List.concatMap(corners)),
  triangles: Bytes.fromU32le(
    pieces |> List.indexedMap((i, p) => boxTriangles |> List.map((t) => t + 8.0 * i))
           |> List.flatten),
})

// The capsule is 1.8 m tall: its centre stands 0.9 m (plus the offset) above
// the feet.
let spawn = Vec3.make(0.0, 0.92, -5.0)
let eyeAboveCentre = 0.7

let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0), [
    Physics.fixed(levelTag, levelShape),
    Physics.character(playerTag, Physics.capsule(0.6, 0.3),
                      { stepHeight: 0.35, maxSlope: Angle.degrees(45.0), snap: 0.3, offset: 0.02 })
      |> Physics.at(spawn),
  ])

let gravity = 9.81
let walkSpeed = 3.0
let turnRate = 2.0
let jumpSpeed = 4.5

// Facing +x, toward the stairs.
let init = { yaw: Math.pi / 2.0, vy: 0.0, walk: 0.0, strafe: 0.0, turn: 0.0, jump: false, jumpHeld: false }

let held = (k, snap) => snap.heldKeys |> List.any((h) => h == k)
let axis = (neg, pos, snap) =>
  (if held(pos, snap) then 1.0 else 0.0) - (if held(neg, snap) then 1.0 else 0.0)
let deadzone = (v) => if Math.abs(v) < 0.15 then 0.0 else v
let unit = (v) => Math.clamp(-1.0, 1.0, v)
let still = { x: 0.0, y: 0.0 }

// Held keys and thumbsticks, sampled once per fixed step. Turning left is
// positive yaw.
let sampledInput = (model, snap: Input.snapshot) =>
  let pad = (match snap.xr with
    | Option.Some(xr) => { stick: xr.left.thumbstick, look: xr.right.thumbstick, jump: xr.right.primaryPressed }
    | Option.None => { stick: still, look: still, jump: false }) in
  let jumping = held(Key.Space, snap) || pad.jump in
  { model with
    walk: unit(axis(Key.S, Key.W, snap) + axis(Key.Down, Key.Up, snap) + deadzone(pad.stick.y)),
    strafe: unit(axis(Key.A, Key.D, snap) + deadzone(pad.stick.x)),
    turn: unit(axis(Key.Right, Key.Left, snap) - deadzone(pad.look.x)),
    // A rising edge, latched until `tick` uses it.
    jump: model.jump || (jumping && not model.jumpHeld),
    jumpHeld: jumping }

let tick = (model, dt, tts) =>
  let grounded = Physics.grounded(playerTag) in
  let yaw = model.yaw + model.turn * turnRate * dt in
  // Forward is (sin yaw, cos yaw); right is (-cos yaw, sin yaw).
  let vx = walkSpeed * (model.walk * Math.sin(yaw) - model.strafe * Math.cos(yaw)) in
  let vz = walkSpeed * (model.walk * Math.cos(yaw) + model.strafe * Math.sin(yaw)) in
  // The ground resets the fall, but gravity always pulls: a move needs a
  // downward part to follow stairs and ramps down.
  let vy = (if grounded && model.jump then jumpSpeed else if grounded then 0.0 else model.vy)
           - gravity * dt in
  let next = { model with yaw: yaw, vy: vy, jump: false } in
  if Physics.position(playerTag).y < -10.0 then
    ({ next with vy: 0.0 }, Physics.teleport(playerTag, spawn))
  else
    (next, Physics.move(playerTag, Vec3.make(vx * dt, vy * dt, vz * dt)))

let pieceVisual = (p: Piece) =>
  Scene.cube()
    |> Scene.scaleXYZ(p.w, p.h, p.d)
    |> Scene.lit(if p == floor then Color.rgb(0.30, 0.33, 0.38)
                 else if p.tilt > 0.0 then Color.rgb(0.85, 0.55, 0.25)
                 else Color.rgb(0.55, 0.60, 0.68))
    |> Scene.rotateZ(Angle.degrees(p.tilt))
    |> Scene.translate(Vec3.make(p.x, p.y, p.z))

let draw = (model, tts) =>
  let p = Physics.position(playerTag) in
  Frame.createLit(
    Camera3D.firstPerson(Vec3.make(p.x, p.y + eyeAboveCentre, p.z),
                         Angle.radians(model.yaw), Angle.degrees(0.0), Angle.degrees(70.0)),
    Scene.group(pieces |> List.map(pieceVisual)),
    [
      Light.ambient(Color.rgb(0.30, 0.32, 0.38)),
      Light.directional(Vec3.make(-0.5, -0.8, 0.3), Color.rgb(1.0, 0.97, 0.92), 1.0),
    ])

let ui = (model) =>
  let p = Physics.position(playerTag) in
  Ui.column([
    Ui.text(Text.join("", ["grounded  ", if Physics.grounded(playerTag) then "yes" else "no"])),
    Ui.text(Text.join("", ["position  ", Text.fixed(p.x, 2.0), ", ", Text.fixed(p.y, 2.0), ", ",
                           Text.fixed(p.z, 2.0)])),
  ])
    |> Ui.panel(Ui.topLeft())

// The level data is whole: 8 corners and 36 indices per piece.
expect List.length(corners(floor)) == 24.0
expect List.length(boxTriangles) == 36.0
