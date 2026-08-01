// examples/shooting-range — a first-person shooting range.
//
//   functor -d examples/shooting-range run native
//
// CONTROLS
//   mouse         look
//   LEFT MOUSE    fire (hold — the weapon is full-auto at ~5.2 rounds/s:
//                 `cooldown` is decremented before the fire check, so the
//                 cadence is ceil(fireInterval / dt) = 11 frames at 60 Hz,
//                 12 at the replay script's 0.016 s timestep).
//                 This is also what the committed `firing.input` replay
//                 script drives, as `0 Mouse.Left down`
//   W A S D       move along the firing line
//   R             reload
//   SPACE         fire (keyboard fallback)
//
// WHAT THIS DEMONSTRATES (that no other example does)
//   * hitscan shooting through `Physics.raycast` — a QUERY effect issued from
//     `tick`, answered against THIS frame's stepped world, folded back through
//     `update`, and visible in the SAME frame's `draw`. The tagger is a
//     closure capturing the shot's origin and direction, so `update` can build
//     the tracer without stashing the aim in the model.
//   * a recoiling weapon: one critically-ish-damped spring per axis, driven by
//     an impulse on each shot. The camera, the aim ray, the viewmodel, and the
//     crosshair bloom all read the SAME spring state, so the gun climbs under
//     sustained fire and your shots climb with it.
//   * a camera-space viewmodel and muzzle, composed by hand (`camRot` below)
//     because the engine exposes no camera-space scene node.
//   * transient VFX as pure model state: tracers and impact sparks are lists of
//     records with a lifetime float (the muzzle flash is a single scalar timer,
//     since there is only ever one), stepped in `tick` and mapped to scene
//     nodes in `draw`. Nothing is retained by the renderer.
//   * a 2D crosshair/hitmarker pass over the 3D frame (`Frame.with2D`) whose
//     spread is a pure function of recoil and movement.
//   * all three input styles at once, on one trigger: `mouseButton` for the
//     rising EDGE of the left button (so a click shorter than a frame still
//     fires a round), `sampledInput` for HELD state — movement keys plus
//     `snapshot.mouse.buttons.left`, which is what makes the weapon full-auto
//     — and `input` for a latched key edge (reload). Mouse-button edges are
//     delivered while the cursor is captured, so clicking to shoot works
//     under free-look.
//
// Everything is a pure function of the model: `--fixed-time` captures are
// deterministic, the whole session time-travels in the scrubber, and hot
// reload keeps your score, your magazine, and the targets mid-fall.

// ---------------------------------------------------------------- tunables

let sensitivity = 0.0025
let pitchLimit = 1.35
let moveSpeed = 4.2
let eyeHeight = 1.65

let magSize = 15.0
let fireInterval = 0.18
let reloadTime = 1.45
let shotRange = 90.0

// Recoil springs: an impulse per shot, then x'' = -k x - c x'.
let recoilKick = 1.05
let recoilYawKick = 0.35
let recoilK = 62.0
let recoilC = 9.5

// Target lifecycle, in seconds.
let popShow = 2.4
let popHide = 1.3
let hitHide = 1.15
let fallTime = 0.26
let riseTime = 0.20

// VFX lifetimes, in seconds.
let tracerLife = 0.075
let sparkLife = 0.30
let flashLife = 0.055
let markLife = 0.28

// Range dimensions.
let rangeHalfWidth = 9.0
let rangeLength = 46.0

// ------------------------------------------------------- plain-data vectors
//
// `Vec3.t` is an opaque engine value with no arithmetic, so the game keeps its
// own plain-data vector and converts at the engine boundary.

type V = { x: float, y: float, z: float }

let vec = (x: float, y: float, z: float): V => { x: x, y: y, z: z }
let vadd = (a: V, b: V): V => vec(a.x + b.x, a.y + b.y, a.z + b.z)
let vmul = (k: float, a: V): V => vec(a.x * k, a.y * k, a.z * k)
let vlen = (a: V): float => Math.sqrt(a.x * a.x + a.y * a.y + a.z * a.z)
let v3 = (a: V) => Vec3.make(a.x, a.y, a.z)

let clamp = (lo: float, hi: float, n: float): float => Math.min(hi, Math.max(lo, n))

// -------------------------------------------------------------------- types

type Kind =
  | Popper
  | Runner

// The previous mouse sample, so the view turns by deltas and never jumps on
// the first event (the `examples/hello` idiom).
type Mouse =
  | NoMouse
  | MouseAt(x: float, y: float)

// One message kind: a raycast answer, carrying the ray it answers so `update`
// can draw the tracer without stashing the aim in the model.
type Msg =
  | Shot(origin: V, dir: V, hit: Physics.rayHit)

// ------------------------------------------------------------- range layout

let target = (id: float, x0: float, z: float, kind: Kind, amp: float, spd: float, phase: float) =>
  { id: id,
    x0: x0,
    z: z,
    kind: kind,
    amp: amp,
    spd: spd,
    phase: phase,
    down: false,
    // Stagger the poppers so the lanes don't blink in unison.
    timer: popShow + id * 0.37,
    anim: 0.0 }

