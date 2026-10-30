// bars.fun — riding with a headset: handlebars, eye calibration and comfort,
// as pure functions.
//
// Each step `sampledInput` hands the XR sample to `read`. It latches each hand
// onto its grip, turns the grip-to-grip vector into steering and the triggers
// into throttle and brake, and keeps the eye calibration. `game.fun` places
// the camera at the calibrated eye, and steps `envelope` (the vignette) and
// `turn` (stepped turning) with the ride.
//
// Points here are in the bike's yaw-only frame (+x left, +y up, +z forward,
// from the emitter-plane centre), the frame the saddle and grips are rigid
// in. XR poses are rig-local (+x right, +y up, -z forward, from the reference
// eye); `toBike` maps one to the other through the calibration.
//
// The expects at the bottom drive these with synthetic samples, no headset.

// ---- Geometry ------------------------------------------------------------

let gripSpan = 0.31 // each grip's distance from the centre line
let gripHeight = 1.0
let gripForward = 0.62
let leftGrip: Input.point3 = { x: gripSpan, y: gripHeight, z: gripForward }
let rightGrip: Input.point3 = { x: -gripSpan, y: gripHeight, z: gripForward }

// Where the calibrated eye sits. Seated, the grips are 0.4 m below it and
// 0.47 m ahead, about 40° down; standing, the eye rises so they are 0.62 m
// below.
let seatedEye: Input.point3 = { x: 0.0, y: 1.4, z: 0.15 }
let standingEye: Input.point3 = { seatedEye with y: 1.62 }
// On foot the frame is the walker's, and the eye is its standing eye.
let footEye = Walk.eye

// ---- Calibration ---------------------------------------------------------

// The rig-local head pose taken as the eye: where it was, and its heading
// (positive = looking left of the rig's forward).
type Calibration = { x: float, y: float, z: float, yaw: float }
let uncalibrated: Calibration = { x: 0.0, y: 0.0, z: 0.0, yaw: 0.0 }

// The heading of an orientation's forward (-z), about +y.
let headingOf = (q: Input.quaternion): float =>
  Math.atan2(2.0 * (q.x * q.z + q.w * q.y), 1.0 - 2.0 * (q.x * q.x + q.y * q.y))

let headOf = (p: Input.pose): Calibration =>
  { x: p.position.x, y: p.position.y, z: p.position.z, yaw: headingOf(p.orientation) }

// A rig-local point in the bike frame: the calibrated head lands on `eye`,
// and its heading on the bike's forward.
let toBike = (cal: Calibration, eye: Input.point3, p: Input.point3): Input.point3 =>
  let c = Math.cos(cal.yaw) in
  let s = Math.sin(cal.yaw) in
  let dx = p.x - cal.x in
  let dz = p.z - cal.z in
  { x: eye.x - (dx * c - dz * s), y: eye.y + p.y - cal.y, z: eye.z - (dx * s + dz * c) }

// The inverse, for building samples: a bike-frame point as rig-local.
let toRig = (cal: Calibration, eye: Input.point3, b: Input.point3): Input.point3 =>
  let c = Math.cos(cal.yaw) in
  let s = Math.sin(cal.yaw) in
  let lx = eye.x - b.x in
  let lz = eye.z - b.z in
  { x: cal.x + lx * c + lz * s, y: cal.y + b.y - eye.y, z: cal.z - lx * s + lz * c }

// The head must hold within these of where it settled, for `calibrateAfter`,
// with the bike at rest, before a calibration is taken.
let stillRadius = 0.02 // m
let stillYaw = 0.05 // rad
let calibrateAfter = 0.5 // s
let restSpeed = 0.1 // m/s

let near = (a: Calibration, b: Calibration) =>
  distance({ x: a.x, y: a.y, z: a.z }, { x: b.x, y: b.y, z: b.z }) < stillRadius
    && Math.abs(Math.atan2(Math.sin(a.yaw - b.yaw), Math.cos(a.yaw - b.yaw))) < stillYaw

// ---- Grips ---------------------------------------------------------------

// Squeeze hysteresis: a hand grabs above `squeezeOn` and lets go below
// `squeezeOff`. It latches only on a fresh squeeze within `latchRadius` of its
// grip, with the hand tracked the step before: holding the squeeze while
// tracking returns doesn't relatch it.
let squeezeOn = 0.7
let squeezeOff = 0.3
let latchRadius = 0.15 // m

