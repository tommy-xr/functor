// physics-controller — a platformer character controller where the character
// is a REAL dynamic physics body, driven from `tick`.
//
// The counterpart to `examples/platformer`, which hand-rolls its kinematics as
// a pure function and uses no physics world at all. This one puts a dynamic
// capsule in the `physics` hook and steers it with the SYNCHRONOUS physics
// queries, so collision, walls, and slopes come from the solver instead of
// from collision code in the game.
//
//   functor -d examples/physics-controller run native
//   functor -d examples/physics-controller test          # the pure control logic
//
// Controls: WASD / arrows to run, SPACE to jump, ENTER to respawn.
//
// THE WHOLE CONTROL LOOP IS ONE FRAME, and it all lives in `tick`:
//
//   read   Physics.position / linearVelocity / castExcluding   (synchronous)
//   decide Control.sense / desiredVelocity                     (pure)
//   write  Physics.setVelocityXZ (+ setVelocityY on a jump)     (same-frame)
//
// The reads see the PREVIOUS step (physics runs after `tick`) and the command
// applies at the step that immediately follows this `tick`, so the loop closes
// within one frame. That one step of read latency is inherent to any
// read -> decide -> step simulation, not a Functor restriction.

// Branded body identities: declared once, used as the VALUE at every site.
let playerTag = Physics.tag("player")
let groundTag = Physics.tag("ground")
let ledgeTag = Physics.tag("ledge")
let wallTag = Physics.tag("wall")
let deckTag = Physics.tag("deck")

// The capsule: half-height 0.5 plus radius 0.4, so the feet sit 0.9 below the
// body's center. The ground probe's length follows from exactly that number.
let capsuleHalfHeight = 0.5
let capsuleRadius = 0.4
let feetOffset = capsuleHalfHeight + capsuleRadius
// Skin thickness: how far past the feet the probe looks. Too small and the
// character flickers un-grounded over a seam; too large and it claims to be
// standing while still visibly airborne.
let probeSkin = 0.15

// Flip to `true` to print one line of controller state per frame; the scripted
// verification run in `README.md` reads exactly this. Off by default, because
// a `Debug.log` in `tick` fires ~60 times a second.
//
// `Debug.log` returns its value unchanged, so `trace` is pure with respect to
// the model: the game runs identically with `traceOn` either way.
let traceOn = false
// `row` is a THUNK so the record is only built when tracing is on.
let trace = (row, m) =>
  if traceOn then (let logged = Debug.log("row", row()) in m) else m

// Put the body back at the spawn, at rest. Used by both the fall-out-of-the-
// world path and the ENTER key.
let respawn =
  Effect.batch([
    Physics.teleport(playerTag, Vec3.make(World.spawn.x, World.spawn.y, World.spawn.z)),
    Physics.setVelocity(playerTag, Vec3.make(0.0, 0.0, 0.0)),
  ])

let slabBody = (tag, s: World.Slab) =>
  Physics.fixed(tag, Physics.box(s.w, s.h, s.d))
    |> Physics.at(Vec3.make(s.x, s.y, s.z))
    |> Physics.friction(0.8)

// The world. The deck is KINEMATIC and its declared pose is re-derived from the
// clock every frame: the runtime drives a kinematic body toward each newly
// declared pose, so the deck moves without any command effects, and rapier
// derives the velocity that the controller reads back as `carry`.
//
// Never read a body from inside this hook — it is a pure declaration.
let physics = (model) =>
  let deck = World.deckAt(model.clock) in
  Physics.scene(Vec3.make(0.0, -22.0, 0.0), [
    slabBody(groundTag, World.ground),
    slabBody(ledgeTag, World.ledge),
    slabBody(wallTag, World.wall),
    Physics.kinematic(deckTag, Physics.box(deck.w, deck.h, deck.d))
      |> Physics.at(Vec3.make(deck.x, deck.y, deck.z))
      |> Physics.friction(0.8),
    // Low friction on the character: the controller commands its horizontal
    // velocity outright every frame, so surface friction would only fight it.
    // Platform carry comes from the probe, not from being dragged along.
    Physics.dynamic(playerTag, Physics.capsule(capsuleHalfHeight, capsuleRadius))
      |> Physics.at(Vec3.make(World.spawn.x, World.spawn.y, World.spawn.z))
      |> Physics.friction(0.0)
      |> Physics.restitution(0.0)
      |> Physics.mass(1.0)
      // Non-negotiable for a character. Without it the capsule picks up
      // angular velocity from any glancing contact and topples — and a tipped
      // capsule's lowest point is no longer `feetOffset` below its center, so
      // the grounding probe and the ground clamp both start lying.
      |> Physics.upright,
  ])

