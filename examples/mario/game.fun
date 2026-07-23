// mario — a pure-model 2D sprite platformer (walk, jump, gravity) with a
// CHASM: two ground platforms with a visible gap between them. Inspired by
// Elm's classic Mario demo. Everything the simulation needs lives IN the
// model, and there is deliberately NO `physics` hook — the whole state
// forward-steps in `tick`, so the game can be rewound / ghosted / replayed
// exactly. This is the vehicle for a "rewind + tweak a constant + replay the
// jump" time-travel demo.
//
// Run with:
//   functor -d examples/mario run native
//
// Controls (held-key movement, Elm-Mario style — `tick` does the moving):
//   Left / A , Right / D  — run
//   Up / W / Space        — jump (only when grounded)

// --- Tunables (tweak these to change whether the jump clears the chasm) ---
let runSpeed = 8.0        // horizontal speed while a direction is held
let jumpVelocity = 13.0   // upward launch speed on jump
let gravity = 30.0        // downward acceleration

// A jump launched at the edge covers runSpeed * (2*jumpVelocity/gravity)
//   = 8 * (2*13/30) = 6.93 units horizontally at the same height it started.
// The chasm is 6.0 wide (chasmHalf 3.0), so the DEFAULT jump clears it with
// a small margin — but lowering jumpVelocity (or raising gravity) a little
// makes the character fall short. That knife-edge is the whole point.

// --- Level geometry (side view: XY plane, +X right, +Y up) ---
let groundTop = 0.0       // y of the platform surface the character stands on
let chasmHalf = 3.0       // gap spans x in [-chasmHalf, chasmHalf] -> width 6.0
let leftEdge = -11.0      // outer x of the left platform
let rightEdge = 11.0      // outer x of the right platform
let startX = -6.0         // character spawn (on the left platform)
let fallLimit = -2.0      // fall past this (into the chasm) and respawn.
                          // Shallow on purpose: a weak jump that dips into the
                          // gap respawns BEFORE its x reaches the right edge
                          // (chasmHalf 3.0), so it visibly FALLS IN rather than
                          // snapping up onto the far platform from below. The
                          // default jump clears with y still above this line.

let startModel =
  { x: startX, y: groundTop, vx: 0.0, vy: 0.0,
    grounded: true, leftHeld: false, rightHeld: false }

let init = startModel

// Held-direction intent: right wins if both are held. -1 left, +1 right, 0 idle.
let dirOf = (model) =>
  match model.rightHeld with
  | true => 1.0
  | false =>
    (match model.leftHeld with
     | true => -1.0
     | false => 0.0)

// Is x horizontally over solid ground (either platform, not the chasm)?
let overLeft = (x) =>
  match x > leftEdge with
  | true => x < -chasmHalf
  | false => false

let overRight = (x) =>
  match x > chasmHalf with
  | true => x < rightEdge
  | false => false

let overSolid = (x) =>
  match overLeft(x) with
  | true => true
  | false => overRight(x)

// Land only while crossing the surface from above. Checking the previous Y
// prevents a late jump from snapping upward through the far platform.
let landed = (wasY, nx, ny) =>
  match overSolid(nx) with
  | true =>
    (match wasY < groundTop with
     | true => false
     | false => ny < groundTop)
  | false => false

// Jump only from the ground; airborne jump requests are ignored (so GLFW key
// repeats can't double-jump — grounded is already false in the air).
let jump = (model, isDown) =>
  match isDown with
  | false => model
  | true =>
    (match model.grounded with
     | true => { model with vy: jumpVelocity, grounded: false }
     | false => model)

let input = (model, key, isDown) =>
  match key with
  | Key.Left => { model with leftHeld: isDown }
  | Key.A => { model with leftHeld: isDown }
  | Key.Right => { model with rightHeld: isDown }
  | Key.D => { model with rightHeld: isDown }
  | Key.Up => jump(model, isDown)
  | Key.W => jump(model, isDown)
  | Key.Space => jump(model, isDown)
  | _ => model

let tick = (model, dt, tts) =>
  let vx = dirOf(model) * runSpeed in
  let vy1 = model.vy - gravity * dt in
  let nx = model.x + vx * dt in
  let ny = model.y + vy1 * dt in
  match landed(model.y, nx, ny) with
  | true =>
    { model with x: nx, y: groundTop, vx: vx, vy: 0.0, grounded: true }
  | false =>
    (match ny < fallLimit with
     | true => startModel
     | false =>
       { model with x: nx, y: ny, vx: vx, vy: vy1, grounded: false })

// --- 2D rendering ---

let sky = () =>
  Sprite.rectangle(Color.rgb(0.20, 0.64, 0.88), 24.0, 13.5)

// Simple geometric scenery keeps the Elm-playground flavor alongside the
// image sprites. Earlier group entries draw behind later ones.
let hill = (x, y, size, color) =>
  Sprite.square(color, size)
    |> Sprite.rotate(Angle.degrees(45.0))
    |> Sprite.move(x, y)

let backdrop = () =>
  Sprite.group([
    sky(),
    hill(-8.0, -3.0, 8.0, Color.rgb(0.35, 0.73, 0.52)),
    hill(-1.0, -3.8, 7.0, Color.rgb(0.29, 0.66, 0.45)),
    hill(7.0, -3.2, 9.0, Color.rgb(0.38, 0.76, 0.50)),
  ])

let groundTile = (x) =>
  Sprite.image(1.0, 1.0, Assets.ground)
    |> Sprite.move(x, groundTop - 0.5)

// A platform combines a cheap filled body with one row of textured top tiles.
// Widths in this level are whole numbers, so List.range gives one tile/unit.
let platform = (cx, width) =>
  let firstX = cx - width / 2.0 + 0.5 in
  let tiles =
    List.range(width)
      |> List.map((i) => groundTile(firstX + i))
      |> Sprite.group in
  Sprite.group([
    Sprite.rectangle(Color.rgb(0.63, 0.39, 0.20), width, 2.0)
      |> Sprite.move(cx, groundTop - 1.5),
    tiles,
  ])

let characterAsset = (model, tts) =>
  if not model.grounded then Assets.hero_jump
  else if model.vx == 0.0 then Assets.hero_idle
  else if Math.mod(Math.floor(tts * 8.0), 2.0) == 0.0 then Assets.hero_walk_1
  else Assets.hero_walk_2

let faceCharacter = (model, sprite) =>
  if model.vx < 0.0 then sprite |> Sprite.scaleXY(-1.0, 1.0) else sprite

let character = (model, tts) =>
  Sprite.image(1.6, 1.6, characterAsset(model, tts))
    |> faceCharacter(model)
    |> Sprite.move(model.x, model.y + 0.8)

let world = (model, tts) =>
  Sprite.group([
    backdrop(),
    // Left platform: x in [leftEdge, -chasmHalf]; right: [chasmHalf, rightEdge].
    platform((leftEdge - chasmHalf) / 2.0, -chasmHalf - leftEdge),
    platform((rightEdge + chasmHalf) / 2.0, rightEdge - chasmHalf),
    character(model, tts),
  ])

let draw = (model, tts) =>
  Frame.create2D(Camera2D.create(24.0, 13.5), world(model, tts))
    |> Frame.withClearColor(Color.rgb(0.08, 0.16, 0.25))
