// Pad — pure floating-thumbstick math over the sampled touch domain.
//
// A stick is CLAIMED by the first touch that begins inside its zone: the
// stick's center anchors where the finger landed (the classic mobile
// "floating stick"), deflection is the finger's offset from that anchor, and
// lifting the finger releases the stick. All plain data and pure functions —
// the `expect`s below run under `functor test` with no GPU, window, or real
// touches, and the game folds one `Pad.update` per stick per sampled step.

/// One floating stick: inactive, or anchored at the claim point and
/// following contact `id`. Coordinates are surface CSS pixels, like the
/// touch domain itself.
type Stick = {
  active: bool,
  id: float,
  anchorX: float,
  anchorY: float,
  x: float,
  y: float
}

let idle = { active: false, id: 0.0, anchorX: 0.0, anchorY: 0.0, x: 0.0, y: 0.0 }

// Follow the claimed contact: reposition with it, release when it lifts. A
// contact that vanishes from the held list without a release edge (a swept
// snapshot) also releases — a stick must never track a ghost.
let track = (touch: Input.touch, stick: Stick): Stick =>
  if not stick.active then stick
  else if touch.released |> List.any((p: Input.touchPoint) => p.id == stick.id) then idle
  else
    touch.touches
    |> List.find((p: Input.touchPoint) => p.id == stick.id)
    |> Option.map((p: Input.touchPoint) => { stick with x: p.x, y: p.y })
    |> Option.defaultValue(idle)

// Claim: an inactive stick anchors on the first press inside its zone.
let claim = (inZone: (Input.touchPoint) => bool, touch: Input.touch, stick: Stick): Stick =>
  if stick.active then stick
  else
    touch.pressed
    |> List.filter(inZone)
    |> List.head
    |> Option.map((p: Input.touchPoint) =>
        { active: true, id: p.id, anchorX: p.x, anchorY: p.y, x: p.x, y: p.y })
    |> Option.defaultValue(stick)

/// Fold one sampled touch record into the stick: existing contacts tracked
/// (and released) first, then a fresh press inside `inZone` may claim it.
let update = (inZone: (Input.touchPoint) => bool, touch: Input.touch, stick: Stick): Stick =>
  claim(inZone, touch, track(touch, stick))

/// The stick's deflection as a unit-clamped vector in STICK space (`x` right,
/// `y` down — surface pixels; negate `y` for an up-positive world). `radius`
/// is the pixel deflection that reads as full tilt; a radial dead zone keeps
/// a resting thumb from drifting the player.
let vector = (radius: float, stick: Stick): Input.point2 =>
  if not stick.active then { x: 0.0, y: 0.0 }
  else
    let dx = (stick.x - stick.anchorX) / radius in
    let dy = (stick.y - stick.anchorY) / radius in
    let len = Math.sqrt(dx * dx + dy * dy) in
    if len < 0.15 then { x: 0.0, y: 0.0 }
    else if len > 1.0 then { x: dx / len, y: dy / len }
    else { x: dx, y: dy }

// --- Inline tests (functor test) -------------------------------------------

let noTouches: Input.touch = { touches: [], pressed: [], released: [] }
let anywhere = (p: Input.touchPoint) => true
let leftHalf = (p: Input.touchPoint) => p.x < 400.0

// A press inside the zone claims the stick AT the press point.
expect (
  let press = { noTouches with
                touches: [{ id: 3.0, x: 120.0, y: 300.0 }],
                pressed: [{ id: 3.0, x: 120.0, y: 300.0 }] } in
  let s = update(leftHalf, press, idle) in
  s.active && s.id == 3.0 && s.anchorX == 120.0 && s.anchorY == 300.0
)

// A press OUTSIDE the zone is ignored.
expect (
  let press = { noTouches with
                touches: [{ id: 0.0, x: 700.0, y: 300.0 }],
                pressed: [{ id: 0.0, x: 700.0, y: 300.0 }] } in
  not update(leftHalf, press, idle).active
)

// Movement deflects relative to the ANCHOR, and a release resets to idle.
expect (
  let press = { noTouches with
                touches: [{ id: 1.0, x: 100.0, y: 100.0 }],
                pressed: [{ id: 1.0, x: 100.0, y: 100.0 }] } in
  let moved = { noTouches with touches: [{ id: 1.0, x: 160.0, y: 100.0 }] } in
  let lifted = { noTouches with released: [{ id: 1.0, x: 160.0, y: 100.0 }] } in
  let held = update(anywhere, moved, update(anywhere, press, idle)) in
  let v = vector(60.0, held) in
  v.x == 1.0 && v.y == 0.0 && not update(anywhere, lifted, held).active
)

// Deflection clamps to the unit circle beyond `radius`…
expect (
  let s = { active: true, id: 0.0, anchorX: 0.0, anchorY: 0.0, x: 120.0, y: 0.0 } in
  vector(60.0, s).x == 1.0
)

// …and rests inside the dead zone.
expect (
  let s = { active: true, id: 0.0, anchorX: 0.0, anchorY: 0.0, x: 5.0, y: 3.0 } in
  let v = vector(60.0, s) in
  v.x == 0.0 && v.y == 0.0
)

// A second finger cannot steal an already-claimed stick.
expect (
  let press = { noTouches with
                touches: [{ id: 1.0, x: 100.0, y: 100.0 }],
                pressed: [{ id: 1.0, x: 100.0, y: 100.0 }] } in
  let second = { noTouches with
                 touches: [{ id: 1.0, x: 100.0, y: 100.0 }, { id: 2.0, x: 50.0, y: 50.0 }],
                 pressed: [{ id: 2.0, x: 50.0, y: 50.0 }] } in
  let s = update(anywhere, second, update(anywhere, press, idle)) in
  s.id == 1.0 && s.anchorX == 100.0
)

// A contact that vanishes without a release edge still frees the stick.
expect (
  let press = { noTouches with
                touches: [{ id: 1.0, x: 100.0, y: 100.0 }],
                pressed: [{ id: 1.0, x: 100.0, y: 100.0 }] } in
  not update(anywhere, noTouches, update(anywhere, press, idle)).active
)