let init = {
  ctl: Control.zero,
  // A jump press latched as a rising edge, plus the held state that derives it.
  jumpEdge: false,
  jumpHeld: false,
  // ENTER's own rising-edge latch (key repeats arrive as `isDown = true`), and
  // the respawn request it raises for `tick` to perform.
  enterHeld: false,
  respawnPending: false,
  clock: 0.0,
  frame: 0.0,
  // Whether the ground probe hit the moving deck, and that surface's velocity —
  // model state purely so the HUD and the headless trace can show them.
  onDeck: false,
  carryX: 0.0,
  falls: 0.0,
}

// Held-key steering. There is no `Input.isHeld`, so this is the usual scan.
let held = (k, snap) => snap.heldKeys |> List.any((h) => h == k)

let axis = (neg, pos, snap) =>
  (if held(pos, snap) then 1.0 else 0.0) - (if held(neg, snap) then 1.0 else 0.0)

// `sampledInput` runs once per fixed simulation step, which is what held-key
// movement needs (the `input` hook's repeats would be frame-rate dependent).
// The jump is latched as a rising EDGE here and consumed by `tick`, so the
// press survives to the frame that can act on it.
let sampledInput = (model, snapshot: Input.snapshot) =>
  let wishX = axis(Key.A, Key.D, snapshot) + axis(Key.Left, Key.Right, snapshot) in
  let wishZ = axis(Key.W, Key.S, snapshot) + axis(Key.Up, Key.Down, snapshot) in
  let (nx, nz) = Control.normalizeWish(wishX, wishZ) in
  let jumping = held(Key.Space, snapshot) in
  { model with
    ctl: { model.ctl with wishX: nx, wishZ: nz },
    // Rising edge only: holding SPACE must not re-arm the buffer every frame.
    // The latch is sticky until `tick` consumes it, so a press cannot be lost
    // between a sampled step and the tick that acts on it.
    jumpEdge: model.jumpEdge || (jumping && not model.jumpHeld),
    jumpHeld: jumping,
  }

let zeroVelocity: Physics.velocity = { x: 0.0, y: 0.0, z: 0.0 }

// The grounding probe: straight down from the capsule's center, past the feet
// by `probeSkin`, ignoring the character's own collider. Without the exclusion
// the ray would hit the capsule it starts inside at distance 0 and report the
// character standing on itself.
let groundProbe = (pos) =>
  Physics.castExcluding(playerTag,
                        Vec3.make(pos.x, pos.y, pos.z),
                        Vec3.make(0.0, -1.0, 0.0),
                        feetOffset + probeSkin)

// The velocity of whatever we are standing on — the whole of moving-platform
// carry, with no platform identity, no "which deck was I on last frame", and
// no per-platform state. A fixed body reads back zero, so static ground needs
// no special case.
let surfaceVelocity = (probe) =>
  if probe.hit then Physics.linearVelocity(probe.tag) else zeroVelocity

let tick = (model, dt, tts) =>
  let m = { model with clock: model.clock + dt, frame: model.frame + 1.0 } in
  // The pre-step physics reads raise on the very first frame: the `physics`
  // hook's declaration has not been reconciled and stepped yet.
  if not m.ctl.started then { m with ctl: { m.ctl with started: true } }
  else
    let pos = Physics.position(playerTag) in
    let vel = Physics.linearVelocity(playerTag) in
    let probe = groundProbe(pos) in
    // The observation handed to the pure controller. It needs no probe
    // distance and no rest height: the controller never writes the vertical
    // axis, so it has no standing height to hold. `vy` is here only so a
    // landing's impact can scale the squash.
    let obs = {
      grounded: probe.hit,
      vx: vel.x, vy: vel.y, vz: vel.z,
    } in
    let carry = surfaceVelocity(probe) in
    let sensed = Control.sense(dt, obs, m.jumpEdge, m.ctl) in
    let want = Control.desiredVelocity(dt, obs, carry, sensed) in
    let ctl = if Control.jumpNow(sensed) then Control.consumeJump(sensed) else sensed in
    let next =
      { m with
        ctl: ctl,
        jumpEdge: false,
        onDeck: probe.hit && probe.tag == deckTag,
        carryX: carry.x,
      } in
    // Respawn — either because the character fell out of the world, or because
    // ENTER asked for one. Both go through `tick`, and both return `respawn`
    // INSTEAD of this frame's velocity command, never alongside it: commands
    // apply in queue order, so a `setVelocity` issued after the respawn's would
    // win and hand the body back its pre-teleport fall speed.
    //
    // The body is put back with a command rather than by re-declaring its
    // spawn: a changed declaration would teleport it, but then every later
    // frame's declaration would be a change back again.
    let fellOut = pos.y < World.voidY in
    if fellOut || m.respawnPending then
      ({ next with
         falls: if fellOut then next.falls + 1.0 else next.falls,
         respawnPending: false,
         ctl: Control.respawned(next.ctl) },
       respawn)
    else
      // Built lazily: under the default `traceOn = false` the record — and the
      // `Math.cos` inside `World.deckVx` — is never constructed, because `if`
      // only evaluates the branch it takes. (The closure itself is still
      // allocated each frame; only its body is skipped.)
      let row = () => {
        f: next.frame,
        x: pos.x, y: pos.y, z: pos.z, vy: vel.y,
        g: obs.grounded, deck: next.onDeck,
        carry: carry.x, deckVx: World.deckVx(m.clock),
        coy: ctl.coyote, sq: ctl.squash, impact: ctl.landImpact,
      } in
      // Steer the horizontal plane and leave the vertical axis to the solver,
      // which owns the ground contact, the landing impulse, and gravity. The
      // ONLY frame that writes vy is the one a jump fires on — and because
      // the two masks are disjoint, the pair applies as one whole-vector
      // write without either clobbering the other.
      (trace(row, next),
       if Control.jumpNow(sensed) then
         Effect.batch([
           Physics.setVelocityXZ(playerTag, want.x, want.z),
           Physics.setVelocityY(playerTag, Control.jumpVelocity(carry)),
         ])
       else Physics.setVelocityXZ(playerTag, want.x, want.z))

