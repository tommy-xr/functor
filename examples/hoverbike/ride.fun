// ride.fun — the hoverbike's controller, as pure functions.
//
// Nothing here touches the physics world. `step` takes the controls, what the
// four hover probes saw, and the bike's velocity, and returns the next
// velocity, yaw and visual lean. `game.fun` reads the probes and hands the
// velocity to `Physics.move`, which collides and slides.
//
// The expects at the bottom drive `step` over an analytic flat ground (a probe
// at height h reads h), so hover, drive and steering are checked against
// closed-form answers with no world, window or GPU.

// Hover: each emitter is a spring/damper on its own downward ray.
let gravity = 9.81
let hoverHeight = 1.2 // m, emitter plane to ground at rest, ridden
// Parked, the springs aim lower so a standing walker's hands meet the grips
// (`Bars.gripHeight` up from the emitter plane): 1.3 m off the ground, chest
// height (`mount.fun` pins it below the shoulder and above the hip).
let idleHeight = 0.3 // m
let probeRange = 3.0 // m; a probe that sees nothing nearer gives no lift
let springRate = 25.0 // m/s² per m of compression: 0.8 Hz
let springDamping = 7.0 // m/s² per m/s: damping ratio 0.7
let maxLift = 2.0 * gravity // per emitter, so at most 1 g net upward

// Drive, along the bike's heading.
let thrust = 12.0 // m/s² at full throttle
let topSpeed = 24.0 // m/s (86 km/h), where thrust meets drag
let dragLinear = 0.3 // 1/s; lets a coasting bike stop
let dragQuadratic = (thrust - dragLinear * topSpeed) / (topSpeed * topSpeed)
let brakeDecel = 14.0 // m/s² at full brake; never reverses

// Steering. Turning left is positive yaw.
let maxYawRate = 1.0 // rad/s, at walking pace
let maxLateralAccel = 5.0 // m/s²: speed × yaw rate never exceeds it
let yawResponse = 5.0 // 1/s: how fast the yaw rate follows the stick
let grip = 4.0 // 1/s: sideways velocity decays at this rate

// Visual lean of the fairing only; the collider and camera stay upright.
let leanPerAccel = 0.06 // rad per m/s² of lateral acceleration
let maxLean = 0.35 // rad
let pitchPerAccel = 0.015 // rad per m/s² of forward acceleration
let maxPitch = 0.12 // rad
let leanResponse = 6.0 // 1/s

// Emitter offsets in the bike's frame (+z forward, +x left), metres.
let emitters = [
  { x: 0.45, z: 0.75 }, { x: -0.45, z: 0.75 },
  { x: 0.45, z: -0.75 }, { x: -0.45, z: -0.75 },
]

type State = { vx: float, vy: float, vz: float, yaw: float, yawRate: float, roll: float, pitch: float }
// Throttle and brake 0..1; steer -1..1, left positive.
type Controls = { throttle: float, brake: float, steer: float }
type Probe = { hit: bool, distance: float }

let rest: State = { vx: 0.0, vy: 0.0, vz: 0.0, yaw: 0.0, yawRate: 0.0, roll: 0.0, pitch: 0.0 }
let idle: Controls = { throttle: 0.0, brake: 0.0, steer: 0.0 }

// One emitter's upward acceleration toward a target `height`. Gravity is fed
// forward, so the bike rests exactly at it; emitters only push, and a miss
// gives nothing.
let lift = (height: float, vy: float, p: Probe): float =>
  if p.hit && p.distance < probeRange then
    Math.clamp(0.0, maxLift, gravity + springRate * (height - p.distance) - springDamping * vy)
  else 0.0

let verticalAccel = (height: float, probes: List<Probe>, vy: float): float =>
  (probes |> List.map((p) => lift(height, vy, p)) |> List.sum) / List.length(probes) - gravity

// Every probe sees ground in range. Riding waits for this: on a cold load a
// heightfield's collider appears only once its heightmap decodes, and until
// then the probes miss and the bike would fall.
let sensed = (probes: List<Probe>) => probes |> List.all((p) => p.hit && p.distance < probeRange)

// The steepest turn allowed at a speed: `maxYawRate`, or less once
// speed × yaw rate would pass `maxLateralAccel`.
let yawLimit = (speed: float): float => Math.min(maxYawRate, maxLateralAccel / Math.max(speed, 0.001))

// Forward speed after one step of thrust, drag and brake.
let drive = (c: Controls, dt: float, v: float): float =>
  let pushed = v + (c.throttle * thrust - dragLinear * v - dragQuadratic * v * Math.abs(v)) * dt in
  Math.sign(pushed) * Math.max(0.0, Math.abs(pushed) - c.brake * brakeDecel * dt)

let approach = (target: float, rate: float, dt: float, v: float): float =>
  v + (target - v) * Math.min(1.0, rate * dt)

