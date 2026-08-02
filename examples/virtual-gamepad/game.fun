// virtual-gamepad — twin-stick arena driven by TOUCH THUMBSTICKS built in
// pure game code over the sampled touch domain (`snap.touch`), with a
// keyboard fallback so desktop plays too.
//
//   functor -d examples/virtual-gamepad run wasm    # then open it on a phone
//   functor -d examples/virtual-gamepad run native  # WASD moves, arrows aim/fire
//
// Left half of the screen: a floating MOVE stick (anchors where your thumb
// lands). Right half: an AIM stick — deflect past half tilt to fire. The
// stick math lives in the sibling `Pad` module as pure, `expect`-tested
// functions; this file only folds touches in `sampledInput` and draws.
//
// Headless driving (docs/debug-runtime.md): touch is injectable —
//   POST /input {"type":"touch","phase":"begin","id":0,"x":160,"y":400}
//   POST /input {"type":"touch","phase":"move","id":0,"x":220,"y":400}
// moves the player right, exactly as a real thumb would.

// --- Tunables ---------------------------------------------------------------
let stickRadius = 70.0    // px of thumb travel that reads as full tilt
let moveSpeed = 9.0       // world units/s at full tilt
let shotSpeed = 16.0      // projectile speed
let fireTilt = 0.5        // right-stick deflection that starts firing
let fireCooldown = 0.14   // seconds between shots
let shotLife = 1.1        // seconds a shot lives

// --- Model ------------------------------------------------------------------
type Shot = { x: float, y: float, vx: float, vy: float, ttl: float }
type Keys = { moveX: float, moveY: float, aimX: float, aimY: float }
type Model = {
  px: float,
  py: float,
  aimX: float,
  aimY: float,
  cooldown: float,
  shots: List<Shot>,
  left: Pad.Stick,
  right: Pad.Stick,
  keys: Keys,
  surfaceW: float,
  surfaceH: float,
  hasTouch: bool
}

let init = {
  px: 0.0,
  py: 0.0,
  aimX: 1.0,
  aimY: 0.0,
  cooldown: 0.0,
  shots: [],
  left: Pad.idle,
  right: Pad.idle,
  keys: { moveX: 0.0, moveY: 0.0, aimX: 0.0, aimY: 0.0 },
  surfaceW: 800.0,
  surfaceH: 600.0,
  hasTouch: false
}

// --- Input: fold sticks and keyboard once per fixed step --------------------
let axis = (heldKeys: List<Key.t>, minus: Key.t, plus: Key.t) =>
  let held = (k: Key.t) => heldKeys |> List.any((h: Key.t) => h == k) in
  (if held(plus) then 1.0 else 0.0) - (if held(minus) then 1.0 else 0.0)

let sampledInput = (m: Model, snap: Input.snapshot) =>
  let keys = {
    moveX: axis(snap.heldKeys, Key.A, Key.D),
    moveY: axis(snap.heldKeys, Key.S, Key.W),
    aimX: axis(snap.heldKeys, Key.Left, Key.Right),
    aimY: axis(snap.heldKeys, Key.Down, Key.Up)
  } in
  let sized = { m with
    keys: keys,
    surfaceW: Math.max(1.0, snap.mouse.surfaceWidth),
    surfaceH: Math.max(1.0, snap.mouse.surfaceHeight) } in
  match snap.touch with
  | Option.Some(t) =>
    let mid = sized.surfaceW / 2.0 in
    { sized with
      hasTouch: true,
      left: Pad.update((p: Input.touchPoint) => p.x < mid, t, sized.left),
      right: Pad.update((p: Input.touchPoint) => p.x >= mid, t, sized.right) }
  | Option.None => sized

// --- Simulation -------------------------------------------------------------
// Stick vectors are surface-space (y down); the world is up-positive.
let moveVec = (m: Model): Input.point2 =>
  let pad = Pad.vector(stickRadius, m.left) in
  if pad.x != 0.0 || pad.y != 0.0 then { x: pad.x, y: 0.0 - pad.y }
  else { x: m.keys.moveX, y: m.keys.moveY }

let aimVec = (m: Model): Input.point2 =>
  let pad = Pad.vector(stickRadius, m.right) in
  if pad.x != 0.0 || pad.y != 0.0 then { x: pad.x, y: 0.0 - pad.y }
  else { x: m.keys.aimX, y: m.keys.aimY }

let arenaHalfW = 7.6
let arenaHalfH = 4.2

let stepShot = (dt: float, s: Shot): Shot =>
  { s with x: s.x + s.vx * dt, y: s.y + s.vy * dt, ttl: s.ttl - dt }