type Hand = { tracked: bool, squeezing: bool, latched: bool, at: Input.point3 }
let loose: Hand = { tracked: false, squeezing: false, latched: false, at: { x: 0.0, y: 0.0, z: 0.0 } }

let distance = (a: Input.point3, b: Input.point3) =>
  Math.sqrt((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y) + (a.z - b.z) * (a.z - b.z))

let hand = (cal: Calibration, eye: Input.point3, grip: Input.point3, canLatch: bool, c: Input.controller, h: Hand): Hand =>
  let squeezing = c.squeeze >= (if h.squeezing then squeezeOff else squeezeOn) in
  match (if c.active then c.grip else Option.None) with
  | Option.None => { h with tracked: false, squeezing: squeezing, latched: false }
  | Option.Some(pose) =>
    let at = toBike(cal, eye, pose.position) in
    let grabbed = canLatch && h.tracked && squeezing && not h.squeezing && distance(at, grip) < latchRadius in
    { tracked: true, squeezing: squeezing, latched: canLatch && ((h.latched && squeezing) || grabbed), at: at }

// ---- Steering, throttle, brake -------------------------------------------

// The bars' turn from the grip-to-grip vector, flat: positive when the right
// hand pushes forward (a left turn). No twist: grip orientation is ignored.
let barAngle = (left: Input.point3, right: Input.point3): float =>
  Math.atan2(right.z - left.z, left.x - right.x)

// Steer from the bars' turn past the neutral: nothing inside `deadZone`,
// full lock at `fullLock`.
let deadZone = 0.05 // rad, about 3°
let fullLock = 0.35 // rad, about 20°
let steerOf = (angle: float): float =>
  Math.sign(angle) * Math.clamp01((Math.abs(angle) - deadZone) / (fullLock - deadZone))

// An exponential moving average: each 60 Hz sample moves the steer this
// fraction of the way to its target.
let smoothing = 0.25
let ema = (target: float, v: float) => v |> Math.lerp(target, smoothing)

// A trigger below `triggerRest` is released. Arming waits for the throttle
// to be: re-grabbing with it held doesn't lurch.
let triggerRest = 0.1
// Without both grips: no throttle and this much brake (of `Ride.brakeDecel`),
// stopping the bike gently rather than at full brake.
let gripLossBrake = 0.4

// ---- Comfort -------------------------------------------------------------

// The vignette's strength steps through `strengthLevels` settings (0 = off).
let strengthLevels = 4.0
let defaultStrength = 2.0

// Stepped turning: below `steppedSpeed`, pushing the bars past `snapOn`
// fades out, turns the bike (and the rig with it) by `snapAngle`, and fades
// back in. The bars must come back inside `snapOff` before the next one.
let steppedSpeed = 2.0 // m/s
let snapAngle = 0.5236 // rad, 30°
let snapOn = 0.7
let snapOff = 0.3
let fadeOut = 0.1 // s
let fadeIn = 0.15 // s

// `t` is the time since the snap began, negative when idle.
type Snap = { t: float, angle: float, ready: bool }
let idleSnap: Snap = { t: -1.0, angle: 0.0, ready: true }

type Buttons = { menu: bool, stance: bool, strength: bool, turning: bool, respawn: bool, course: bool }

type Rider = {
  // A headset sample arrived this step.
  present: bool,
  // On the bike: points are in the bike's frame and the hands can latch.
  // On foot they are in the walker's.
  mounted: bool,
  calibration: Calibration,
  standing: bool,
  // A calibration is wanted: one is taken once head and bike are still,
  // and until then the hands can't latch.
  pending: bool,
  // Where the head settled, and for how long.
  anchor: Calibration,
  still: float,
  left: Hand,
  right: Hand,
  // Both grips latched, and the throttle released since: powered.
  armed: bool,
  // The bars' angle when they armed: steering is relative to it.
  neutral: float,
  steer: float,
  strength: float,
  stepped: bool,
  buttons: Buttons,
  // The vignette's level, 0..1, before strength.
  vignette: float,
  snap: Snap,
}