let input = (model, key, isDown) =>
  match key with
  | Key.Enter =>
    (match isDown with
     | false => { model with enterHeld: false }
     | true =>
       // Rising edge: GLFW delivers key REPEATS as `isDown = true`, so without
       // the latch holding ENTER would request a respawn every repeat. The key
       // only records the INTENT — `tick` performs it, so the respawn is not
       // immediately undone by that same frame's velocity command.
       (match model.enterHeld with
        | true => model
        | false => { model with enterHeld: true, respawnPending: true }))
  | _ => model

// -- drawing -----------------------------------------------------------------

let slabColor = Color.rgb(0.42, 0.45, 0.52)
let deckColor = Color.rgb(0.85, 0.55, 0.25)
let bodyColor = Color.rgb(0.30, 0.72, 0.95)
let groundColor = Color.rgb(0.30, 0.33, 0.38)

let slabVisual = (color, s: World.Slab) =>
  Scene.cube()
    |> Scene.scaleXYZ(s.w, s.h, s.d)
    |> Scene.lit(color)
    |> Scene.translate(Vec3.make(s.x, s.y, s.z))

// The deck's visual is placed by `Physics.transformed`, not by re-deriving the
// pose: the picture and the collider cannot disagree. It must be a FUNCTION,
// not a top-level value — a top-level initializer evaluates at load, when no
// body has been declared yet, and `Physics.transformed` would raise there.
let deckVisual = () =>
  Scene.cube()
    |> Scene.scaleXYZ(World.deckHome.w, World.deckHome.h, World.deckHome.d)
    |> Scene.lit(deckColor)
    |> Physics.transformed(deckTag)

// The landing squash scales about the FEET, not the center, so the character
// flattens onto the surface instead of sinking through it.
let playerVisual = (model) =>
  let sy = Control.squashScale(model.ctl) in
  let sxz = 1.0 + (1.0 - sy) * 0.7 in
  Scene.cylinder()
    |> Scene.scaleXYZ(capsuleRadius * 2.0 * sxz, feetOffset * 2.0 * sy, capsuleRadius * 2.0 * sxz)
    |> Scene.lit(bodyColor)
    |> Scene.translate(Vec3.make(0.0, -feetOffset * (1.0 - sy), 0.0))
    |> Physics.transformed(playerTag)

// `draw` runs AFTER the step, so this read is this frame's pose: the camera
// never trails the body it follows.
let draw = (model, tts) =>
  let pos = Physics.position(playerTag) in
  Frame.createLit(
    Camera.lookAt(Vec3.make(pos.x + 6.5, pos.y + 4.5, pos.z + 12.0),
                  Vec3.make(pos.x, pos.y + 0.7, pos.z)),
    Scene.group([
      slabVisual(groundColor, World.ground),
      slabVisual(slabColor, World.ledge),
      slabVisual(slabColor, World.wall),
      deckVisual(),
      playerVisual(model),
    ]),
    [
      Light.ambient(Color.rgb(0.28, 0.30, 0.36)),
      // Deliberately oblique, so the capsule's SIDES catch light and the
      // landing squash is legible as a change in silhouette.
      Light.directional(Vec3.make(-0.55, -0.7, -0.45), Color.rgb(1.0, 0.97, 0.92), 1.05)
        |> Light.castShadows,
    ])

let onOff = (b) => if b then "yes" else "no"

let ui = (model) =>
  Ui.column([
    Ui.text(Text.join("", ["grounded  ", onOff(model.ctl.grounded)])),
    Ui.text(Text.join("", ["on deck   ", onOff(model.onDeck)])),
    Ui.text(Text.join("", ["carry vx  ", Text.fixed(model.carryX, 2.0)])),
    Ui.text(Text.join("", ["coyote    ", Text.fixed(model.ctl.coyote, 3.0)])),
    Ui.text(Text.join("", ["squash    ", Text.fixed(model.ctl.squash, 3.0)])),
    Ui.text(Text.join("", ["falls     ", Text.fixed(model.falls, 0.0)])),
  ])
    |> Ui.panel(Ui.topLeft())