// Six static poppers on three lanes at two distances, plus three runners
// sliding across the far end.
let initialTargets =
  [ target(0.0, 0.0 - 4.2, 13.0, Popper, 0.0, 0.0, 0.0),
    target(1.0, 0.0, 13.0, Popper, 0.0, 0.0, 0.0),
    target(2.0, 4.2, 13.0, Popper, 0.0, 0.0, 0.0),
    target(3.0, 0.0 - 4.2, 23.0, Popper, 0.0, 0.0, 0.0),
    target(4.0, 0.0, 23.0, Popper, 0.0, 0.0, 0.0),
    target(5.0, 4.2, 23.0, Popper, 0.0, 0.0, 0.0),
    target(6.0, 0.0, 34.0, Runner, 6.4, 0.85, 0.0),
    target(7.0, 0.0, 38.5, Runner, 5.2, 0.0 - 1.15, 1.9),
    target(8.0, 0.0, 30.0, Runner, 7.0, 1.45, 3.6) ]

// Branded body identities, derived from the target id so a ray hit can be
// mapped back to the target it struck (tags compare with `==`).
let targetTag = (id: float) => Physics.tag(Text.concat("target-", Text.fixed(id, 0.0)))
let floorTag = Physics.tag("floor")
let backstopTag = Physics.tag("backstop")
let leftWallTag = Physics.tag("wall-left")
let rightWallTag = Physics.tag("wall-right")

// The static shell, declared ONCE as (extents, position) so the collider and
// the drawn box can never drift apart — `physics` and `rangeNode` both build
// from these.
let shellFloor = { w: rangeHalfWidth * 2.0, h: 0.4, d: rangeLength * 2.0,
                   x: 0.0, y: 0.0 - 0.2, z: 0.0 }
let shellBackstop = { w: rangeHalfWidth * 2.0, h: 8.0, d: 0.6,
                      x: 0.0, y: 4.0, z: rangeLength }
let shellLeft = { w: 0.6, h: 8.0, d: rangeLength * 2.0,
                  x: 0.0 - rangeHalfWidth, y: 4.0, z: 0.0 }
let shellRight = { w: 0.6, h: 8.0, d: rangeLength * 2.0,
                   x: rangeHalfWidth, y: 4.0, z: 0.0 }

let targetX = (t) => t.x0 + t.amp * Math.sin(t.phase)
let standing = (t) => not t.down
let points = (t) => 10.0 + Math.floor(t.z)

// --------------------------------------------------------------------- init

let init = {
  eye: { x: 0.0, y: eyeHeight, z: 0.0 - 1.0 },
  yaw: 0.0,
  pitch: 0.0,
  lastMouse: NoMouse,
  // Held state sampled from `Input.snapshot` every fixed step.
  held: { fwd: false, back: false, left: false, right: false, trigger: false },
  // A left-click edge that has not been consumed by a `tick` yet.
  triggerEdge: false,
  walk: 0.0,
  moving: 0.0,
  // Recoil spring state (displacement + velocity, per axis).
  recoilPitch: 0.0,
  recoilVel: 0.0,
  recoilYaw: 0.0,
  recoilYawVel: 0.0,
  ammo: magSize,
  reload: 0.0,
  reloadLatch: false,
  cooldown: 0.0,
  flash: 0.0,
  hitMark: 0.0,
  score: 0.0,
  shots: 0.0,
  hits: 0.0,
  targets: initialTargets,
  tracers: [],
  sparks: [],
  // A fixed seed keeps captures reproducible; recoil jitter draws from it.
  seed: Random.seed(1701.0),
}

// ------------------------------------------------------------ camera basis
//
// Rotate a camera-local offset (+X right-of-frame mirrored, +Y up, +Z forward)
// into world space: Ry(yaw) . Rx(-pitch). `Camera3D.firstPerson` has no
// camera-space scene node, so the viewmodel and the muzzle are placed by hand.

let camRot = (yaw: float, pitch: float, l: V): V =>
  let cp = Math.cos(pitch) in
  let sp = Math.sin(pitch) in
  let y2 = l.y * cp + l.z * sp in
  let z2 = l.z * cp - l.y * sp in
  let cy = Math.cos(yaw) in
  let sy = Math.sin(yaw) in
  vec(l.x * cy + z2 * sy, y2, z2 * cy - l.x * sy)

// The pitch and yaw actually looked along: aim = input + recoil, so shots
// climb with the muzzle instead of being magically stabilised.
let aimPitch = (m) => clamp(0.0 - pitchLimit, pitchLimit, m.pitch + m.recoilPitch)
let aimYaw = (m) => m.yaw + m.recoilYaw

let forwardOf = (m) => camRot(aimYaw(m), aimPitch(m), vec(0.0, 0.0, 1.0))
let toWorld = (m, l: V) => vadd(m.eye, camRot(aimYaw(m), aimPitch(m), l))

// Where the weapon viewmodel hangs off the camera. The muzzle is derived from
// the SAME transform the viewmodel is built from, so the flash and the tracer
// leave the barrel the player can see instead of a second, drifting copy of it.
// Every term the viewmodel applies (recoil kick, walk bob, walk sway) has to
// appear here too — an earlier version omitted all three, which pulled the
// muzzle ~5-9 cm off the rendered barrel tip exactly while firing.
let gunScale = 0.60
let gunMount = vec(0.0 - 0.205, 0.0 - 0.200, 0.62)
let barrelTipLocal = vec(0.0, 0.012, 0.80)
// A small cant so the weapon's side reads instead of a black end-on blob.
let gunCant = 0.0 - 0.17