let rider: Rider = {
  present: false,
  mounted: false,
  calibration: uncalibrated,
  standing: false,
  pending: true,
  anchor: uncalibrated,
  still: 0.0,
  left: loose,
  right: loose,
  armed: false,
  neutral: 0.0,
  steer: 0.0,
  strength: defaultStrength,
  stepped: false,
  buttons: { menu: false, stance: false, strength: false, turning: false, respawn: false, course: false },
  vignette: 0.0,
  snap: idleSnap,
}

// The headset went away: drop the hands, power and any snap in flight; keep
// the settings.
let absent = (r: Rider): Rider =>
  { r with present: false, pending: true, still: 0.0, left: loose, right: loose, armed: false, steer: 0.0,
           snap: idleSnap }

// Getting on, at the black of the fade: points move to the bike's frame, and
// the hand whose grab mounted (1 left, -1 right, 0 none) stays latched while
// it squeezes. Power still waits for both grips and a released throttle.
let mountOn = (hand: float, r: Rider): Rider =>
  { r with mounted: true, armed: false, steer: 0.0, snap: idleSnap,
           left: { r.left with latched: hand > 0.0 }, right: { r.right with latched: hand < 0.0 } }

// Getting off: the hands let go.
let dismount = (r: Rider): Rider =>
  { r with mounted: false, armed: false, steer: 0.0, snap: idleSnap,
           left: { r.left with latched: false }, right: { r.right with latched: false } }

let eyeOf = (r: Rider) => if not r.mounted then footEye else if r.standing then standingEye else seatedEye

let sampleDt = 1.0 / 60.0 // `read` runs once per fixed step

// One XR sample: buttons, calibration, grips, then the controls. Left X
// cycles the vignette strength, left Y the turning mode, right A the stance
// (which recalibrates), and the left menu button recalibrates; the last two
// only with the bike at rest (`atRest`), so a stray press never drops the
// grips mid-ride. The settling time counts only while head and bike are both
// still, and restarts on each request. (`game.fun` reads right B and the
// right stick click, respawn and course, from `buttons`.)
let read = (atRest: bool, x: Input.xr, r: Rider): (Rider, Ride.Controls) =>
  let buttons = { menu: x.left.menuPressed, stance: x.right.primaryPressed,
                  strength: x.left.primaryPressed, turning: x.left.secondaryPressed,
                  respawn: x.right.secondaryPressed, course: x.right.thumbstickPressed } in
  let pressed = (now: bool, before: bool) => now && not before in
  let standing = if atRest && pressed(buttons.stance, r.buttons.stance) then not r.standing else r.standing in
  let requested = (atRest && pressed(buttons.menu, r.buttons.menu)) || standing != r.standing in
  let wanted = not r.present || r.pending || requested in
  let (anchor, still) = (match x.head with
    | Option.None => (r.anchor, 0.0)
    | Option.Some(p) =>
      let head = headOf(p) in
      if r.present && atRest && not requested && near(head, r.anchor) then (r.anchor, r.still + sampleDt)
      else (head, 0.0)) in
  // (The tolerance absorbs the float sum of `sampleDt`s.)
  let calibrate = wanted && still >= calibrateAfter - 0.000001 in
  let pending = wanted && not calibrate in
  let calibration = if calibrate then anchor else r.calibration in
  let eye = eyeOf({ r with standing: standing }) in
  let left = hand(calibration, eye, leftGrip, r.mounted && not pending, x.left, r.left) in
  let right = hand(calibration, eye, rightGrip, r.mounted && not pending, x.right, r.right) in
  let armed = left.latched && right.latched && (r.armed || x.right.trigger < triggerRest) in
  let angle = barAngle(left.at, right.at) in
  let neutral = if armed && not r.armed then angle else r.neutral in
  let steer = ema(if armed then steerOf(angle - neutral) else 0.0, r.steer) in
  let controls: Ride.Controls =
    if armed then { throttle: x.right.trigger, brake: x.left.trigger, steer: steer }
    else { throttle: 0.0, brake: Math.max(gripLossBrake, x.left.trigger), steer: steer } in
  ({ r with present: true, calibration: calibration, standing: standing, pending: pending,
            anchor: anchor, still: still, left: left, right: right, armed: armed, neutral: neutral,
            steer: steer,
            strength: if pressed(buttons.strength, r.buttons.strength) then Math.mod(r.strength + 1.0, strengthLevels) else r.strength,
            stepped: if pressed(buttons.turning, r.buttons.turning) then not r.stepped else r.stepped,
            buttons: buttons },
   controls)

