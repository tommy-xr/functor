// control.fun — the character controller's decision logic, as pure functions.
//
// Nothing in this file touches the engine prelude: every function takes what
// the world was OBSERVED to be (`obs`) and returns what to do about it. That
// keeps the whole feel layer — coyote time, jump buffering, acceleration,
// landing response — covered by `expect` tests with no GPU, no window, and no
// rapier world. See the expects at the bottom of the file.
//
// `game.fun` owns the two impure lines: reading the observation out of the
// physics world, and commanding the resulting velocity back into it.

// Tuning. All of the platformer's "feel" is here.
let speed = 6.0
let jumpSpeed = 7.2
let coyoteTime = 0.12
let bufferTime = 0.12
let squashTime = 0.18
let groundAccel = 45.0
let airAccel = 14.0
// How long after a jump grounding is ignored. For the first frames of a jump
// the feet are still within the probe's reach, so without this the character
// reads as grounded, the ground clamp corrects it back down to standing
// height, and the jump is cancelled on the frame after it fires. Measured: it
// rose 0.12 units instead of 1.2, with vy flipping +6.83 to -6.37 in one frame.
let jumpLockTime = 0.1
// Downward speed at which a landing reads as maximally hard.
let landHardness = 6.0

// The controller's own state. Everything here is plain data, so it snapshots,
// hot-reloads, and time-travels with the rest of the model.
type Ctl = {
  // Gate for the first frame: the physics reads raise before the `physics`
  // hook's declaration has been reconciled and stepped.
  started: bool,
  // Grounded as of the last observation.
  grounded: bool,
  // Seconds of jump grace remaining after leaving the ground.
  coyote: float,
  // Seconds a buffered jump press stays live.
  buffer: float,
  // Seconds of landing-squash remaining.
  squash: float,
  // Seconds remaining in which grounding is ignored, just after a jump.
  lock: float,
  // Downward speed at the most recent touchdown; scales the squash.
  landImpact: float,
  // Steering wish, in world XZ, from `sampledInput`.
  wishX: float,
  wishZ: float,
}

let zero: Ctl = {
  started: false,
  grounded: false,
  coyote: 0.0,
  buffer: 0.0,
  squash: 0.0,
  lock: 0.0,
  landImpact: 0.0,
  wishX: 0.0,
  wishZ: 0.0,
}

// Clear the transient feel state after a respawn, but KEEP `started` — that is
// the "the physics world has been reconciled" gate, not controller state, and
// clearing it would make the character free-fall one uncommanded frame. The
// steering wish is kept too: it belongs to the keys currently held down.
let respawned = (c) =>
  { c with grounded: false, coyote: 0.0, buffer: 0.0, squash: 0.0, lock: 0.0, landImpact: 0.0 }

let decay = (dt, t) => Math.max(0.0, t - dt)

// Advance the state machine from one observation.
//
//   obs       — { grounded: bool, vx, vy, vz } as read back from the world
//   jumpEdge  — a jump press arrived since the last tick
//
// Note the ordering that makes coyote time work: `grounded` is written from
// this observation, but `landed` is computed against the PREVIOUS one, so a
// touchdown is an edge rather than a level.
let sense = (dt, obs, jumpEdge, c) =>
  let lock = decay(dt, c.lock) in
  // While the post-jump lock is live, grounding is ignored outright: everything
  // downstream — coyote refill, the landing edge, and the ground clamp — reads
  // the character as airborne, which is what it physically is.
  let grounded = obs.grounded && not (lock > 0.0) in
  let landed = grounded && not c.grounded in
  { c with
    lock: lock,
    grounded: grounded,
    // Refilled while grounded, drains in the air: a jump stays legal for a
    // moment after walking off a ledge.
    coyote: if grounded then coyoteTime else decay(dt, c.coyote),
    // A press slightly BEFORE touchdown still fires on landing.
    buffer: if jumpEdge then bufferTime else decay(dt, c.buffer),
    // The landing response starts on the observed touchdown edge.
    squash: if landed then squashTime else decay(dt, c.squash),
    // Clamped at zero: a touchdown observed while still moving UPWARD (a rising
    // deck, or a lock expiring under a low ceiling) would otherwise record a
    // negative impact and turn the squash into a stretch.
    landImpact: if landed then Math.max(0.0, -obs.vy) else c.landImpact,
  }

// Both windows must be open for a jump to fire.
let jumpNow = (c) => c.coyote > 0.0 && c.buffer > 0.0