let speedOf = (s: State) => Math.sqrt(s.vx * s.vx + s.vz * s.vz)

// `height` is the hover target: `hoverHeight` ridden, `idleHeight` parked.
let step = (c: Controls, height: float, probes: List<Probe>, dt: float, s: State): State =>
  let speed = speedOf(s) in
  // Capped at the most speed this step can reach, so speed × yaw rate stays
  // under `maxLateralAccel` after the step too.
  let limit = yawLimit(speed + c.throttle * thrust * dt) in
  let yawRate = Math.clamp(-limit, limit, s.yawRate |> approach(c.steer * limit, yawResponse, dt)) in
  let yaw = s.yaw + yawRate * dt in
  // Split the velocity along the new heading: forward (sin, cos), left (cos, -sin).
  let fx = Math.sin(yaw) in
  let fz = Math.cos(yaw) in
  let forward = s.vx * fx + s.vz * fz in
  let side = s.vx * fz - s.vz * fx in
  let forward2 = drive(c, dt, forward) in
  let side2 = side * Math.exp(-grip * dt) in
  let accel = if dt > 0.0 then (forward2 - forward) / dt else 0.0 in
  { vx: forward2 * fx + side2 * fz,
    vy: s.vy + verticalAccel(height, probes, s.vy) * dt,
    vz: forward2 * fz - side2 * fx,
    yaw: yaw,
    yawRate: yawRate,
    // Bank into the turn (positive = left side down); the nose lifts under
    // thrust and dips under brake (positive = nose up).
    roll: s.roll |> approach(Math.clamp(-maxLean, maxLean, leanPerAccel * speed * yawRate), leanResponse, dt),
    pitch: s.pitch |> approach(Math.clamp(-maxPitch, maxPitch, pitchPerAccel * accel), leanResponse, dt) }

// ---- Flat-ground oracle -------------------------------------------------

let dt60 = 1.0 / 60.0

// Over flat ground at emitter height h, every probe reads h.
let flatProbes = (h: float): List<Probe> =>
  emitters |> List.map((e) => { hit: h >= 0.0 && h < probeRange, distance: h })

type Sample = { t: float, h: float, x: float, z: float, s: State, ay: float }

// `seconds` of 60 Hz steps from emitter height h0 toward a hover target,
// newest sample first. Positions advance by the new velocity, as
// `Physics.move(v × dt)` does.
let simulateTo = (height: float, seconds: float, controlsAt: (float) => Controls, h0: float, s0: State): List<Sample> =>
  List.range(Math.round(seconds * 60.0)) |> List.fold((acc, i) =>
    let last = acc |> List.head |> Option.defaultValue({ t: 0.0, h: h0, x: 0.0, z: 0.0, s: s0, ay: 0.0 }) in
    let s = step(controlsAt(last.t), height, flatProbes(last.h), dt60, last.s) in
    [{ t: (i + 1.0) * dt60, h: last.h + s.vy * dt60, x: last.x + s.vx * dt60, z: last.z + s.vz * dt60,
       s: s, ay: (s.vy - last.s.vy) / dt60 }, ..acc], [])
// Ridden.
let simulate = (seconds: float, controlsAt: (float) => Controls, h0: float, s0: State): List<Sample> =>
  simulateTo(hoverHeight, seconds, controlsAt, h0, s0)

let latest = (samples: List<Sample>): Sample =>
  samples |> List.head |> Option.defaultValue({ t: 0.0, h: 0.0, x: 0.0, z: 0.0, s: rest, ay: 0.0 })

let maxOf = (f: (Sample) => float, samples: List<Sample>): float =>
  samples |> List.map(f) |> List.maximum |> Option.defaultValue(0.0)

let between = (t0: float, t1: float, samples: List<Sample>) =>
  samples |> List.filter((x) => x.t >= t0 && x.t < t1)

let always = (c: Controls) => (t: float) => c

// Hover settles within ±5 cm of its target in 3 s from below, from above,
// and from high enough to free-fall, and the error never grows again.
let settlesTo = (height: float, h0: float) =>
  let run = simulateTo(height, 6.0, always(idle), h0, rest) in
  let error = (x: Sample) => Math.abs(x.h - height) in
  maxOf(error, between(3.0, 6.0, run)) < 0.05
    && maxOf(error, between(4.5, 6.0, run)) <= maxOf(error, between(3.0, 4.5, run))
