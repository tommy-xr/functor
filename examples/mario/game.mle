// mario — a pure-model side-view platformer (walk, jump, gravity) with a
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
let jumpVelocity = 12.0   // upward launch speed on jump
let gravity = 30.0        // downward acceleration

// A jump launched at the edge covers runSpeed * (2*jumpVelocity/gravity)
//   = 8 * (2*12/30) = 6.4 units horizontally at the same height it started.
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

// Character box half-extents (feet are at model.y; the box is drawn centered
// half a height above).
let charHalfH = 0.6

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

// Landed = over a platform AND at/below the surface (falling in).
let landed = (nx, ny) =>
  match overSolid(nx) with
  | true => ny < groundTop
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
  | "Left" => { model with leftHeld: isDown }
  | "A" => { model with leftHeld: isDown }
  | "Right" => { model with rightHeld: isDown }
  | "D" => { model with rightHeld: isDown }
  | "Up" => jump(model, isDown)
  | "W" => jump(model, isDown)
  | "Space" => jump(model, isDown)
  | _ => model

let tick = (model, dt, tts) =>
  let vx = dirOf(model) * runSpeed in
  let vy1 = model.vy - gravity * dt in
  let nx = model.x + vx * dt in
  let ny = model.y + vy1 * dt in
  match landed(nx, ny) with
  | true =>
    { model with x: nx, y: groundTop, vx: vx, vy: 0.0, grounded: true }
  | false =>
    (match ny < fallLimit with
     | true => startModel
     | false =>
       { model with x: nx, y: ny, vx: vx, vy: vy1, grounded: false })

// --- Rendering ---

// A ground platform: a wide, deep box whose TOP face sits at groundTop.
let platform = (cx, width) =>
  Scene.cube()
    |> Scene.scaleXYZ(width, 2.0, 4.0)
    |> Scene.lit(0.30, 0.68, 0.36)
    |> Scene.translate(cx, groundTop - 1.0, 0.0)

let character = (model) =>
  Scene.cube()
    |> Scene.scaleXYZ(0.8, 1.2, 0.8)
    |> Scene.lit(0.95, 0.35, 0.25)
    |> Scene.translate(model.x, model.y + charHalfH, 0.0)

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(0.0, 3.0, 20.0, 0.0, 1.0, 0.0),
    Scene.group([
      // Left platform: x in [leftEdge, -chasmHalf]; right: [chasmHalf, rightEdge].
      platform((leftEdge - chasmHalf) / 2.0, -chasmHalf - leftEdge),
      platform((rightEdge + chasmHalf) / 2.0, rightEdge - chasmHalf),
      character(model),
    ]),
    [
      Light.ambient(0.18, 0.18, 0.22),
      Light.directional(-0.4, -1.0, -0.5, 1.0, 0.98, 0.92, 0.9) |> Light.castShadows,
    ])