// Close both windows so one press cannot fire twice, and arm the lock that
// keeps the ground clamp off the character while it is leaving the ground.
let consumeJump = (c) => { c with coyote: 0.0, buffer: 0.0, lock: jumpLockTime }

// Move `current` toward `target` at `rate` units/s^2 without overshooting.
let approach = (rate, dt, current, target) =>
  let d = target - current in
  let step = rate * dt in
  if Math.abs(d) < step then target
  else if d > 0.0 then current + step
  else current - step

// The velocity to command this frame.
//
//   carry — the velocity of whatever surface the ground probe hit ({0,0,0} in
//           the air). Steering is relative to the SURFACE, so standing still
//           on a moving deck rides along with it, and a jump inherits the
//           deck's motion instead of stopping dead in mid-air.
// How fast the ground clamp below may correct the standing height.
let maxSnapSpeed = 6.0

// The vertical axis is the subtle part, because `Physics.setVelocity` replaces
// ALL THREE components — there is no horizontal-only command. So the controller
// must say something about vy every frame, and two obvious answers are both
// wrong. Measured, on a capsule that should rest with its center 0.90 above the
// surface:
//
//   echo the read back      -> the read is one step stale, so while grounded it
//                              still carries the downward speed the solver just
//                              cancelled at the contact. Re-commanding it drives
//                              the capsule in: it rested at y = 0.40, visibly
//                              sunk half a unit into the floor.
//   command 0 while grounded -> better, but overwriting vy every frame also
//                              destroys the contact's own pushout impulse, so
//                              each step's gravity increment accumulates as
//                              penetration. It still sank, just slower —
//                              y = 0.98 drifting to 0.40 over 260 frames.
//
// The rule that holds: while grounded, own the standing height EXPLICITLY.
// The ground probe already measured the distance to the surface, so correct
// toward the rest distance with a critically-damped `error / dt`. That is the
// standard character-controller ground clamp, and it also buys stick-to-a-
// descending-deck for free: a surface dropping away pulls the character down
// with it instead of letting it go momentarily ballistic.
//
//   jumping  -> the jump speed, plus the surface's own rise
//   grounded -> the surface's own vy, plus the height correction
//   airborne -> pass the read through and let gravity own it (one step of
//               delay, which integrates correctly)
//
// `obs.standHeight` is the capsule's rest distance from the surface, and
// `obs.groundDistance` is what the probe measured; both come from the caller so
// that this module stays free of engine constants.
// NOTE both this and `desiredVelocity` branch on `c.grounded` — the EFFECTIVE
// grounded state that `sense` already computed, which honours the post-jump
// lock — never on the raw `obs.grounded`.
let verticalVelocity = (dt, obs, carry, c) =>
  if jumpNow(c) then jumpSpeed + carry.y
  else if c.grounded then
    let error = obs.standHeight - obs.groundDistance in
    // `dt` is zero on the bootstrap sub-frame and on every frame under
    // `--fixed-time` (the capture/golden path), where `error / dt` would be
    // NaN or infinite. No time passing means no correction is owed.
    let correction =
      if dt > 0.0 then Math.max(-maxSnapSpeed, Math.min(maxSnapSpeed, error / dt))
      else 0.0 in
    carry.y + correction
  else obs.vy

let desiredVelocity = (dt, obs, carry, c) =>
  let rate = if c.grounded then groundAccel else airAccel in
  let targetX = carry.x + c.wishX * speed in
  let targetZ = carry.z + c.wishZ * speed in
  {
    x: approach(rate, dt, obs.vx, targetX),
    y: verticalVelocity(dt, obs, carry, c),
    z: approach(rate, dt, obs.vz, targetZ),
  }

// Landing squash as a vertical scale factor: 1.0 at rest, dipping right after
// a hard landing and easing back as the timer drains.
let squashScale = (c) =>
  let t = c.squash / squashTime in
  // Bounded on BOTH sides: this must never exceed 1.0, or the "squash" becomes
  // a stretch and `game.fun` derives an inverted horizontal scale from it.
  let hardness = Math.max(0.0, Math.min(1.0, c.landImpact / landHardness)) in
  1.0 - 0.35 * hardness * t

// Normalize a steering wish so diagonals are not faster than the axes.
let normalizeWish = (x, z) =>
  let len = Math.sqrt(x * x + z * z) in
  if len > 1.0 then (x / len, z / len) else (x, z)

// ---------------------------------------------------------------------------
// Tests. `functor -d examples/physics-controller test` runs these headlessly.
// ---------------------------------------------------------------------------