let gunBob = (m) => Math.sin(m.walk * 9.5) * 0.011 * Math.clamp01(m.moving)
let gunSway = (m) => Math.cos(m.walk * 4.75) * 0.009 * Math.clamp01(m.moving)

// The viewmodel's camera-local translation, shared by the gun and the muzzle.
let gunOffset = (m) =>
  vec(gunMount.x + gunSway(m),
      gunMount.y + gunBob(m),
      gunMount.z - 0.35 * m.recoilPitch)

// `camRot(yaw, pitch, l)` is Ry(yaw).Rx(-pitch), so Ry(gunCant) is
// `camRot(gunCant, 0, _)` and the viewmodel's `rotateX(-0.7 * kick)` is
// `camRot(0, 0.7 * kick, _)`.
let barrelTipCanted = vmul(gunScale, camRot(gunCant, 0.0, barrelTipLocal))
let muzzleLocalOf = (m) =>
  vadd(gunOffset(m), camRot(0.0, 0.7 * m.recoilPitch, barrelTipCanted))
let muzzleOf = (m) => toWorld(m, muzzleLocalOf(m))

// -------------------------------------------------------------------- input

let mouseMove = (model, x, y) =>
  match model.lastMouse with
  | NoMouse => { model with lastMouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    { model with
        yaw: model.yaw - (x - lastX) * sensitivity,
        pitch: clamp(0.0 - pitchLimit, pitchLimit, model.pitch - (y - lastY) * sensitivity),
        lastMouse: MouseAt(x, y) }

// Reload is an EDGE: key repeats arrive as `isDown = true`, so latch.
// SPACE is the trigger's keyboard fallback, and gets the same rising edge the
// left mouse button does below — otherwise a tap shorter than one fixed step
// would fire from the mouse but not from the key.
let input = (model, key, isDown) =>
  match key with
  | Key.Space => if isDown then { model with triggerEdge: true } else model
  | Key.R =>
    (match isDown with
     | false => { model with reloadLatch: false }
     | true =>
       if model.reloadLatch || model.reload > 0.0 || model.ammo == magSize then
         { model with reloadLatch: true }
       else
         { model with reloadLatch: true, reload: reloadTime })
  | _ => model

// The trigger's rising EDGE. Held fire comes from the snapshot below, so all
// this has to catch is the case the level state cannot: a click pressed AND
// released inside one frame, which would otherwise never appear as held.
// The latch is consumed by the next `tick`.
let mouseButton = (model, button, isDown) =>
  match button with
  | Mouse.Left => if isDown then { model with triggerEdge: true } else model
  | _ => model

let heldKey = (k, keys) => keys |> List.any((x) => x == k)

// Continuous state: movement and the trigger are HELD, not edge-triggered.
// The trigger reads the mouse's held left button — the level twin of the
// `mouseButton` edge above — with SPACE as a keyboard fallback.
let sampledInput = (model, snapshot) =>
  let keys = snapshot.heldKeys in
  { model with
      held: {
        fwd: heldKey(Key.W, keys) || heldKey(Key.Up, keys),
        back: heldKey(Key.S, keys) || heldKey(Key.Down, keys),
        left: heldKey(Key.A, keys) || heldKey(Key.Left, keys),
        right: heldKey(Key.D, keys) || heldKey(Key.Right, keys),
        trigger: snapshot.mouse.buttons.left || heldKey(Key.Space, keys),
      } }

// --------------------------------------------------------------------- step

let axis = (neg, pos) =>
  (if pos then 1.0 else 0.0) - (if neg then 1.0 else 0.0)

// Walk in the ground plane relative to the look direction. forward =
// (sin yaw, 0, cos yaw), screen-right = (-cos yaw, 0, sin yaw), and the eye is
// clamped to the firing line so you can't wander downrange into the targets.
let stepPlayer = (dt, m) =>
  let f = axis(m.held.back, m.held.fwd) in
  let r = axis(m.held.left, m.held.right) in
  let mag = vlen(vec(f, 0.0, r)) in
  // Normalise diagonals so strafing forward isn't 41% faster.
  let k = moveSpeed * dt * (if mag > 1.0 then 1.0 / mag else 1.0) in
  let e = m.eye in
  { m with
      eye: {
        x: clamp(0.0 - rangeHalfWidth + 1.0, rangeHalfWidth - 1.0,
                 e.x + k * (f * Math.sin(m.yaw) - r * Math.cos(m.yaw))),
        y: eyeHeight,
        z: clamp(0.0 - 4.0, 5.0,
                 e.z + k * (f * Math.cos(m.yaw) + r * Math.sin(m.yaw))) },
      // The bob phase must advance with the speed actually travelled, not the
      // raw axis magnitude — otherwise a diagonal bobs 41% too fast.
      walk: m.walk + Math.min(1.0, mag) * dt,
      moving: mag }

// One damped-spring integration step: returns (displacement', velocity').
let spring = (dt, x, v) =>
  let v2 = v + (0.0 - recoilK * x - recoilC * v) * dt in
  (x + v2 * dt, v2)

let stepWeapon = (dt, m) =>
  let (rp, rpv) = spring(dt, m.recoilPitch, m.recoilVel) in
  let (ry, ryv) = spring(dt, m.recoilYaw, m.recoilYawVel) in
  let rl = Math.max(0.0, m.reload - dt) in
  let finished = m.reload > 0.0 && rl <= 0.0 in
  { m with
      recoilPitch: rp, recoilVel: rpv,
      recoilYaw: ry, recoilYawVel: ryv,
      reload: rl,
      ammo: if finished then magSize else m.ammo,
      cooldown: Math.max(0.0, m.cooldown - dt),
      flash: Math.max(0.0, m.flash - dt / flashLife),
      hitMark: Math.max(0.0, m.hitMark - dt / markLife) }

// Targets: `anim` is 0 when fully standing and 1 when fully knocked flat, so
// one float drives both the fall and the rise. Poppers cycle on their timer;
// runners only go down when they are hit.
let stepTarget = (dt, t) =>
  let anim = if t.down then Math.min(1.0, t.anim + dt / fallTime)
             else Math.max(0.0, t.anim - dt / riseTime) in
  let phase = t.phase + t.spd * dt in
  let timer = t.timer - dt in
  let moved = { t with anim: anim, phase: phase } in
  if timer > 0.0 then { moved with timer: timer }
  else
    (match t.kind with
     | Popper =>
       let nowDown = not t.down in
       { moved with down: nowDown, timer: if nowDown then popHide else popShow }
     | Runner =>
       if t.down then { moved with down: false, timer: popShow }
       else { moved with timer: popShow })

let stepVfx = (dt, m) =>
  { m with
      targets: m.targets |> List.map((t) => stepTarget(dt, t)),
      tracers: m.tracers
        |> List.map((s) => { s with t: s.t - dt / tracerLife })
        |> List.filter((s) => s.t > 0.0),
      sparks: m.sparks
        |> List.map((s) => { s with t: s.t - dt / sparkLife })
        |> List.filter((s) => s.t > 0.0) }

let weaponReady = (m) => m.cooldown <= 0.0 && m.reload <= 0.0 && m.ammo > 0.0

// A shot: spend a round, kick both springs, and issue the hitscan query. The
// tagger closes over the ray so `update` can draw the tracer from it.
let fire = (m) =>
  // The ray leaves the EYE (so the shot lands exactly where the crosshair is,
  // with no muzzle parallax), while the tracer is drawn from the muzzle the
  // player can see. Every shooter does this; the two only visibly disagree
  // within the first metre.
  let o = muzzleOf(m) in
  let rayOrigin = toWorld(m, vec(0.0, 0.0, 0.12)) in
  let d = forwardOf(m) in
  let (jitter, seed2) = Random.range(0.0 - 1.0, 1.0, m.seed) in
  ({ m with
       ammo: m.ammo - 1.0,
       shots: m.shots + 1.0,
       cooldown: fireInterval,
       flash: 1.0,
       seed: seed2,
       recoilVel: m.recoilVel + recoilKick,
       recoilYawVel: m.recoilYawVel + jitter * recoilYawKick },
   Effect.batch([
     Physics.raycast(v3(rayOrigin), v3(d), shotRange, (h) => Shot(o, d, h)),
     Effect.play(Assets.shot),
   ]))

let tick = (model, dt, tts) =>
  let stepped = stepVfx(dt, stepWeapon(dt, stepPlayer(dt, model))) in
  // The trigger is pulled if it is held OR if a click edge landed since the
  // last step. Either way the edge is spent by this step, so a click never
  // queues up a second round behind the cooldown.
  let pulled = stepped.held.trigger || stepped.triggerEdge in
  let m = { stepped with triggerEdge: false } in
  // Every branch returns the same (model, effect) shape.
  if pulled && weaponReady(m) then fire(m)
  // Pulling an empty trigger starts the reload for you.
  else if pulled && m.ammo <= 0.0 && m.reload <= 0.0 then
    ({ m with reload: reloadTime }, Effect.none())
  else (m, Effect.none())

// The trigger contract, checked headlessly by `functor -d examples/shooting-range test`.
// A left-click edge alone fires a round, and the edge is spent by the step
// that saw it — so one click is exactly one shot.
expect mouseButton(init, Mouse.Left, true).triggerEdge
expect not mouseButton(init, Mouse.Right, true).triggerEdge
// The SPACE fallback latches the SAME edge, so a sub-frame tap fires from the
// keyboard exactly as it does from the mouse.
expect input(init, Key.Space, true).triggerEdge
expect (
  let tapped = input(init, Key.Space, true) in
  let (fired, _fx) = tick(tapped, 0.016, 0.016) in
  fired.shots == 1.0 && not fired.triggerEdge
)
expect (
  let clicked = mouseButton(init, Mouse.Left, true) in
  let (fired, _fx) = tick(clicked, 0.016, 0.016) in
  fired.shots == 1.0 && not fired.triggerEdge && fired.ammo == magSize - 1.0
)
// An untouched trigger fires nothing, and the cooldown holds the next round.
expect (
  let (idle, _fx) = tick(init, 0.016, 0.016) in
  idle.shots == 0.0
)
expect (
  let clicked = mouseButton(init, Mouse.Left, true) in
  let (fired, _fx) = tick(clicked, 0.016, 0.016) in
  let (again, _fx2) = tick(mouseButton(fired, Mouse.Left, true), 0.016, 0.016) in
  again.shots == 1.0
)

// The other half of the trigger: HELD fire through `sampledInput`. A snapshot
// is plain data, so the level path is testable without a window too.
let snapshotWith = (leftDown) =>
  { heldKeys: [],
    mouse: { x: 0.0, y: 0.0,
             buttons: { left: leftDown, right: false, middle: false } } }

expect sampledInput(init, snapshotWith(true)).held.trigger
expect not sampledInput(init, snapshotWith(false)).held.trigger
// Holding the button fires without any edge at all...
expect (
  let holding = sampledInput(init, snapshotWith(true)) in
  let (fired, _fx) = tick(holding, 0.016, 0.016) in
  fired.shots == 1.0
)
// ...but `fireInterval` (0.18 s) still paces it: the very next 16 ms step,
// still held, must NOT fire a second round.
expect (
  let holding = sampledInput(init, snapshotWith(true)) in
  let (fired, _fx) = tick(holding, 0.016, 0.016) in
  let (soon, _fx2) = tick(sampledInput(fired, snapshotWith(true)), 0.016, 0.016) in
  soon.shots == 1.0
)
// Releasing stops fire even once the cooldown has expired.
expect (
  let holding = sampledInput(init, snapshotWith(true)) in
  let (fired, _fx) = tick(holding, 0.016, 0.016) in
  let (released, _fx2) = tick(sampledInput(fired, snapshotWith(false)), 0.5, 0.5) in
  released.shots == 1.0 && released.cooldown <= 0.0
)

// -------------------------------------------------------------------- update
//
// The raycast answers after this frame's physics step, so the tracer and the
// knockdown it causes are already visible in this frame's `draw`.

// Knock down whichever standing target owns the hit tag, and report the points
// it was worth. `List` has no map-with-accumulator, so this is a `List.fold`
// that rebuilds the list alongside the score. NOTE: written WITHOUT a pipeline
// on purpose — `ts |> List.fold(fn, init, ts)` appends a fourth argument and
// over-applies the builtin, which the typechecker accepts and the runtime
// reports only as "update error: cannot call a record".
let hitOutcome = (h, ts) =>
  let folded =
    List.fold((acc, t) =>
      if standing(t) && targetTag(t.id) == h.tag then
        { list: [{ t with down: true, timer: hitHide }, ..acc.list],
          gained: acc.gained + points(t) }
      else { acc with list: [t, ..acc.list] },
      { list: [], gained: 0.0 }, ts) in
  // Prepend-then-reverse: `[x, ..xs]` is O(1), so the fold stays linear.
  { folded with list: folded.list |> List.reverse }

let update = (model, msg) =>
  match msg with
  | Shot(origin, dir, hit) =>
    let endPoint = if hit.hit then vec(hit.x, hit.y, hit.z)
                   else vadd(origin, vmul(shotRange, dir)) in
    let outcome = hitOutcome(hit, model.targets) in
    let scored = outcome.gained > 0.0 in
    let tracers = model.tracers |> List.append([{ a: origin, b: endPoint, t: 1.0 }]) in
    let sparks = if hit.hit
                 then model.sparks |> List.append([{ p: endPoint, t: 1.0, scored: scored }])
                 else model.sparks in
    ({ model with
         tracers: tracers,
         sparks: sparks,
         targets: outcome.list,
         score: model.score + outcome.gained,
         hits: if scored then model.hits + 1.0 else model.hits,
         hitMark: if scored then 1.0 else model.hitMark },
     // A spatial ding at the plate, so distant hits sound distant.
     if scored then Effect.playAt(Assets.hit, v3(endPoint)) else Effect.none())

// ------------------------------------------------------------------ physics
//
// The world is only here so the hitscan has something to hit: static
// geometry plus one box per STANDING target. A knocked target simply isn't
// declared, which drops its body — so it can't be shot again while it's down.

// Poppers never move, so a `fixed` body is re-declared unchanged and the
// simulation leaves it alone. Runners slide, so they are `kinematic` — the
// game drives their declared position every frame.
// `Physics.box` takes FULL extents. These match the drawn plate (0.78 x 0.90 at
// y 0.60-1.50) plus the head sphere above it (radius 0.30 at y 1.70, so it tops
// out at y 2.00), so a scoring hit always lands on something the player can
// actually see. Deliberately NOT a taller box: a 0.8 x 1.5 one centred at
// y = 1.05 spans y 0.30-1.80 and scores hits on the empty air beside the thin
// post — while a shorter one leaves the visible head unshootable.
let shellBody = (tag, s) =>
  Physics.fixed(tag, Physics.box(s.w, s.h, s.d))
    |> Physics.at(Vec3.make(s.x, s.y, s.z))

let targetBody = (t) =>
  // y 0.575 (just under the plate) to y 2.00 (the top of the head sphere).
  let shape = Physics.box(0.80, 1.425, 0.16) in
  let tag = targetTag(t.id) in
  (match t.kind with
   | Runner => Physics.kinematic(tag, shape)
   | Popper => Physics.fixed(tag, shape))
    |> Physics.at(Vec3.make(targetX(t), 1.2875, t.z))

let physics = (model) =>
  Physics.scene(Vec3.make(0.0, 0.0 - 9.81, 0.0),
    [ shellBody(floorTag, shellFloor),
      shellBody(backstopTag, shellBackstop),
      shellBody(leftWallTag, shellLeft),
      shellBody(rightWallTag, shellRight) ]
      |> List.append(model.targets |> List.filter(standing) |> List.map(targetBody)))

// --------------------------------------------------------------------- draw

let concrete = Color.rgb(0.30, 0.31, 0.34)
let stripe = Color.rgb(0.55, 0.50, 0.20)
let steel = Color.rgb(0.42, 0.45, 0.50)
let plateWhite = Color.rgb(0.86, 0.86, 0.82)
let plateRed = Color.rgb(0.78, 0.16, 0.14)
let gunmetal = Color.rgb(0.30, 0.32, 0.37)

let box = (w, h, d) => Scene.cube() |> Scene.scaleXYZ(w, h, d)

// The drawn counterpart of `shellBody` — same extents, same position.
let shellNode = (s) =>
  box(s.w, s.h, s.d) |> Scene.translate(Vec3.make(s.x, s.y, s.z))

// A steel popper: post, plate, and head, built with its base at y = 0 so
// `rotateX` at the group origin tips it over from the foot.
let targetNode = (t) =>
  let fall = t.anim in
  let plate = if t.id > 5.0 then plateRed else plateWhite in
  let glow = Math.clamp01(1.0 - fall * 3.0) in
  Scene.group([
    box(0.10, 0.55, 0.10) |> Scene.translate(Vec3.make(0.0, 0.275, 0.0))
      |> Scene.lit(steel),
    box(0.78, 0.90, 0.12) |> Scene.translate(Vec3.make(0.0, 1.05, 0.0))
      |> Scene.lit(plate),
    Scene.sphere() |> Scene.scale(0.30) |> Scene.translate(Vec3.make(0.0, 1.70, 0.0))
      |> Scene.lit(plate),
    // A scoring ring on the plate face, toward the shooter (-Z).
    Scene.cylinder() |> Scene.scaleXYZ(0.30, 0.03, 0.30)
      |> Scene.rotateX(Angle.degrees(90.0))
      |> Scene.translate(Vec3.make(0.0, 1.10, 0.0 - 0.08))
      |> Scene.emissive(Color.rgb(0.9 * glow + 0.35, 0.25 * glow + 0.05, 0.15)),
  ])
    |> Scene.rotateX(Angle.radians(1.45 * fall))
    |> Scene.translate(Vec3.make(targetX(t), 0.0, t.z))

// A tracer: a thin cylinder from a to b. The cylinder is Y-aligned, so aim it
// with rotateX(atan2(horizontal, dy)) then rotateY(atan2(dx, dz)).
let tracerNode = (s) =>
  let d = vec(s.b.x - s.a.x, s.b.y - s.a.y, s.b.z - s.a.z) in
  let len = Math.max(0.001, vlen(d)) in
  let horiz = Math.sqrt(d.x * d.x + d.z * d.z) in
  let mid = vadd(s.a, vmul(0.5, d)) in
  let f = Math.clamp01(s.t) in
  Scene.cylinder()
    |> Scene.scaleXYZ(0.022, len, 0.022)
    |> Scene.rotateX(Angle.radians(Math.atan2(horiz, d.y)))
    |> Scene.rotateY(Angle.radians(Math.atan2(d.x, d.z)))
    |> Scene.translate(v3(mid))
    // Scene materials have no alpha, so a tracer fades toward black instead.
    |> Scene.emissive(Color.rgb(0.55 * f, 0.95 * f, 1.0 * f))

let sparkNode = (s) =>
  let f = Math.clamp01(s.t) in
  let r = if s.scored then 0.10 + 0.30 * (1.0 - f) else 0.05 + 0.14 * (1.0 - f) in
  let tint = if s.scored then Color.rgb(2.6 * f, 1.5 * f, 0.4 * f)
             else Color.rgb(1.5 * f, 1.4 * f, 1.2 * f) in
  Scene.sphere() |> Scene.scale(r) |> Scene.translate(v3(s.p)) |> Scene.emissive(tint)

// The weapon viewmodel, in camera-local space, then rotated and translated
// onto the camera by hand. Local -X is screen-right (Ry maps local +X to
// -screenRight), so the gun sits at x = -0.20.
let weaponNode = (m) =>
  let kick = m.recoilPitch in
  let mount = gunOffset(m) in
  Scene.group([
    // receiver
    box(0.105, 0.145, 0.44) |> Scene.translate(Vec3.make(0.0, 0.0, 0.30))
      |> Scene.lit(gunmetal),
    // stock, nearest the camera — gives the silhouette some mass
    box(0.09, 0.13, 0.22) |> Scene.rotateX(Angle.degrees(0.0 - 6.0))
      |> Scene.translate(Vec3.make(0.0, 0.0 - 0.03, 0.0 - 0.02))
      |> Scene.lit(Color.rgb(0.30, 0.21, 0.15)),
    // rear sight
    box(0.05, 0.035, 0.03) |> Scene.translate(Vec3.make(0.0, 0.092, 0.30))
      |> Scene.lit(Color.rgb(0.18, 0.19, 0.22)),
    // barrel
    Scene.cylinder() |> Scene.scaleXYZ(0.032, 0.34, 0.032)
      |> Scene.rotateX(Angle.degrees(90.0))
      |> Scene.translate(Vec3.make(0.0, 0.012, 0.60))
      |> Scene.lit(Color.rgb(0.22, 0.24, 0.28)),
    // grip
    box(0.065, 0.15, 0.085) |> Scene.rotateX(Angle.degrees(0.0 - 14.0))
      |> Scene.translate(Vec3.make(0.0, 0.0 - 0.105, 0.13))
      |> Scene.lit(Color.rgb(0.34, 0.24, 0.18)),
    // magazine
    box(0.052, 0.13, 0.085) |> Scene.translate(Vec3.make(0.0, 0.0 - 0.095, 0.30))
      |> Scene.lit(Color.rgb(0.24, 0.25, 0.28)),
    // front sight post
    box(0.014, 0.055, 0.02) |> Scene.translate(Vec3.make(0.0, 0.085, 0.70))
      |> Scene.lit(steel),
    // a green status lamp on the receiver, dark while reloading
    Scene.sphere() |> Scene.scale(0.016)
      |> Scene.translate(Vec3.make(0.0 - 0.045, 0.055, 0.18))
      |> Scene.emissive(if m.reload > 0.0 then Color.rgb(1.6, 0.5, 0.1)
                        else Color.rgb(0.2, 1.8, 0.6)),
  ])
    // Recoil pushes the gun back and tips the muzzle up; walking bobs it.
    |> Scene.scale(gunScale)
    |> Scene.rotateY(Angle.radians(gunCant))
    |> Scene.rotateX(Angle.radians(0.0 - 0.7 * kick))
    |> Scene.translate(v3(mount))
    |> Scene.rotateX(Angle.radians(0.0 - aimPitch(m)))
    |> Scene.rotateY(Angle.radians(aimYaw(m)))
    |> Scene.translate(v3(m.eye))

// Emissive black is still opaque black, not invisible, so an idle flash has to
// be omitted from the graph rather than faded out.
let muzzleFlashNode = (m) =>
  if m.flash <= 0.0 then Scene.group([]) else
  let f = Math.clamp01(m.flash) in
  Scene.group([
    // Kept small: the flash sits ~0.6 units from the near plane, where a
    // half-metre sphere covers half the screen.
    Scene.sphere() |> Scene.scale(0.010 + 0.032 * f)
      |> Scene.emissive(Color.rgb(1.8 * f, 1.3 * f, 0.5 * f)),
    Scene.cylinder() |> Scene.scaleXYZ(0.007 + 0.014 * f, 0.13 * f, 0.007 + 0.014 * f)
      |> Scene.rotateX(Angle.degrees(90.0))
      |> Scene.translate(Vec3.make(0.0, 0.0, 0.065 * f))
      |> Scene.emissive(Color.rgb(1.6 * f, 1.1 * f, 0.4 * f)),
  ])
    |> Scene.rotateX(Angle.radians(0.0 - aimPitch(m)))
    |> Scene.rotateY(Angle.radians(aimYaw(m)))
    |> Scene.translate(v3(muzzleOf(m)))

// Lane markers every 5 m downrange, so distance reads at a glance.
let laneMarks =
  Scene.group(List.range(9.0) |> List.map((i) =>
    box(rangeHalfWidth * 2.0 - 1.0, 0.02, 0.14)
      |> Scene.translate(Vec3.make(0.0, 0.01, 5.0 + i * 5.0))
      |> Scene.emissive(stripe)))

let laneDividers =
  Scene.group([0.0 - 6.3, 0.0 - 2.1, 2.1, 6.3] |> List.map((x) =>
    box(0.16, 0.22, rangeLength - 10.0)
      |> Scene.translate(Vec3.make(x, 0.11, rangeLength * 0.5 + 2.0))
      |> Scene.lit(Color.rgb(0.24, 0.25, 0.28))))

let rangeNode =
  Scene.group([
    // The floor is drawn as a plane (the collider is the thin `shellFloor` slab
    // just beneath it, so a missed shot terminates instead of running to
    // `shotRange`).
    Scene.plane() |> Scene.scale(rangeLength * 2.0) |> Scene.lit(concrete),
    // backstop and side walls, drawn from the same extents as their colliders
    shellNode(shellBackstop) |> Scene.lit(Color.rgb(0.20, 0.19, 0.18)),
    shellNode(shellLeft) |> Scene.lit(Color.rgb(0.26, 0.27, 0.30)),
    shellNode(shellRight) |> Scene.lit(Color.rgb(0.26, 0.27, 0.30)),
    // No ceiling slab: a big box overhead sits inside the shadow map and puts
    // the whole near half of the range in shadow, which reads as unlit target
    // plates. The strip lights below hang in the dark instead.
    // The shooting stall: two side partitions framing the firing position,
    // with a counter along each — kept out of the centre of the view.
    Scene.group([0.0 - 2.9, 2.9] |> List.map((x) =>
      Scene.group([
        box(0.12, 2.1, 3.4) |> Scene.translate(Vec3.make(x, 1.05, 0.6))
          |> Scene.lit(Color.rgb(0.23, 0.24, 0.27)),
        box(1.1, 0.10, 0.8) |> Scene.translate(Vec3.make(x, 1.02, 2.1))
          |> Scene.lit(Color.rgb(0.30, 0.22, 0.16)),
      ]))),
    laneMarks,
    laneDividers,
    // strip lights in the ceiling
    Scene.group(List.range(5.0) |> List.map((i) =>
      box(1.6, 0.06, 0.5) |> Scene.translate(Vec3.make(0.0, 5.7, 4.0 + i * 9.0))
        |> Scene.emissive(Color.rgb(1.5, 1.5, 1.4)))),
  ])

let lights = (m) =>
  [ Light.ambient(Color.rgb(0.20, 0.21, 0.25)),
    // A steep shadow-casting key light: the weapon viewmodel is ordinary world
    // geometry, so a shallow light throws its shadow across the middle of the
    // screen. Steep keeps that shadow under the player, out of frame.
    Light.directional(Vec3.make(0.16, 0.0 - 0.94, 0.30), Color.rgb(1.0, 0.97, 0.90), 0.95)
      |> Light.castShadows,
    // A non-shadowing fill aimed downrange, so the plate faces (which point
    // back at -Z) stay the brightest thing on the range.
    Light.directional(Vec3.make(0.0, 0.0 - 0.18, 0.98), Color.rgb(0.85, 0.90, 1.0), 0.55),
    Light.point(Vec3.make(0.0, 5.3, 13.0), Color.rgb(0.9, 0.95, 1.0), 1.1, 22.0),
    Light.point(Vec3.make(0.0, 5.3, 31.0), Color.rgb(0.9, 0.95, 1.0), 1.1, 22.0),
    // The muzzle flash lights the room for one frame.
    Light.point(v3(muzzleOf(m)), Color.rgb(1.0, 0.8, 0.45), 1.6 * Math.clamp01(m.flash), 7.0) ]

// The crosshair spreads with recoil and movement; a hitmarker X flashes on a
// scoring hit. Pure 2D, composed over the 3D pass with `Frame.with2D`.
let reticleColor = Color.rgb(0.55, 1.0, 0.95)

let crosshair = (m) =>
  let spread = 6.0 + 260.0 * Math.abs(m.recoilPitch) + 4.0 * Math.clamp01(m.moving) in
  let arm = (dx, dy, w, h) =>
    Sprite.rectangle(reticleColor, w, h) |> Sprite.move(dx, dy) in
  let mark = Math.clamp01(m.hitMark) in
  Sprite.group([
    arm(0.0 - spread - 5.0, 0.0, 10.0, 2.0),
    arm(spread + 5.0, 0.0, 10.0, 2.0),
    arm(0.0, spread + 5.0, 2.0, 10.0),
    arm(0.0, 0.0 - spread - 5.0, 2.0, 10.0),
    Sprite.square(reticleColor, 2.0),
    // hitmarker: two diagonals, fading out
    Sprite.group([
      Sprite.rectangle(Color.rgb(1.0, 0.85, 0.3), 18.0, 2.5) |> Sprite.rotate(Angle.degrees(45.0)),
      Sprite.rectangle(Color.rgb(1.0, 0.85, 0.3), 18.0, 2.5) |> Sprite.rotate(Angle.degrees(0.0 - 45.0)),
    ]) |> Sprite.scale(0.7 + 0.6 * (1.0 - mark)) |> Sprite.fade(mark),
  ])

let draw = (model, tts) =>
  Frame.createLit(
    Camera3D.firstPerson(
      v3(model.eye),
      Angle.radians(aimYaw(model)),
      Angle.radians(aimPitch(model)),
      Angle.degrees(72.0))
      |> Camera3D.clip(0.08, 200.0),
    Scene.group([
      rangeNode,
      Scene.group(model.targets |> List.map(targetNode)),
      Scene.group(model.tracers |> List.map(tracerNode)),
      Scene.group(model.sparks |> List.map(sparkNode)),
      weaponNode(model),
      muzzleFlashNode(model),
    ]),
    lights(model))
    |> Frame.withClearColor(Color.rgb(0.04, 0.05, 0.07))
    |> Frame.withFog(Fog.linear(28.0, 105.0, Color.rgb(0.07, 0.08, 0.11)))
    |> Frame.with2D(Camera2D.create(640.0, 360.0), crosshair(model))

// ----------------------------------------------------------------------- ui

let join = (parts) => parts |> List.fold((acc, s) => Text.concat(acc, s), "")
let f0 = (n) => Text.fixed(n, 0.0)

let accuracy = (m) => if m.shots > 0.0 then 100.0 * m.hits / m.shots else 0.0

let ammoBar = (m) =>
  let filled = Math.floor(m.ammo) in
  join(List.range(magSize) |> List.map((i) => if i < filled then "|" else "."))

let ui = (model) =>
  let low = model.ammo < 5.0 in
  Ui.column([
    Ui.text("functor // SHOOTING RANGE"),
    Ui.textColor(Color.rgb(1.0, 0.85, 0.35), join(["SCORE  ", f0(model.score)])),
    Ui.textColor(if low then Color.rgb(1.0, 0.35, 0.3) else Color.rgb(0.6, 1.0, 0.9),
      join(["AMMO   ", f0(model.ammo), " / ", f0(magSize), "  ", ammoBar(model)])),
    Ui.text(join(["HITS   ", f0(model.hits), " / ", f0(model.shots),
                  "   (", f0(accuracy(model)), "%)"])),
    Ui.text(if model.reload > 0.0 then "-- RELOADING --" else "READY"),
    Ui.text("LMB / SPACE fire   R reload   WASD move   mouse look"),
  ]) |> Ui.panel(Ui.topLeft())
