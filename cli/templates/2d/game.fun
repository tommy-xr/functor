// A 2D Functor starter. Move the player with WASD or the arrow keys. Run it with:
//
//   functor run native
//
// Saving this file while the game runs hot-reloads it immediately.

// The view is 32 x 18 world units, centred on the origin with +y up. A window
// of another shape letterboxes instead of stretching.
let camera = Camera2D.create(32.0, 18.0)

let speed = 8.0        // world units per second
let radius = 0.8       // the player's size
let halfW = 14.0       // the playfield's half-extent
let halfH = 7.0

let background = Color.rgb(0.04, 0.03, 0.09)
let field = Color.rgb(0.08, 0.09, 0.18)
let ink = Color.rgb(0.75, 0.8, 0.95)
let cyan = Color.rgb(0.2, 0.9, 1.0)
let pink = Color.rgb(1.0, 0.35, 0.7)
let amber = Color.rgb(1.0, 0.8, 0.3)

let init = { x: 0.0, y: 0.0, dx: 0.0, dy: 0.0 }

let held = (keys: List<Key.t>, a: Key.t, b: Key.t) =>
  List.any((k) => k == a || k == b, keys)

let axis = (negative: bool, positive: bool) =>
  (if positive then 1.0 else 0.0) - (if negative then 1.0 else 0.0)

// Held keys become a direction once per fixed step.
let sampledInput = (model, snapshot: Input.snapshot) =>
  let keys = snapshot.heldKeys in
  { model with
      dx: axis(held(keys, Key.A, Key.Left), held(keys, Key.D, Key.Right)),
      dy: axis(held(keys, Key.S, Key.Down), held(keys, Key.W, Key.Up)) }

// Move, then keep the whole player inside the playfield.
let tick = (model, dt, tts) =>
  { model with
      x: Math.clamp(radius - halfW, halfW - radius, model.x + model.dx * speed * dt),
      y: Math.clamp(radius - halfH, halfH - radius, model.y + model.dy * speed * dt) }

let draw = (model, tts) =>
  Sprite.group([
    Sprite.rectangle(field, halfW * 2.0, halfH * 2.0),
    Sprite.text(ink, 0.6, "WASD / ARROWS TO MOVE") |> Sprite.moveY(halfH + 0.8),
    Sprite.polygon(pink, [{ x: -1.2, y: -1.0 }, { x: 1.2, y: -1.0 }, { x: 0.0, y: 1.2 }])
      |> Sprite.move(-8.0, 3.0),
    Sprite.square(amber, 1.8)
      |> Sprite.rotate(Angle.radians(tts))
      |> Sprite.move(8.0, -3.0),
    Sprite.circle(cyan, radius) |> Sprite.move(model.x, model.y),
  ])
    |> Frame.create2D(camera)
    |> Frame.withClearColor(background)

// `functor test` runs every `expect` in the project.
expect tick({ init with dx: 1.0 }, 0.5, 0.0).x == speed * 0.5
expect tick({ init with x: halfW, dy: -1.0 }, 0.1, 0.0).x == halfW - radius
expect axis(true, false) == -1.0