let settles = (h0: float) => settlesTo(hoverHeight, h0)
expect settles(0.3)
expect settles(1.8)
expect settles(2.9)
// Parked: it rests at the idle height, and sinks to it from ride height (or
// from a free fall) without the error growing.
expect verticalAccel(idleHeight, flatProbes(idleHeight), 0.0) == 0.0
expect settlesTo(idleHeight, hoverHeight) && settlesTo(idleHeight, 2.9)
// Sinking from ride height, the springs only stop pushing: the bike falls at
// most 1 g and never touches the ground.
expect (
  let run = simulateTo(idleHeight, 3.0, always(idle), hoverHeight, rest) in
  (run |> List.all((x) => x.h > 0.2 && x.ay >= -gravity - 0.001))
)
// Damping ratio pinned by the overshoot: rising 0.9 m overshoots under 5 cm.
expect maxOf((x) => x.h, simulate(3.0, always(idle), 0.3, rest)) < hoverHeight + 0.05
// A fall from the edge of probe range neither grounds the bike nor slams it:
// the emitters cap the net upward pull at 1 g.
expect (
  let run = simulate(3.0, always(idle), 2.9, rest) in
  (run |> List.all((x) => x.h > 0.5 && x.ay <= gravity + 0.001))
)

// A miss, or a hit beyond range, lifts nothing: the bike falls at 1 g.
expect lift(hoverHeight, 0.0, { hit: false, distance: 0.0 }) == 0.0
expect lift(hoverHeight, 0.0, { hit: true, distance: probeRange + 0.1 }) == 0.0
expect verticalAccel(hoverHeight, flatProbes(probeRange + 1.0), 0.0) == -gravity
// At rest height the lift exactly carries the bike.
expect verticalAccel(hoverHeight, flatProbes(hoverHeight), 0.0) == 0.0

// Throttle against drag: dv/dt = T − a·v − b·v² has roots v₊ = topSpeed and
// v₋ < 0, and (v − v₊)/(v − v₋) decays as exp(−b(v₊ − v₋)t).
let vMinus = -dragLinear / dragQuadratic - topSpeed
let throttleSpeed = (t: float) =>
  let k = Math.exp(-dragQuadratic * (topSpeed - vMinus) * t) in
  // From rest, (0 − v₊)/(0 − v₋) = −v₊/v₋ scaled by k, solved back for v.
  let r = (topSpeed / vMinus) * k in
  (topSpeed - r * vMinus) / (1.0 - r)
// Coasting: dv/dt = −a·v − b·v².
let coastSpeed = (v0: float, t: float) =>
  let e = Math.exp(-dragLinear * t) in
  dragLinear * v0 * e / (dragLinear + dragQuadratic * v0 * (1.0 - e))

let full = { throttle: 1.0, brake: 0.0, steer: 0.0 }
let cruising = (v: float) => { rest with vz: v }

expect Math.abs(speedOf(latest(simulate(3.0, always(full), hoverHeight, rest)).s) - throttleSpeed(3.0)) < 0.02 * throttleSpeed(3.0)
expect Math.abs(speedOf(latest(simulate(30.0, always(full), hoverHeight, rest)).s) - topSpeed) < 0.01 * topSpeed
expect Math.abs(speedOf(latest(simulate(4.0, always(idle), hoverHeight, cruising(20.0))).s) - coastSpeed(20.0, 4.0)) < 0.02 * coastSpeed(20.0, 4.0)
// Braking from 20 m/s stops, never reverses, and stops between the
// brake-only distance v²/2B and the distance with drag at its peak.
expect (
  let run = simulate(3.0, always({ throttle: 0.0, brake: 1.0, steer: 0.0 }), hoverHeight, cruising(20.0)) in
  let stopped = latest(run) in
  let peak = brakeDecel + dragLinear * 20.0 + dragQuadratic * 400.0 in
  stopped.s.vz == 0.0 && (run |> List.all((x) => x.s.vz >= 0.0))
    && stopped.z < 400.0 / (2.0 * brakeDecel) && stopped.z > 400.0 / (2.0 * peak)
)

// Turn radius: a steady turn at total speed v yaws at w = yawLimit(v) and
// traces radius v / w. In the bike's frame the sideways slip settles at
// w·vF/grip, which bleeds w²·vF/grip of forward speed that the throttle
// replaces. Checked once capped by yaw rate, once by lateral acceleration;
// the radius is half the circle's span, within 2%.
let steadyTurn = (v: float) =>
  let w = yawLimit(v) in
  let vF = v / Math.sqrt(1.0 + (w / grip) * (w / grip)) in
  { w: w, vF: vF, throttle: (dragLinear * vF + dragQuadratic * vF * vF + w * w * vF / grip) / thrust }
let turnRadius = (v: float) =>
  let turn = steadyTurn(v) in
  let start = { rest with vz: turn.vF, vx: -turn.w * turn.vF / grip, yawRate: turn.w } in
  let run = simulate(2.0 * Math.pi / turn.w + 1.0, always({ throttle: turn.throttle, brake: 0.0, steer: 1.0 }), hoverHeight, start) in
  maxOf((x) => Math.sqrt(x.x * x.x + x.z * x.z), run) / 2.0