// Where the rig-local origin sits in the bike frame: the camera goes there,
// turned by the calibration's heading, so the calibrated head lands on the eye.
let rigOrigin = (r: Rider): Input.point3 => toBike(r.calibration, eyeOf(r), { x: 0.0, y: 0.0, z: 0.0 })

// ---- Vignette ------------------------------------------------------------

// Each motion closes the vignette from its onset to fully at its `full`:
// forward acceleration, yaw rate, lateral acceleration (speed × yaw rate)
// and vertical speed over crests. The level follows the strongest, quickly
// closing (`attack`) and slowly opening (`release`).
let ramp = (onset: float, full: float, v: float) => Math.clamp01((v - onset) / (full - onset))
let discomfort = (accel: float, yawRate: float, speed: float, vy: float): float =>
  Math.max(Math.max(ramp(1.0, 6.0, Math.abs(accel)), ramp(0.15, 0.8, Math.abs(yawRate))),
           Math.max(ramp(1.0, 4.5, speed * Math.abs(yawRate)), ramp(0.3, 1.5, Math.abs(vy))))
let attack = 10.0 // 1/s
let release = 1.5 // 1/s
let envelope = (target: float, dt: float, level: float): float =>
  level |> Ride.approach(target, if target > level then attack else release, dt)

// The drawn vignette: its clear radius (in the comfort layer's units, the
// eye buffer's half-height) closes from `apertureOpen` to `apertureMin` at
// full strength and level, fading in over the first `fadeInBy` of that so it
// doesn't pop. The image is clear inside `vignetteInner` of its half-width
// and opaque past `vignetteOuter` (`generate-vignette.mjs`).
let apertureOpen = 1.2
let apertureMin = 0.55
let fadeInBy = 0.2
let vignetteInner = 0.5
let vignetteOuter = 0.75
let closure = (r: Rider) => r.vignette * r.strength / (strengthLevels - 1.0)
let aperture = (closed: float) => apertureOpen |> Math.lerp(apertureMin, closed)
let opacity = (closed: float) => Math.min(1.0, closed / fadeInBy)
// The comfort layer's extent: a Quest 3 eye buffer's aspect (1680 × 1760).
let layerWidth = 2.0 * 1680.0 / 1760.0
let layerHeight = 2.0

// ---- Stepped turning -----------------------------------------------------

let snapping = (r: Rider, speed: float) => r.present && r.stepped && speed < steppedSpeed

// Whether a snap `t` in has faded fully black. (The tolerance absorbs the
// float sum of the steps, so the turn lands on the black step.)
let black = (t: float) => t >= fadeOut - 0.000001

// One step of a fade `t` in: the next `t` (-1 once faded back in) and
// whether this is the step it went black. Mounting and dismounting
// (`mount.fun`) fade the same way.
let fadeStep = (dt: float, t: float): (float, bool) =>
  let next = t + dt in
  ((if next >= fadeOut + fadeIn then -1.0 else next), not black(t) && black(next))

// One step of a snap: the next state, and the yaw to turn the bike by this
// step (the snap's angle on the step the fade goes black, else 0).
let turn = (active: bool, steer: float, dt: float, s: Snap): (Snap, float) =>
  if s.t >= 0.0 then
    let (t, went) = fadeStep(dt, s.t) in
    ({ s with t: t }, if went then s.angle else 0.0)
  else
    let ready = s.ready || Math.abs(steer) < snapOff in
    if active && ready && Math.abs(steer) > snapOn then ({ t: 0.0, angle: Math.sign(steer) * snapAngle, ready: false }, 0.0)
    else ({ s with ready: ready }, 0.0)

// How black a fade `t` in is, 0..1.
let fadeAt = (t: float): float =>
  if t < 0.0 then 0.0
  else if not black(t) then t / fadeOut
  else Math.clamp01(1.0 - (t - fadeOut) / fadeIn)
let fade = (s: Snap): float => fadeAt(s.t)

// Turn the bike's heading and velocity together by `angle`.
let rotate = (angle: float, s: Ride.State): Ride.State =>
  let c = Math.cos(angle) in
  let n = Math.sin(angle) in
  { s with yaw: s.yaw + angle, vx: s.vx * c + s.vz * n, vz: s.vz * c - s.vx * n }