// An observation fixture. `standHeight` is the capsule's rest distance from a
// surface and `groundDistance` is what the probe measured; equal means resting
// at exactly the right height, so the ground clamp contributes nothing.
let obsAt = (grounded, vx, vy, dist) =>
  { grounded: grounded, vx: vx, vy: vy, vz: 0.0, standHeight: 0.9, groundDistance: dist }

let air = obsAt(false, 0.0, 0.0, 0.9)
let onGround = obsAt(true, 0.0, 0.0, 0.9)
let still = { x: 0.0, y: 0.0, z: 0.0 }
let dt = 0.0166667

// `desiredVelocity` branches on the EFFECTIVE grounded state, so tests must run
// the observation through `sense` first, exactly as `tick` does.
let step = (obs, jumpEdge, carry, c) =>
  let sensed = sense(dt, obs, jumpEdge, c) in
  (desiredVelocity(dt, obs, carry, sensed),
   if jumpNow(sensed) then consumeJump(sensed) else sensed)

// Grounded refills coyote time to full.
expect sense(dt, onGround, false, zero).coyote == coyoteTime

// Leaving the ground drains it, and after coyoteTime it is spent.
expect (
  let c = sense(dt, onGround, false, zero) in
  let c2 = sense(dt, air, false, c) in
  c2.coyote < coyoteTime && c2.coyote > 0.0
)

// Coyote time is a real grace window: a jump pressed a few frames AFTER
// walking off still fires...
expect (
  let grounded = sense(dt, onGround, false, zero) in
  let f1 = sense(dt, air, false, grounded) in
  let f2 = sense(dt, air, false, f1) in
  let pressed = sense(dt, air, true, f2) in
  jumpNow(pressed)
)

// ...but not once the window has fully drained (0.12s is ~8 frames).
expect (
  let grounded = sense(dt, onGround, false, zero) in
  let drained =
    List.range(12.0) |> List.fold((c, _) => sense(dt, air, false, c), grounded) in
  not (drained.coyote > 0.0) && not jumpNow(sense(dt, air, true, drained))
)

// Jump buffering: a press while still falling fires on the touchdown frame.
expect (
  let pressedInAir = sense(dt, air, true, zero) in
  let land = sense(dt, onGround, false, pressedInAir) in
  jumpNow(land)
)

// Consuming closes both windows, so a held press cannot double-jump.
expect (
  let land = sense(dt, onGround, false, sense(dt, air, true, zero)) in
  not jumpNow(consumeJump(land))
)

// A jump commands the jump speed; a non-jumping frame passes vertical
// velocity through untouched, leaving gravity in charge.
expect (
  let (want, _) = step(onGround, true, still, zero) in
  want.y == jumpSpeed
)
expect (
  let (want, _) = step(obsAt(false, 0.0, -4.25, 0.9), false, still, zero) in
  want.y == -4.25
)

// The sinking regression, pinned. A GROUNDED frame must NOT echo back the stale
// downward speed the read still carries; resting at exactly the right height it
// must command exactly the surface's own vy, which on static ground is 0.
expect (
  let (want, _) = step(obsAt(true, 0.0, -6.7, 0.9), false, still, zero) in
  want.y == 0.0
)

// The ground clamp: sunk below the rest height, it corrects UPWARD; hovering
// within the probe's reach, it pulls DOWN to stick to the surface.
expect (
  let (want, _) = step(obsAt(true, 0.0, 0.0, 0.84), false, still, zero) in
  want.y > 0.0
)
expect (
  let (want, _) = step(obsAt(true, 0.0, 0.0, 0.98), false, still, zero) in
  want.y < 0.0
)

// The correction is bounded, so a deep overlap cannot launch the character.
expect (
  let (want, _) = step(obsAt(true, 0.0, 0.0, 0.0), false, still, zero) in
  want.y == maxSnapSpeed
)

// On a RISING surface, grounded vy follows the surface, so a lift carries its
// passenger up instead of leaving them behind.
expect (
  let (want, _) = step(obsAt(true, 0.0, -6.7, 0.9), false, { x: 0.0, y: 2.0, z: 0.0 }, zero) in
  want.y == 2.0
)

// The post-jump lock, pinned — the regression that made the jump collapse. On
// the frame AFTER a jump fires, the feet are still within the probe's reach, so
// the raw observation still says grounded. The lock must make the controller
// treat that as airborne and pass the rising velocity through, instead of
// clamping it back down to standing height.
expect (
  let (_, afterJump) = step(onGround, true, still, zero) in
  afterJump.lock == jumpLockTime
)
expect (
  let (_, afterJump) = step(onGround, true, still, zero) in
  // Still physically overlapping the ground probe, and rising fast.
  let rising = obsAt(true, 0.0, 6.83, 0.98) in
  let (want, next) = step(rising, false, still, afterJump) in
  not next.grounded && want.y == 6.83
)