let tick = (m: Model, dt, tts) =>
  let mv = moveVec(m) in
  let px = Math.clamp(0.0 - arenaHalfW, arenaHalfW, m.px + mv.x * moveSpeed * dt) in
  let py = Math.clamp(0.0 - arenaHalfH, arenaHalfH, m.py + mv.y * moveSpeed * dt) in
  let aim = aimVec(m) in
  let tilt = Math.sqrt(aim.x * aim.x + aim.y * aim.y) in
  let aimed =
    if tilt > 0.0 then { m with px: px, py: py, aimX: aim.x / tilt, aimY: aim.y / tilt }
    else { m with px: px, py: py } in
  let flying =
    aimed.shots
    |> List.map((s: Shot) => stepShot(dt, s))
    |> List.filter((s: Shot) => s.ttl > 0.0) in
  if tilt >= fireTilt && aimed.cooldown <= 0.0 then
    let shot = {
      x: px,
      y: py,
      vx: aimed.aimX * shotSpeed,
      vy: aimed.aimY * shotSpeed,
      ttl: shotLife
    } in
    { aimed with shots: [shot, ..flying], cooldown: fireCooldown }
  else
    { aimed with shots: flying, cooldown: Math.max(0.0, aimed.cooldown - dt) }

// --- Drawing ----------------------------------------------------------------
let camW = 16.0
let camH = 9.0

// Surface CSS pixels (top-left origin) → world units (center origin, +Y up),
// for drawing the stick overlays where the thumbs actually are.
let toWorldX = (m: Model, sx: float) => (sx / m.surfaceW - 0.5) * camW
let toWorldY = (m: Model, sy: float) => (0.5 - sy / m.surfaceH) * camH
let pxToWorld = (m: Model, px: float) => px / m.surfaceW * camW

let stickOverlay = (m: Model, stick: Pad.Stick) =>
  if not stick.active then Sprite.blank()
  else
    let base = pxToWorld(m, stickRadius) in
    Sprite.group([
      Sprite.circle(Color.rgb(0.5, 0.75, 0.95), base)
        |> Sprite.fade(0.18)
        |> Sprite.move(toWorldX(m, stick.anchorX), toWorldY(m, stick.anchorY)),
      Sprite.circle(Color.rgb(0.75, 0.9, 1.0), base * 0.35)
        |> Sprite.fade(0.45)
        |> Sprite.move(toWorldX(m, stick.x), toWorldY(m, stick.y)),
    ])

let shotSprite = (s: Shot) =>
  Sprite.circle(Color.rgb(1.0, 0.85, 0.4), 0.09)
    |> Sprite.fade(Math.min(1.0, s.ttl / 0.35))
    |> Sprite.move(s.x, s.y)

let player = (m: Model) =>
  Sprite.group([
    Sprite.circle(Color.rgb(0.35, 0.9, 0.7), 0.34),
    // The barrel shows the aim without needing rotation math.
    Sprite.circle(Color.rgb(0.9, 1.0, 0.95), 0.1)
      |> Sprite.move(m.aimX * 0.4, m.aimY * 0.4),
  ])
  |> Sprite.move(m.px, m.py)

let hint = (m: Model) =>
  let label =
    if m.hasTouch then "left thumb: move    right thumb: aim + fire"
    else "WASD: move    arrows: aim + fire    (touch a phone for thumbsticks)" in
  // Sprite.text is center-anchored, so the caption centers under the arena.
  Sprite.text(Color.rgb(0.65, 0.75, 0.9), 0.34, label)
    |> Sprite.move(0.0, 0.0 - arenaHalfH - 0.36)

let arena = () =>
  Sprite.group([
    Sprite.rectangle(Color.rgb(0.10, 0.12, 0.2), arenaHalfW * 2.0 + 0.4, arenaHalfH * 2.0 + 0.4),
    Sprite.rectangle(Color.rgb(0.05, 0.06, 0.11), arenaHalfW * 2.0, arenaHalfH * 2.0),
  ])

let draw = (m: Model, tts) =>
  Frame.create2D(
    Camera2D.create(camW, camH),
    Sprite.group([
      arena(),
      Sprite.group(m.shots |> List.map(shotSprite)),
      player(m),
      hint(m),
      stickOverlay(m, m.left),
      stickOverlay(m, m.right),
    ]),
  )
    |> Frame.withClearColor(Color.rgb(0.03, 0.04, 0.08))

// --- Inline tests: the input → simulation seam ------------------------------

// Full-left-stick tilt moves the player left at moveSpeed.
expect (
  let stick = { active: true, id: 0.0, anchorX: 200.0, anchorY: 300.0, x: 130.0, y: 300.0 } in
  let m = tick({ init with left: stick }, 0.1, 0.1) in
  Math.abs(m.px - (0.0 - 0.9)) < 0.0001 && m.py == 0.0
)

// A deflected right stick fires a shot along the aim; idle sticks fire none.
expect (
  let stick = { active: true, id: 0.0, anchorX: 600.0, anchorY: 300.0, x: 670.0, y: 300.0 } in
  let m = tick({ init with right: stick }, 0.016, 0.016) in
  (m.shots |> List.length) == 1.0 && m.aimX == 1.0 && m.aimY == 0.0
)
expect (tick(init, 0.016, 0.016).shots |> List.length) == 0.0

// Keyboard fallback: surface-independent, up-positive.
expect (
  let keys = { moveX: 0.0, moveY: 1.0, aimX: 0.0, aimY: 0.0 } in
  tick({ init with keys: keys }, 0.1, 0.1).py > 0.0
)

// The player clamps to the arena.
expect (
  let keys = { moveX: 1.0, moveY: 0.0, aimX: 0.0, aimY: 0.0 } in
  tick({ init with keys: keys, px: arenaHalfW }, 0.5, 0.5).px == arenaHalfW
)