// ---- Tests ---------------------------------------------------------------

let identity: Input.quaternion = { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
let yawed = (a: float): Input.quaternion => { x: 0.0, y: Math.sin(a / 2.0), z: 0.0, w: Math.cos(a / 2.0) }
let pose = (p: Input.point3): Input.pose => { position: p, orientation: identity }

// A controller holding `squeeze` and `trigger`, its grip at a bike-frame
// point (through the default seated calibration), or untracked.
let controller = (at: Option.t<Input.point3>, squeeze: float, trigger: float): Input.controller =>
  { active: true, grip: at |> Option.map((b) => pose(toRig(uncalibrated, seatedEye, b))), aim: Option.None,
    trigger: trigger, squeeze: squeeze, thumbstick: { x: 0.0, y: 0.0 },
    primaryPressed: false, secondaryPressed: false, thumbstickPressed: false, menuPressed: false }

// Both hands on the bars turned by `angle` (left positive), about their centre.
let barsAt = (angle: float, side: float): Input.point3 =>
  { x: side * gripSpan * Math.cos(angle), y: gripHeight, z: gripForward - side * gripSpan * Math.sin(angle) }
let sample = (angle: float, leftSqueeze: float, rightSqueeze: float, throttle: float): Input.xr =>
  { head: Option.Some(pose({ x: 0.0, y: 0.0, z: 0.0 })),
    left: controller(Option.Some(barsAt(angle, 1.0)), leftSqueeze, 0.0),
    right: controller(Option.Some(barsAt(angle, -1.0)), rightSqueeze, throttle) }
let gripping = (angle: float, throttle: float) => sample(angle, 1.0, 1.0, throttle)
let open = sample(0.0, 0.0, 0.0, 0.0)

// Feed samples in order (the bike at rest), returning the rider and the last controls.
let feed = (samples: List<Input.xr>, r: Rider): (Rider, Ride.Controls) =>
  samples |> List.fold((acc, x) => let (r1, _) = acc in read(true, x, r1), (r, Ride.idle))
let repeat = (n: float, x: Input.xr) => List.range(n) |> List.map((i) => x)
// Settled on the bike: calibrated after holding still, hands open on the grips.
let settled = let (r, _) = feed(repeat(40.0, open), { rider with mounted: true }) in r
let riderOf = (rc: (Rider, Ride.Controls)) => let (r, _) = rc in r
let controlsOf = (rc: (Rider, Ride.Controls)) => let (_, c) = rc in c
// Grab both grips from settled, then hold this sample.
let ride = (n: float, x: Input.xr) => feed([gripping(0.0, 0.0)] |> List.append(repeat(n, x)), settled)

// Calibration: taken after holding still for `calibrateAfter`, not before,
// and not while the bike moves.
expect riderOf(feed(repeat(25.0, open), rider)).pending && not riderOf(feed(repeat(35.0, open), rider)).pending
expect settled.present && not settled.pending
expect (List.range(40.0) |> List.fold((r, i) => riderOf(read(false, open, r)), rider)).pending
// The calibrated head lands on the eye, a point 1 m ahead of it lands 1 m
// ahead of the eye, and `toRig` inverts `toBike`.
expect (
  let cal = { x: 0.1, y: -0.08, z: 0.2, yaw: 0.6 } in
  let ahead = { x: cal.x - Math.sin(cal.yaw), y: cal.y, z: cal.z - Math.cos(cal.yaw) } in
  let p = { x: 0.3, y: -0.4, z: -0.5 } in
  distance(toBike(cal, seatedEye, { x: cal.x, y: cal.y, z: cal.z }), seatedEye) < 0.000001
    && distance(toBike(cal, seatedEye, ahead), { seatedEye with z: seatedEye.z + 1.0 }) < 0.000001
    && distance(toRig(cal, seatedEye, toBike(cal, seatedEye, p)), p) < 0.000001
)
// A head yawed 0.6 rad left reads as 0.6.
expect Math.abs(headingOf(yawed(0.6)) - 0.6) < 0.000001
// Calibrating takes the settled head: moved and turned, then held still.
expect (
  let head = { position: { x: 0.05, y: -0.1, z: 0.12 }, orientation: yawed(0.3) } in
  let r = riderOf(feed(repeat(40.0, { open with head: Option.Some(head) }), { settled with pending: true })) in
  not r.pending && distance({ x: r.calibration.x, y: r.calibration.y, z: r.calibration.z }, head.position) < 0.000001
    && Math.abs(r.calibration.yaw - 0.3) < 0.000001
)
// Stance and recalibration are ignored while the bike moves: the grips stay.
expect (
  let armed = riderOf(ride(1.0, gripping(0.0, 0.0))) in
  let press = { gripping(0.0, 0.0) with right: { gripping(0.0, 0.0).right with primaryPressed: true } } in
  let r = riderOf(read(false, press, armed)) in
  r.armed && not r.standing && not r.pending
)
// A head turned past ±π from where it settled still reads as near it.
expect near({ uncalibrated with yaw: 3.13 }, { uncalibrated with yaw: -3.13 })
// Nor does a moving bike, and a request restarts the settling: a head long
// still doesn't recalibrate the moment the menu is pressed, or the moment
// the bike stops.
expect (
  let menu = { open with left: { open.left with menuPressed: true } } in
  riderOf(feed([menu], settled)).pending && not riderOf(feed([menu] |> List.append(repeat(35.0, open)), settled)).pending
)
expect (
  let moving = List.range(40.0) |> List.fold((r, i) => riderOf(read(false, open, r)), { settled with pending: true }) in
  riderOf(feed([open], moving)).pending && not riderOf(feed(repeat(35.0, open), moving)).pending
)
// A moving head never settles, so the calibration waits.
expect (
  let drifting = List.range(40.0) |> List.map((i) => { open with head: Option.Some(pose({ x: i * 0.015, y: 0.0, z: 0.0 })) }) in
  riderOf(feed(drifting, { settled with pending: true, still: 0.0 })).pending
)
// Standing raises the eye: the same rig-local head lands 0.22 m higher.
expect Math.abs(rigOrigin({ settled with standing: true }).y - rigOrigin(settled).y - 0.22) < 0.000001

// Latching: a fresh squeeze at the grip latches; one far from it doesn't;
// arriving with the squeeze held doesn't; letting go unlatches.
expect (
  let r = riderOf(ride(1.0, gripping(0.0, 0.0))) in
  r.left.latched && r.right.latched && r.armed
)
expect (
  let away = { gripping(0.0, 0.0) with left: controller(Option.Some({ leftGrip with z: gripForward - 0.2 }), 1.0, 0.0) } in
  let r = riderOf(feed([away, away], settled)) in
  not r.left.latched && r.right.latched && not r.armed
)
expect (
  let reaching = { open with left: controller(Option.Some({ leftGrip with z: gripForward - 0.3 }), 1.0, 0.0) } in
  not riderOf(feed([reaching, gripping(0.0, 0.0)], settled)).left.latched
)
expect not riderOf(ride(1.0, sample(0.0, 1.0, 0.0, 0.0))).right.latched
// Nothing latches while a calibration is pending.
expect not riderOf(feed([open, gripping(0.0, 0.0)], { settled with pending: true, still: 0.0 })).left.latched

// Power needs both grips: one hand gives no throttle and the bounded brake,
// whatever the trigger says.
expect (
  let c = controlsOf(ride(1.0, sample(0.0, 1.0, 0.0, 1.0))) in
  c.throttle == 0.0 && c.brake == gripLossBrake
)
expect controlsOf(ride(1.0, gripping(0.0, 0.8))).throttle == 0.8
// Grabbing with the throttle held doesn't arm until it is released.
expect (
  let r = riderOf(feed([gripping(0.0, 1.0), gripping(0.0, 1.0)], settled)) in
  r.left.latched && r.right.latched && not r.armed
    && riderOf(feed([gripping(0.0, 0.0)], r)).armed
)

// Grip loss: releasing either grip, or losing its tracking, cuts the
// throttle, brakes gently, and only a fresh grab (then a released throttle)
// powers it again.
let armedAt = riderOf(ride(1.0, gripping(0.0, 0.0)))
let lost = { gripping(0.0, 1.0) with right: controller(Option.None, 1.0, 1.0) }
expect (
  let c = controlsOf(feed([sample(0.0, 1.0, 0.0, 1.0)], armedAt)) in
  c.throttle == 0.0 && c.brake == gripLossBrake
)
expect (
  let (r, c) = feed([lost], armedAt) in
  not r.right.tracked && not r.armed && c.throttle == 0.0 && c.brake == gripLossBrake
)
expect (
  // Tracking returns with the squeeze still held: still loose.
  let back = riderOf(feed([lost, gripping(0.0, 1.0), gripping(0.0, 1.0)], armedAt)) in
  // A fresh grab latches, and the throttle must come back to rest.
  let regrab = feed([sample(0.0, 1.0, 0.0, 1.0), gripping(0.0, 1.0)], back) in
  let rearmed = feed([gripping(0.0, 0.0), gripping(0.0, 1.0)], riderOf(regrab)) in
  not back.right.latched && not back.armed
    && riderOf(regrab).right.latched && not riderOf(regrab).armed
    && riderOf(rearmed).armed && controlsOf(rearmed).throttle == 1.0
)
// The grip-loss brake is gentle: from 20 m/s it stops, decelerating at
// under half of full brake.
expect (
  let run = Ride.simulate(6.0, Ride.always({ throttle: 0.0, brake: gripLossBrake, steer: 0.0 }), Ride.hoverHeight, Ride.cruising(20.0)) in
  let pairs = run |> List.zip(run |> List.drop(1.0)) in
  Ride.speedOf(Ride.latest(run).s) == 0.0
    && (pairs |> List.all((p) => let (newer, older) = p in
         (older.s.vz - newer.s.vz) / Ride.dt60 < 0.5 * Ride.brakeDecel + Ride.dragLinear * 20.0 + Ride.dragQuadratic * 400.0))
)

// Steering: turning the bars left steers left, right steers right; inside the
// dead zone nothing; past full lock, full. The neutral is where they armed.
let steerAfter = (angle: float) => riderOf(ride(60.0, gripping(angle, 0.0))).steer
expect steerAfter(0.2) > 0.0 && steerAfter(-0.2) < 0.0
expect steerAfter(0.04) == 0.0 && steerAfter(-0.04) == 0.0
expect Math.abs(steerAfter(0.5) - 1.0) < 0.001
expect Math.abs(steerAfter(0.2) - steerOf(0.2)) < 0.001
expect (
  let r = riderOf(feed([gripping(0.15, 0.0), gripping(0.15, 0.0)], settled)) in
  Math.abs(r.neutral - 0.15) < 0.000001 && r.steer == 0.0
)
// The left turn: the right hand forward of the left.
expect barAngle(barsAt(0.2, 1.0), barsAt(0.2, -1.0)) > 0.0 && barsAt(0.2, -1.0).z > barsAt(0.2, 1.0).z
// EMA: n samples toward a fixed target close 1 − (1 − smoothing)ⁿ of the gap.
expect (
  let after = (n: float) => List.range(n) |> List.fold((v, i) => ema(1.0, v), 0.0) in
  after(1.0) == smoothing && Math.abs(after(4.0) - (1.0 - Math.pow(1.0 - smoothing, 4.0))) < 0.000001
)

// Comfort controls: left X cycles the strength through off and back.
expect (
  let press = { open with left: { open.left with primaryPressed: true } } in
  let cycle = feed([press, open, press, open], settled) |> riderOf in
  cycle.strength == Math.mod(defaultStrength + 2.0, strengthLevels)
)
expect riderOf(feed([{ open with left: { open.left with secondaryPressed: true } }], settled)).stepped
// Losing the headset mid-snap drops the snap: the desktop rider isn't turned.
expect absent({ settled with snap: { t: 0.05, angle: snapAngle, ready: false } }).snap == idleSnap

// Vignette envelope: closes fast, opens slowly, and stays within 0..1.
let hold = (target: float, seconds: float, level: float) =>
  List.range(Math.round(seconds * 60.0)) |> List.fold((v, i) => envelope(target, Ride.dt60, v), level)
expect hold(1.0, 0.1, 0.0) > 0.6 && hold(1.0, 0.5, 0.0) > 0.99
expect hold(0.0, 0.5, 1.0) > 0.4 && hold(0.0, 3.0, 1.0) < 0.02
expect (
  let targets = List.range(200.0) |> List.map((i) => Math.mod(i * 0.37, 1.0)) in
  let (_, ok) = targets |> List.fold((acc, t) => let (v, ok) = acc in
    let next = envelope(t, Ride.dt60, v) in (next, ok && next >= 0.0 && next <= 1.0), (0.0, true)) in
  ok
)
// Each motion alone closes it; cruising straight and level doesn't.
expect discomfort(0.0, 0.0, 20.0, 0.0) == 0.0
expect discomfort(12.0, 0.0, 0.0, 0.0) == 1.0 && discomfort(0.0, 1.0, 0.0, 0.0) == 1.0
expect discomfort(0.0, 0.2, 24.0, 0.0) == 1.0 && discomfort(0.0, 0.0, 10.0, -2.0) == 1.0
// The aperture never closes past `apertureMin`, and off draws nothing.
expect aperture(closure({ rider with vignette: 1.0, strength: strengthLevels - 1.0 })) == apertureMin
expect closure({ rider with vignette: 1.0, strength: 0.0 }) == 0.0
// At its tightest the image still covers the layer: its clear radius is the
// aperture, so its half-size is aperture / vignetteInner.
expect apertureMin / vignetteInner > layerHeight / 2.0 && apertureMin / vignetteInner > layerWidth / 2.0

// Stepped turning: held past `snapOn`, one 30° snap at the step the fade is
// black, the fade back to clear, and no second snap until the bars return.
let steady = (n: float, steer: float) => List.range(n) |> List.map((i) => steer)
let snaps = (steers: List<float>) =>
  steers |> List.fold((acc, steer) =>
    let (s, yaw, fades) = acc in
    let (s2, turned) = turn(true, steer, Ride.dt60, s) in
    (s2, yaw + turned, [fade(s2), ..fades]), (idleSnap, 0.0, []))
expect (
  let (s, yaw, fades) = snaps(steady(60.0, 1.0)) in
  Math.abs(yaw - snapAngle) < 0.000001 && List.maximum(fades) == Option.Some(1.0) && fade(s) == 0.0
)
// The turn lands on the step the fade is fully black: one step starts the
// snap, six reach `fadeOut`.
expect (
  let (s, turned) = List.range(7.0) |> List.fold((acc, i) =>
    let (s0, _) = acc in turn(true, 1.0, Ride.dt60, s0), (idleSnap, 0.0)) in
  turned == snapAngle && fade(s) == 1.0
)
expect (
  let pull = steady(30.0, -1.0) in
  let (_, yaw, _) = snaps(pull |> List.append(steady(10.0, 0.0)) |> List.append(pull)) in
  Math.abs(yaw + 2.0 * snapAngle) < 0.000001
)
expect (let (_, yaw, _) = snaps(steady(60.0, 0.5)) in yaw == 0.0)
expect snapping({ settled with stepped: true }, 1.0) && not snapping({ settled with stepped: true }, steppedSpeed)
  && not snapping(settled, 1.0)
// The snap turns heading and velocity together.
expect (
  let s = rotate(snapAngle, { Ride.rest with vz: 2.0 }) in
  Math.abs(s.vx - 2.0 * Math.sin(snapAngle)) < 0.000001 && Math.abs(s.vz - 2.0 * Math.cos(snapAngle)) < 0.000001
    && s.yaw == snapAngle
)

// Getting on and off.
let onFoot = riderOf(feed(repeat(40.0, open), rider))
// On foot nothing latches, even a fresh squeeze at a grip.
expect (
  let r = riderOf(feed([gripping(0.0, 0.0)], onFoot)) in
  not r.mounted && not r.pending && not r.left.latched && not r.right.latched
)
// Mounted by the left hand's grab: it stays latched while it squeezes, the
// right latches on a fresh grab, and power waits for both.
expect (
  let foot = riderOf(feed([sample(0.0, 1.0, 0.0, 0.0)], onFoot)) in
  let (r1, c1) = read(true, sample(0.0, 1.0, 0.0, 0.0), mountOn(1.0, foot)) in
  let r2 = riderOf(read(true, gripping(0.0, 0.0), r1)) in
  r1.mounted && r1.left.latched && not r1.right.latched && not r1.armed && c1.throttle == 0.0 && r2.armed
)
// Getting off lets go.
expect (let r = dismount(riderOf(ride(1.0, gripping(0.0, 0.0)))) in not r.mounted && not r.left.latched && not r.armed)