let analyticRadius = (v: float) => v / yawLimit(v)
expect Math.abs(turnRadius(4.0) - analyticRadius(4.0)) < 0.02 * analyticRadius(4.0)
expect Math.abs(turnRadius(15.0) - analyticRadius(15.0)) < 0.02 * analyticRadius(15.0)
let holding = (v: float) => (dragLinear * v + dragQuadratic * v * v) / thrust
// Full left steer turns left (+x is left at yaw 0).
expect latest(simulate(1.0, always({ throttle: holding(10.0), brake: 0.0, steer: 1.0 }), hoverHeight, cruising(10.0))).x > 0.0

// Lateral grip: sideways velocity decays as exp(−grip·t), heading unchanged.
expect (
  let after = latest(simulate(1.0, always(idle), hoverHeight, { rest with vx: 6.0 })).s in
  Math.abs(after.vx - 6.0 * Math.exp(-grip)) < 0.0001 && after.yaw == 0.0
)

// Readiness: any miss, or a hit beyond range, is not ground yet.
expect sensed(flatProbes(hoverHeight))
expect not sensed(flatProbes(probeRange + 0.5))
expect not sensed(flatProbes(hoverHeight) |> List.drop(1.0) |> List.append([{ hit: false, distance: 0.0 }]))

// ---- Inclined-plane oracle ----------------------------------------------

// Ground rising along +z at `grade` (0.05 = 5%); the bike heads up it. Each
// probe reads its own emitter's height over the plane, so the front pair
// reads lower than the back.
type Climb = { t: float, y: float, z: float, s: State }
let climb = (seconds: float, grade: float, v: float) =>
  List.range(Math.round(seconds * 60.0)) |> List.fold((acc, i) =>
    let c = acc |> List.head |> Option.defaultValue({ t: 0.0, y: hoverHeight, z: 0.0, s: cruising(v) }) in
    let probes = emitters |> List.map((e) => { hit: true, distance: c.y - grade * (c.z + e.z) }) in
    let s = step({ throttle: holding(v), brake: 0.0, steer: 0.0 }, hoverHeight, probes, dt60, c.s) in
    [{ t: (i + 1.0) * dt60, y: c.y + s.vy * dt60, z: c.z + s.vz * dt60, s: s }, ..acc], [])

// Holding speed v up a steady grade, the bike rises at v × grade and its
// clearance settles where the spring carries the damper's share of that
// climb: hoverHeight − springDamping × v × grade / springRate. Downhill it
// rides high by the same amount. Checked within 1 cm after 4 s.
let slopeClearance = (grade: float, v: float) => hoverHeight - springDamping * v * grade / springRate
let followsSlope = (grade: float, v: float) =>
  let settled = climb(6.0, grade, v) |> List.filter((c) => c.t >= 4.0) in
  settled |> List.all((c) =>
    Math.abs((c.y - grade * c.z) - slopeClearance(grade, v)) < 0.01
      && Math.abs(c.s.vy - v * grade) < 0.01)
expect followsSlope(0.05, 20.0)
expect followsSlope(-0.05, 20.0)
expect followsSlope(0.08, 24.0)
// The same lag the course is sized for: 8% at top speed rides 0.54 m low.
expect Math.abs(slopeClearance(0.08, topSpeed) - 0.6624) < 0.0001

// Comfort over a ride (throttle, hard left, hard right, brake): yaw rate,
// yaw acceleration, lateral acceleration (speed × yaw rate), lean, and vertical
// acceleration on flat ground all stay bounded.
let route = (t: float) =>
  if t < 4.0 then full
  else if t < 8.0 then { throttle: 1.0, brake: 0.0, steer: 1.0 }
  else if t < 12.0 then { throttle: 0.6, brake: 0.0, steer: -1.0 }
  else { throttle: 0.0, brake: 1.0, steer: 0.0 }
// Comfort, checked every step of the ride.
let comfortable = (older: Sample, newer: Sample) =>
  Math.abs(newer.s.yawRate) <= maxYawRate
    && Math.abs(newer.s.yawRate - older.s.yawRate) / dt60 <= 2.0 * maxYawRate * yawResponse
    && speedOf(newer.s) * Math.abs(newer.s.yawRate) <= maxLateralAccel + 0.000001
    && Math.abs(newer.s.roll) <= maxLean && Math.abs(newer.s.pitch) <= maxPitch
    && Math.abs(newer.ay) < 0.05
expect (
  let ride = simulate(15.0, route, hoverHeight, rest) in
  let pairs = ride |> List.zip(ride |> List.drop(1.0)) in
  (pairs |> List.all((p) => let (newer, older) = p in comfortable(older, newer)))
    && speedOf(latest(ride).s) < 0.01
)