// The lock expires, and grounding resumes: a landing after it is a real
// landing, squash and all.
expect (
  let (_, afterJump) = step(onGround, true, still, zero) in
  let expired =
    List.range(8.0) |> List.fold((c, _) => sense(dt, air, false, c), afterJump) in
  not (expired.lock > 0.0) && sense(dt, obsAt(true, 0.0, -6.0, 0.9), false, expired).squash == squashTime
)

// Landing is an EDGE, not a level: the squash fires on the touchdown frame
// and not on the frames that follow it.
expect (
  let hit = obsAt(true, 0.0, -6.0, 0.9) in
  let touchdown = sense(dt, hit, false, zero) in
  let after = sense(dt, onGround, false, touchdown) in
  touchdown.squash == squashTime && after.squash < squashTime
)

// A harder landing squashes more, and the squash eases back to 1.0.
expect (
  let soft = sense(dt, obsAt(true, 0.0, -1.5, 0.9), false, zero) in
  let hard = sense(dt, obsAt(true, 0.0, -6.0, 0.9), false, zero) in
  squashScale(hard) < squashScale(soft) && squashScale(soft) < 1.0
)
expect squashScale(zero) == 1.0

// An UPWARD touchdown must not invert the squash into a stretch. Without the
// clamps this returns > 1.0, and `game.fun` then derives an inverted
// horizontal scale from it. Note the squash timer must be live for this to
// bite, which is why `expect squashScale(zero) == 1.0` alone cannot catch it.
expect (
  let rising = sense(dt, obsAt(true, 0.0, 4.0, 0.9), false, zero) in
  rising.landImpact == 0.0 && squashScale(rising) == 1.0
)

// A zero `dt` (the bootstrap sub-frame, and every frame under `--fixed-time`)
// must not turn the ground clamp into NaN or infinity.
expect (
  let sunk = obsAt(true, 0.0, 0.0, 0.5) in
  let sensed = sense(0.0, sunk, false, zero) in
  desiredVelocity(0.0, sunk, still, sensed).y == 0.0
)

// Platform carry: with no steering wish, the commanded velocity converges on
// the deck's own velocity — that is what riding a moving platform IS.
expect (
  let deck = { x: 3.0, y: 0.0, z: 0.0 } in
  let (want, _) = step(obsAt(true, 3.0, 0.0, 0.9), false, deck, zero) in
  want.x == 3.0
)

// Standing STILL on a deck the character is not yet matching, it accelerates
// toward the deck's velocity rather than away from it.
expect (
  let deck = { x: 3.0, y: 0.0, z: 0.0 } in
  let (want, _) = step(obsAt(true, 0.0, 0.0, 0.9), false, deck, zero) in
  want.x > 0.0 && want.x < 3.0
)

// Steering on a moving deck is relative to the deck, so walking forward on a
// deck moving at 3.0 gives deck speed plus walk speed.
expect (
  let deck = { x: 3.0, y: 0.0, z: 0.0 } in
  let wish = { zero with wishX: 1.0 } in
  let (want, _) = step(obsAt(true, 3.0 + speed, 0.0, 0.9), false, deck, wish) in
  want.x == 3.0 + speed
)

// Jumping off a moving deck inherits its vertical motion (a rising lift
// launches you higher, not out from under itself).
expect (
  let lift = { x: 0.0, y: 2.0, z: 0.0 } in
  let (want, _) = step(onGround, true, lift, zero) in
  want.y == jumpSpeed + 2.0
)

// `approach` converges and never overshoots.
expect approach(45.0, dt, 0.0, 6.0) == 45.0 * dt
expect approach(45.0, dt, 6.0, 6.0) == 6.0
expect approach(45.0, dt, 5.999, 6.0) == 6.0
expect approach(45.0, dt, 0.0, -6.0) == -45.0 * dt

// Diagonal steering is normalized; a single axis is left alone.
expect (
  let (x, z) = normalizeWish(1.0, 1.0) in
  Math.abs(Math.sqrt(x * x + z * z) - 1.0) < 0.0001
)
expect (
  let (x, z) = normalizeWish(1.0, 0.0) in
  x == 1.0 && z == 0.0
)
expect (
  let (x, z) = normalizeWish(0.0, 0.0) in
  x == 0.0 && z == 0.0
)
