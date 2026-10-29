// examples/sniper-scope — a magnified rifle scope on a 200 m range.
//
//   functor -d examples/sniper-scope run native
//   functor -d examples/sniper-scope run vr
//   functor -d examples/sniper-scope test
//
// CONTROLS (desktop)
//   mouse          look
//   RIGHT MOUSE    hold to shoulder the rifle (hip <-> shouldered)
//   WHEEL / Z      zoom preset: 12 deg or 6 deg vertical field of view
//   LEFT MOUSE     fire (shouldered only; the bolt takes `boltTime` to cycle)
//   SPACE          hold breath: steadies the sway for a few seconds
//
// CONTROLS (VR) — the rifle is held in the right hand
//   A              toggle the shoulder (the scope goes live)
//   B / Y          zoom preset
//   TRIGGER        fire; release it before the next shot
//
// HOW THE SCOPE WORKS
//   * The magnified image is a second camera (same eye, narrow FOV) rendered
//     into a 512x512 `RenderTarget` with `Frame.withRenderTarget`; its reticle
//     is a `Frame.with2D` pass INSIDE that target frame, so it stays on the
//     glass. The pass is declared only while shouldered.
//   * The glass is a `Scene.quad` showing the target, recessed at the back of
//     a HOLLOW tube built from mesh data (`Scene.cylinder` has solid caps).
//     Looking down the tube, its walls clip the quad's corners to a circle.
//   * The shot is a hitscan `Physics.raycast` from the muzzle, which sits
//     `sightHeight` below the line of sight and is zeroed at `zeroRange`.
//
// Everything is a pure function of the model and the fixed step, so scripted
// captures are byte-identical; the expects at the bottom pin the rules.

// ---------------------------------------------------------------- tunables

let eyeHeight = 1.5
let sensitivity = 0.0025
let pitchLimit = 1.2
let hipFov = 70.0

// Zoom presets, as the scope camera's vertical FOV in degrees.
let wideFov = 12.0
let narrowFov = 6.0

// Bolt cycle between shots, in seconds.
let boltTime = 1.0
// Seconds a rung plate swings. Shorter than the bolt, so the plate is back
// at rest (matching its fixed collider) before the next shot.
let ringTime = 0.8
// Shoulder blend: seconds from hip to shouldered.
let shoulderTime = 0.25

// Ballistics: a straight hitscan along the bore.
let sightHeight = 0.05
let zeroRange = 100.0
let muzzleZ = 0.9
let shotRange = 600.0

// VR: where the right hand holds the rifle, in rifle space (the scope axis
// through the origin, +Z forward).
let gripLocal = { x: 0.0, y: 0.0 - 0.15, z: 0.22 }
// Trigger hysteresis: a pull past `triggerPull` fires once, and the trigger
// must fall back below `triggerRelease` before the next pull counts.
let triggerPull = 0.6
let triggerRelease = 0.2
// EMA weight on the tracked weapon pose: 0 follows the controller exactly;
// nearer 1 is steadier but lags.
let poseSmoothing = 0.0

// Sway: a figure-eight of two sines (radians of aim), scaled by `steady`.
let swayAmp = 0.002
let swayRate = 0.9
// Breath: seconds of held breath, seconds to refill from empty, and the
// recovery after running out (no holding, extra sway).
let holdTime = 4.0
let refillTime = 2.5
let recoverTime = 1.5
let heldSteady = 0.2
let gaspSteady = 1.6

// Seconds the HUD shows the last shot's result.
let calloutTime = 2.0
// Impact markers kept on the range.
let maxMarks = 8.0

// ------------------------------------------------------------- plain vectors

type V = { x: float, y: float, z: float }

let vec = (x: float, y: float, z: float): V => { x: x, y: y, z: z }
let vadd = (a: V, b: V): V => vec(a.x + b.x, a.y + b.y, a.z + b.z)
let vsub = (a: V, b: V): V => vec(a.x - b.x, a.y - b.y, a.z - b.z)
let vscale = (k: float, a: V): V => vec(k * a.x, k * a.y, k * a.z)
let dot = (a: V, b: V) => a.x * b.x + a.y * b.y + a.z * b.z
let cross = (a: V, b: V): V =>
  vec(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
let unit = (a: V): V => vscale(1.0 / Math.sqrt(dot(a, a)), a)
let v3 = (a: V) => Vec3.make(a.x, a.y, a.z)

// Rotate a camera-local offset (+X screen-left, +Y up, +Z forward) into world
// space: Ry(yaw) . Rx(-pitch) — the basis `Camera3D.firstPerson` uses.
let camRot = (yaw: float, pitch: float, l: V): V =>
  let cp = Math.cos(pitch) in
  let sp = Math.sin(pitch) in
  let y2 = l.y * cp + l.z * sp in
  let z2 = l.z * cp - l.y * sp in
  let cy = Math.cos(yaw) in
  let sy = Math.sin(yaw) in
  vec(l.x * cy + z2 * sy, y2, z2 * cy - l.x * sy)

// The same local axes for a world pose ({ position, forward, up }, as
// `Camera3D.mapTrackedPose` returns): +X left, +Y up, +Z forward.
let poseRot = (p, l: V): V =>
  let f = unit(p.forward) in
  let left = unit(cross(p.up, f)) in
  vadd(vadd(vscale(l.x, left), vscale(l.y, cross(f, left))), vscale(l.z, f))

// ------------------------------------------------------------------- range

// Three 1.2 x 0.6 m steel plates. The near two are offset so the 200 m sight
// line stays clear (+X is the shooter's left).
let plateW = 1.2
let plateH = 0.6
let plateY = 1.0
let plateSpecs = [
  { id: 0.0, x: 22.0, z: 50.0 },
  { id: 1.0, x: 0.0 - 24.0, z: 100.0 },
  { id: 2.0, x: 0.0, z: 200.0 },
]

let plateTag = (id: float) => Physics.tag($"plate-{Text.fixed(id, 0.0)}")
let groundTag = Physics.tag("ground")
let leftBermTag = Physics.tag("berm-left")
let rightBermTag = Physics.tag("berm-right")
let backstopTag = Physics.tag("backstop")

// Earthworks as (extents, centre), shared by the colliders and the drawing.
let backstop = { w: 170.0, h: 8.0, d: 20.0, x: 0.0, y: 4.0, z: 240.0 }
let leftBerm = { w: 14.0, h: 4.0, d: 240.0, x: 76.0, y: 2.0, z: 115.0 }
let rightBerm = { w: 14.0, h: 4.0, d: 240.0, x: 0.0 - 76.0, y: 2.0, z: 115.0 }

// --------------------------------------------------------------------- model

type Msg =
  | Landed(hit: Physics.rayHit)

// What the last round did: rang a plate, flew into the backstop (or the sky),
// or struck scenery short of it.
type Outcome =
  | NoShot
  | Hit(plate: float, range: float)
  | Miss(range: float)
  | Blocked(range: float)

type MousePos =
  | NoMouse
  | MouseAt(x: float, y: float)

// The rifle starts laid on the 200 m plate.
let init = {
  yaw: 0.0,
  pitch: Math.atan2(plateY - eyeHeight, 200.0),
  lastMouse: NoMouse,
  aiming: false,
  shoulder: 0.0,
  zoom: 0.0,
  breathKey: false,
  breath: 1.0,
  recover: 0.0,
  steady: 1.0,
  clock: 0.0,
  triggerEdge: false,
  cooldown: 0.0,
  shots: 0.0,
  hits: 0.0,
  score: 0.0,
  lastShot: NoShot,
  callout: 0.0,
  plates: plateSpecs |> List.map((p) => { id: p.id, x: p.x, z: p.z, ring: 0.0 }),
  marks: [],
  // VR: whether a headset is driving, whether the right hand is tracked, its
  // (smoothed) world pose, the button levels last step, and the trigger latch.
  xr: false,
  tracked: false,
  grip: { position: vec(0.0, 0.0, 0.0), forward: vec(0.0, 0.0, 1.0), up: vec(0.0, 1.0, 0.0) },
  shoulderButton: false,
  bButton: false,
  yButton: false,
  triggerArmed: true,
}

// ------------------------------------------------------------ derived state

let shouldered = (m) => m.shoulder >= 1.0
let zoomFov = (m) => if m.zoom >= 1.0 then narrowFov else wideFov
let holding = (m) => m.breathKey && shouldered(m) && m.breath > 0.0 && m.recover <= 0.0

// The sway offsets, applied only as far as the rifle is shouldered.
let swayYaw = (m) => swayAmp * m.steady * m.shoulder * Math.sin(swayRate * m.clock)
let swayPitch = (m) =>
  0.5 * swayAmp * m.steady * m.shoulder * Math.sin(2.0 * swayRate * m.clock + 0.5)

let aimYaw = (m) => m.yaw + swayYaw(m)
let aimPitch = (m) => m.pitch + swayPitch(m)
let eye = vec(0.0, eyeHeight, 0.0)

// In VR the authored camera is a level rig at the eye; the head and hands
// move within it.
let rig = Camera3D.firstPerson(v3(eye), Angle.radians(0.0), Angle.radians(0.0), Angle.degrees(hipFov))

// The held rifle's frame: the scope axis origin, oriented by the grip.
let rifleOrigin = (m) => vsub(m.grip.position, poseRot(m.grip, gripLocal))
let riflePose = (m) => { position: rifleOrigin(m), forward: m.grip.forward, up: m.grip.up }

// The bore in camera-local space: it starts `sightHeight` below the line of
// sight and crosses it at `zeroRange`. Its height relative to the line of
// sight at distance d is sightHeight * (d / zeroRange - 1): 2.5 cm low at
// 50 m, on at 100 m, 5 cm high at 200 m.
let impactOffset = (d: float) => sightHeight * (d / zeroRange - 1.0)
let boreLen = Math.sqrt(sightHeight * sightHeight + zeroRange * zeroRange)
let boreDirLocal = vec(0.0, sightHeight / boreLen, zeroRange / boreLen)
let muzzleLocal = vec(0.0, impactOffset(muzzleZ), muzzleZ)

// ------------------------------------------------------------------- input

let mouseMove = (m, x, y) =>
  match m.lastMouse with
  | NoMouse => { m with lastMouse: MouseAt(x, y) }
  | MouseAt(lastX, lastY) =>
    // Shouldered, the image is magnified, so slow the look to match.
    let k = if shouldered(m) then sensitivity * zoomFov(m) / 40.0 else sensitivity in
    { m with
        yaw: m.yaw - (x - lastX) * k,
        pitch: Math.clamp(0.0 - pitchLimit, pitchLimit, m.pitch - (y - lastY) * k),
        lastMouse: MouseAt(x, y) }

// Wheel up selects the narrow preset, wheel down the wide one; both clamp.
let mouseWheel = (m, delta) =>
  if delta > 0.0 then { m with zoom: 1.0 }
  else if delta < 0.0 then { m with zoom: 0.0 }
  else m

let has = (k, keys) => keys |> List.any((x) => x == k)

// Desktop: levels and de-duplicated edges for one fixed step: RMB shoulders,
// SPACE holds breath, Z toggles the zoom, and a left-click edge pulls the
// trigger. A headset that drops out counts as losing the hand: the trigger
// must be released before a returning headset can fire.
let desktopInput = (m, s) =>
  { m with
      xr: false,
      tracked: false,
      triggerArmed: false,
      aiming: s.mouse.buttons.right,
      breathKey: has(Key.Space, s.heldKeys),
      zoom: if has(Key.Z, s.pressedKeys) then 1.0 - m.zoom else m.zoom,
      triggerEdge: m.triggerEdge || s.mouse.pressed.left }

// One EMA step from `prev` toward `raw`, `k` being the weight kept. A jump of
// 60 degrees or more in one sample (a re-grab) snaps: blending that far apart
// can collapse forward onto up.
let ema = (k, prev: V, raw: V): V => vadd(vscale(k, prev), vscale(1.0 - k, raw))
let smoothPose = (k, prev, raw) =>
  if dot(prev.forward, raw.forward) <= 0.5 || dot(prev.up, raw.up) <= 0.5 then raw
  else { position: ema(k, prev.position, raw.position),
         forward: ema(k, prev.forward, raw.forward),
         up: ema(k, prev.up, raw.up) }

// VR: the right grip holds the rifle. A toggles the shoulder, B or Y cycles the
// zoom, and each trigger pull fires once. Losing the right hand freezes the
// rifle's pose (it is hidden), unshoulders, and disarms the trigger until it
// is released.
let xrInput = (m, xr) =>
  let shoulderButton = xr.right.primaryPressed in
  let b = xr.right.secondaryPressed in
  let y = xr.left.secondaryPressed in
  let base = { m with
      xr: true,
      breathKey: false,
      shoulderButton: shoulderButton,
      bButton: b,
      yButton: y,
      zoom: if (b && not m.bButton) || (y && not m.yButton) then 1.0 - m.zoom else m.zoom } in
  match (if xr.right.active then xr.right.grip else Option.None) with
  | Option.None => { base with tracked: false, aiming: false, triggerEdge: false, triggerArmed: false }
  | Option.Some(pose) =>
    let raw = Camera3D.mapTrackedPose(rig, pose) in
    let pulled = xr.right.trigger >= triggerPull in
    { base with
        tracked: true,
        grip: if m.tracked then smoothPose(poseSmoothing, m.grip, raw) else raw,
        aiming: if shoulderButton && not m.shoulderButton then not m.aiming else m.aiming,
        triggerEdge: m.triggerEdge || (pulled && m.triggerArmed),
        triggerArmed:
          if pulled then false
          else if xr.right.trigger <= triggerRelease then true
          else m.triggerArmed }

let sampledInput = (m, s) =>
  match s.xr with
  | Option.None => desktopInput(m, s)
  | Option.Some(xr) => xrInput(m, xr)

// -------------------------------------------------------------------- step

// Breath: holding drains the reserve; running dry forces a short recovery.
let stepBreath = (dt, m) =>
  if holding(m) then
    let left = Math.max(0.0, m.breath - dt / holdTime) in
    { m with breath: left, recover: if left <= 0.0 then recoverTime else 0.0 }
  else
    { m with
        breath: Math.min(1.0, m.breath + dt / refillTime),
        recover: Math.max(0.0, m.recover - dt) }

let steadyTarget = (m) =>
  if holding(m) then heldSteady else if m.recover > 0.0 then gaspSteady else 1.0

let stepRifle = (dt, m) =>
  let breathed = stepBreath(dt, m) in
  { breathed with
      shoulder: if m.aiming then Math.min(1.0, m.shoulder + dt / shoulderTime)
                else Math.max(0.0, m.shoulder - dt / shoulderTime),
      steady: m.steady |> Math.lerp(steadyTarget(breathed), Math.min(1.0, 5.0 * dt)),
      clock: m.clock + dt,
      cooldown: Math.max(0.0, m.cooldown - dt),
      callout: Math.max(0.0, m.callout - dt),
      plates: m.plates |> List.map((p) => { p with ring: Math.max(0.0, p.ring - dt / ringTime) }) }

// A shot: spend the bolt, and ask the physics world where the bore line lands
// — from the eye's swaying aim on desktop, from the held rifle in VR.
let fire = (m) =>
  let origin =
    if m.xr then vadd(rifleOrigin(m), poseRot(m.grip, muzzleLocal))
    else vadd(eye, camRot(aimYaw(m), aimPitch(m), muzzleLocal)) in
  let dir =
    if m.xr then poseRot(m.grip, boreDirLocal)
    else camRot(aimYaw(m), aimPitch(m), boreDirLocal) in
  ({ m with shots: m.shots + 1.0, cooldown: boltTime },
   Physics.raycast(v3(origin), v3(dir), shotRange, (h) => Landed(h)))

// A trigger edge is spent by the step that sees it: it fires only when the
// rifle is shouldered and the bolt has cycled, and never queues behind either.
let tick = (model, dt, tts) =>
  let m = { stepRifle(dt, model) with triggerEdge: false } in
  if model.triggerEdge && shouldered(m) && m.cooldown <= 0.0 then fire(m)
  else (m, Effect.none())

// ------------------------------------------------------------------ update

let classify = (plates, h) =>
  if not h.hit then Miss(shotRange)
  else
    match plates |> List.find((p) => plateTag(p.id) == h.tag) with
    | Option.Some(p) => Hit(p.id, h.distance)
    | Option.None => if h.tag == backstopTag then Miss(h.distance) else Blocked(h.distance)

let points = (p) => p.z

let update = (m, msg) =>
  match msg with
  | Landed(h) =>
    let outcome = classify(m.plates, h) in
    let rang = (p) => (match outcome with
      | Hit(id, _r) => id == p.id
      | _ => false) in
    let gained = m.plates |> List.filter(rang) |> List.map(points) |> List.sum in
    let mark = { x: h.x, y: h.y, z: h.z, steel: gained > 0.0 } in
    { m with
        lastShot: outcome,
        callout: calloutTime,
        score: m.score + gained,
        hits: if gained > 0.0 then m.hits + 1.0 else m.hits,
        plates: m.plates |> List.map((p) =>
          if rang(p) then { p with ring: 1.0 } else p),
        marks: if h.hit then [mark, ..m.marks] |> List.take(maxMarks) else m.marks }

// ----------------------------------------------------------------- physics

let boxBody = (tag, s) =>
  Physics.fixed(tag, Physics.box(s.w, s.h, s.d)) |> Physics.at(Vec3.make(s.x, s.y, s.z))

let plateBody = (p) =>
  Physics.fixed(plateTag(p.id), Physics.box(plateW, plateH, 0.05))
    |> Physics.at(Vec3.make(p.x, plateY, p.z))

let rangeBodies =
  [ boxBody(groundTag, { w: 400.0, h: 1.0, d: 600.0, x: 0.0, y: 0.0 - 0.5, z: 250.0 }),
    boxBody(backstopTag, backstop),
    boxBody(leftBermTag, leftBerm),
    boxBody(rightBermTag, rightBerm) ]
    |> List.append(plateSpecs |> List.map(plateBody))

let physics = (m) => Physics.scene(Vec3.make(0.0, 0.0 - 9.81, 0.0), rangeBodies)

// -------------------------------------------------------------- the rifle
//
// Camera-local geometry, in metres from the eye along +Z. Looking down the
// tube from the eye, the quad's corners (radius lensR * sqrt 2 at lensZ) sit
// behind the eyepiece face, so the glass reads as a circle — this holds while
// sqrt(2) * lensR / lensZ <= tubeR / eyeZ.

let lensR = 0.046
let tubeR = 0.062
let eyeZ = 0.155
let lensZ = 0.17
let bellZ = 0.40
let bellR = 0.078
let endZ = 0.50

// Mesh data is Z-up and `Mesh.toScene` maps (x, y, z) to (x, z, -y), so a
// scene point (x, y, z) is stored as (x, -z, y).
let segs = 40.0
let around = List.range(segs) |> List.map((k) => 2.0 * Math.pi * k / segs)
let ringAt = (r, z) => around |> List.concatMap((a) => [r * Math.cos(a), 0.0 - z, r * Math.sin(a)])
let meshOf = (rings, polys) => Mesh.fromPolygons(Bytes.fromF32le(List.flatten(rings)), polys)
// The k-th vertex of ring i, wrapping around.
let vi = (i, k) => i * segs + Math.mod(k, segs)
let quads = (f) => List.range(segs) |> List.map(f)

// A tube wall of radius r from z0 to z1, its faces pointing out or in.
let wall = (r, z0, z1, outward) =>
  meshOf([ringAt(r, z0), ringAt(r, z1)], quads((k) =>
    if outward then [vi(0.0, k), vi(0.0, k + 1.0), vi(1.0, k + 1.0), vi(1.0, k)]
    else [vi(0.0, k), vi(1.0, k), vi(1.0, k + 1.0), vi(0.0, k + 1.0)]))

// A flat ring between radii r0 < r1 at depth z, facing the eye (-Z).
let annulus = (r0, r1, z) =>
  meshOf([ringAt(r1, z), ringAt(r0, z)], quads((k) =>
    [vi(0.0, k), vi(1.0, k), vi(1.0, k + 1.0), vi(0.0, k + 1.0)]))

// A disc at depth z, facing the eye (-Z) or away from it.
let disc = (r, z, towardEye) =>
  meshOf([ringAt(r, z)], [(if towardEye then List.reverse(List.range(segs)) else List.range(segs))])

let gunmetal = Color.rgb(0.07, 0.06, 0.09)
let glass = Color.rgb(0.03, 0.04, 0.07)

// Open at the eyepiece, hollow down to the lens, closed at the objective.
let scopeBody =
  Mesh.combine([
    wall(lensR, eyeZ, lensZ, false),
    wall(tubeR, eyeZ, bellZ, true),
    annulus(tubeR, bellR, bellZ),
    wall(bellR, bellZ, endZ, true),
    disc(bellR, endZ, false),
  ]) |> Mesh.toScene |> Scene.lit(gunmetal)

// The eyepiece face, unlit: it faces the key light head-on, and a lit face
// would wash out to a bright specular disc around the glass.
let eyepiece = annulus(lensR, tubeR, eyeZ) |> Mesh.toScene |> Scene.emissive(Color.rgb(0.05, 0.04, 0.07))

// Dark glass at the bottom of the tube: what the hip view sees.
let darkLens = disc(lensR, lensZ, true) |> Mesh.toScene |> Scene.lit(glass)

// A thin cyan rim just inside the eyepiece.
let rimGlow =
  annulus(lensR, lensR + 0.0015, eyeZ - 0.0005) |> Mesh.toScene
    |> Scene.emissive(Color.rgb(0.10, 0.55, 0.60))

let scopeFeed = RenderTarget.named("scope") |> RenderTarget.sized(512.0, 512.0)

// The live image, a hair in front of the dark glass. A quad's front is +Z, so
// it turns 180 degrees to face the eye — unturned, the image reads mirrored.
let liveLens =
  Scene.quad()
    |> Scene.screen(scopeFeed)
    |> Scene.rotateY(180deg)
    |> Scene.scale(2.0 * lensR * 1.01)
    |> Scene.translate(Vec3.make(0.0, 0.0, lensZ - 0.0015))

let box = (w, h, d) => Scene.cube() |> Scene.scaleXYZ(w, h, d)
let at = (x, y, z, s) => s |> Scene.translate(Vec3.make(x, y, z))

let rifleParts =
  Scene.group([
    scopeBody,
    eyepiece,
    darkLens,
    rimGlow,
    // elevation and windage turrets
    Scene.cylinder() |> Scene.scaleXYZ(0.04, 0.06, 0.04) |> at(0.0, 0.085, 0.24)
      |> Scene.lit(Color.rgb(0.18, 0.17, 0.22)),
    Scene.cylinder() |> Scene.scaleXYZ(0.036, 0.05, 0.036) |> Scene.rotateZ(90deg)
      |> at(0.0 - 0.08, 0.0, 0.24) |> Scene.lit(Color.rgb(0.18, 0.17, 0.22)),
    // rings, receiver, barrel and stock
    box(0.03, 0.05, 0.03) |> at(0.0, 0.0 - 0.06, 0.2) |> Scene.lit(gunmetal),
    box(0.03, 0.05, 0.03) |> at(0.0, 0.0 - 0.06, 0.36) |> Scene.lit(gunmetal),
    box(0.045, 0.06, 0.4) |> at(0.0, 0.0 - 0.13, 0.4) |> Scene.lit(gunmetal),
    Scene.cylinder() |> Scene.scaleXYZ(0.024, 0.7, 0.024) |> Scene.rotateX(90deg)
      |> at(0.0, 0.0 - 0.1, 0.9) |> Scene.lit(gunmetal),
    box(0.05, 0.12, 0.3) |> at(0.0, 0.0 - 0.16, 0.0 - 0.05)
      |> Scene.lit(Color.rgb(0.24, 0.20, 0.28)),
  ])

// Hip carry: low on the right (local -X), canted in toward the eye.
let hip = vec(0.0 - 0.24, 0.0 - 0.2, 0.3)
let hipCant = 0.06

let rifleNode = (m) =>
  let t = 1.0 - Math.smoothstep(0.0, 1.0, m.shoulder) in
  Scene.group(if shouldered(m) then [rifleParts, liveLens] else [rifleParts])
    |> Scene.rotateY(Angle.radians(hipCant * t))
    |> Scene.translate(Vec3.make(hip.x * t, hip.y * t, hip.z * t))
    |> Scene.rotateX(Angle.radians(0.0 - aimPitch(m)))
    |> Scene.rotateY(Angle.radians(aimYaw(m)))
    |> Scene.translate(v3(eye))

// Turn a rifle-space scene to a world pose: roll about +Z, then the pitch and
// yaw `rifleNode` uses.
let posed = (p, origin, scene) =>
  let f = unit(p.forward) in
  let yaw = Math.atan2(f.x, f.z) in
  let pitch = Math.asin(Math.clamp(0.0 - 1.0, 1.0, f.y)) in
  let roll = Math.atan2(0.0 - dot(p.up, camRot(yaw, pitch, vec(1.0, 0.0, 0.0))),
                        dot(p.up, camRot(yaw, pitch, vec(0.0, 1.0, 0.0)))) in
  scene
    |> Scene.rotateZ(Angle.radians(roll))
    |> Scene.rotateX(Angle.radians(0.0 - pitch))
    |> Scene.rotateY(Angle.radians(yaw))
    |> Scene.translate(v3(origin))

// VR: the rifle in the right hand; hidden while that hand is untracked.
let heldRifle = (m) =>
  if m.tracked then
    posed(m.grip, rifleOrigin(m), Scene.group(if shouldered(m) then [rifleParts, liveLens] else [rifleParts]))
  else Scene.group([])

// ------------------------------------------------------------------ world

let sky = Color.rgb(0.12, 0.07, 0.20)
let haze = Fog.linear(120.0, 650.0, Color.rgb(0.34, 0.20, 0.36))
let cyan = Color.rgb(0.35, 0.95, 1.0)
let pink = Color.rgb(1.0, 0.45, 0.75)
let earth = Color.rgb(0.30, 0.22, 0.34)

let earthwork = (s) => box(s.w, s.h, s.d) |> at(s.x, s.y, s.z) |> Scene.lit(earth)

// A distance post: grey pole, glowing cap.
let post = (x, z) =>
  Scene.group([
    box(0.12, 1.2, 0.12) |> at(x, 0.6, z) |> Scene.lit(Color.rgb(0.45, 0.42, 0.5)),
    box(0.3, 0.12, 0.12) |> at(x, 1.25, z) |> Scene.emissive(cyan),
  ])

// Low ridges beyond the backstop, and the setting sun.
let ridge = (x, z, s) =>
  Scene.cube() |> Scene.scale(s) |> Scene.rotateZ(45deg) |> at(x, 0.0, z)
    |> Scene.lit(Color.rgb(0.20, 0.12, 0.27))

let world =
  Scene.group([
    Scene.plane() |> Scene.scale(900.0) |> at(0.0, 0.0, 250.0)
      |> Scene.lit(Color.rgb(0.20, 0.15, 0.25)),
    earthwork(backstop),
    earthwork(leftBerm),
    earthwork(rightBerm),
    // firing line and the lane edges
    box(40.0, 0.01, 0.05) |> at(0.0, 0.005, 3.0) |> Scene.emissive(Color.rgb(0.7, 0.25, 0.5)),
    box(0.1, 0.01, 226.0) |> at(9.0, 0.005, 116.0) |> Scene.emissive(Color.rgb(0.10, 0.30, 0.36)),
    box(0.1, 0.01, 226.0) |> at(0.0 - 9.0, 0.005, 116.0) |> Scene.emissive(Color.rgb(0.10, 0.30, 0.36)),
    Scene.group([50.0, 100.0, 150.0, 200.0] |> List.concatMap((z) => [post(10.0, z), post(0.0 - 10.0, z)])),
    ridge(0.0 - 220.0, 470.0, 110.0),
    ridge(0.0 - 90.0, 500.0, 80.0),
    ridge(40.0, 460.0, 120.0),
    ridge(200.0, 490.0, 95.0),
    Scene.sphere() |> Scene.scale(60.0) |> at(0.0 - 120.0, 60.0, 560.0)
      |> Scene.emissive(Color.rgb(0.95, 0.45, 0.6)),
  ])

// A plate on its frame. The frame is deliberately lopsided — a tall post with
// a cyan cap on the shooter's left, a short plain one on the right — so a
// mirrored scope image is obvious next to the main view. A rung plate swings
// on its hangers and flashes.
let plateNode = (p) =>
  let swing = 0.5 * p.ring * Math.sin((1.0 - p.ring) * 24.0) in
  let face = (s) =>
    if p.ring > 0.8 then s |> Scene.emissive(pink) else s |> Scene.lit(Color.rgb(0.92, 0.9, 0.96)) in
  Scene.group([
    box(0.08, 1.7, 0.08) |> at(0.75, 0.85, 0.0) |> Scene.lit(Color.rgb(0.45, 0.42, 0.5)),
    box(0.25, 0.25, 0.1) |> at(0.75, 1.8, 0.0) |> Scene.emissive(cyan),
    box(0.08, 1.45, 0.08) |> at(0.0 - 0.75, 0.725, 0.0) |> Scene.lit(Color.rgb(0.45, 0.42, 0.5)),
    box(1.58, 0.06, 0.06) |> at(0.0, 1.45, 0.0) |> Scene.lit(Color.rgb(0.45, 0.42, 0.5)),
    Scene.group([
      box(plateW, plateH, 0.05) |> at(0.0, 0.0 - 0.45, 0.0) |> face,
      box(0.02, 0.15, 0.02) |> at(0.4, 0.0 - 0.075, 0.0) |> Scene.lit(gunmetal),
      box(0.02, 0.15, 0.02) |> at(0.0 - 0.4, 0.0 - 0.075, 0.0) |> Scene.lit(gunmetal),
    ]) |> Scene.rotateX(Angle.radians(swing)) |> at(0.0, 1.45, 0.0),
  ]) |> at(p.x, 0.0, p.z)

// Impact markers: big enough to read at 200 m through the scope.
let markNode = (k) =>
  Scene.sphere() |> Scene.scale(if k.steel then 0.09 else 0.25)
    |> at(k.x, k.y, k.z - 0.04)
    |> Scene.emissive(if k.steel then cyan else Color.rgb(0.55, 0.35, 0.45))

let rangeScene = (m) =>
  Scene.group([
    world,
    Scene.group(m.plates |> List.map(plateNode)),
    Scene.group(m.marks |> List.map(markNode)),
  ])

// Dusk: violet ambient, a warm key from behind the shooter onto the plate
// faces. No shadows — the scope pass stays one plain scene pass.
let lights = [
  Light.ambient(Color.rgb(0.32, 0.26, 0.40)),
  Light.directional(Vec3.make(0.3, 0.0 - 0.55, 0.78), Color.rgb(1.0, 0.85, 0.9), 0.85),
]

// ------------------------------------------------------------------- draw

// The reticle, in the 512x512 target's own pixels: a duplex crosshair —
// heavy posts that thin out near the centre — and a pink aim dot.
let ink = Color.rgb(0.03, 0.02, 0.05)
let reticle =
  Sprite.group([
    Sprite.rectangle(ink, 512.0, 1.5),
    Sprite.rectangle(ink, 1.5, 512.0),
    Sprite.rectangle(ink, 180.0, 7.0) |> Sprite.moveX(0.0 - 166.0),
    Sprite.rectangle(ink, 180.0, 7.0) |> Sprite.moveX(166.0),
    Sprite.rectangle(ink, 7.0, 180.0) |> Sprite.moveY(0.0 - 166.0),
    Sprite.rectangle(ink, 7.0, 180.0) |> Sprite.moveY(166.0),
    Sprite.circle(pink, 2.5),
  ])

// The scope camera looks down the rifle's axis: from the eye on desktop, from
// the held rifle (roll included) in VR.
let scopeFrame = (m, scene) =>
  Frame.createLit(
    (if m.xr then Camera3D.fromPose(riflePose(m), Angle.degrees(zoomFov(m)))
     else Camera3D.firstPerson(v3(eye), Angle.radians(aimYaw(m)), Angle.radians(aimPitch(m)),
                               Angle.degrees(zoomFov(m))))
      |> Camera3D.clip(1.0, 700.0),
    scene,
    lights)
    |> Frame.withClearColor(sky)
    |> Frame.withFog(haze)
    |> Frame.with2D(Camera2D.create(512.0, 512.0), reticle)

// HUD, in an 800x600 centre-origin, Y-up space.
let dim = Color.rgb(0.55, 0.52, 0.66)
let panelInk = Color.rgb(0.07, 0.05, 0.11)

let leftText = (c, size, s) => Sprite.text(c, size, s) |> Sprite.moveX(Sprite.measure(size, s).width * 0.5)
let pad4 = (n) =>
  let s = Text.fixed(n, 0.0) in
  Text.concat(if n < 10.0 then "000" else if n < 100.0 then "00" else if n < 1000.0 then "0" else "", s)

let degreeMark = Sprite.group([Sprite.circle(cyan, 3.5), Sprite.circle(panelInk, 2.0)])

let calloutText = (m) =>
  match m.lastShot with
  | Hit(_id, r) => $"RING  {Text.fixed(r, 0.0)} M"
  | Miss(_r) => "MISS"
  | Blocked(r) => $"BLOCKED  {Text.fixed(r, 0.0)} M"
  | NoShot => " "

let hud = (m) =>
  let fov = Text.fixed(zoomFov(m), 0.0) in
  let breathW = 120.0 * m.breath in
  Sprite.group([
    // optic + breath, bottom left
    Sprite.group([
      Sprite.rectangle(panelInk, 230.0, 70.0) |> Sprite.fade(0.85) |> Sprite.move(115.0, 0.0),
      leftText(dim, 9.0, "OPTIC") |> Sprite.move(14.0, 22.0),
      leftText(cyan, 24.0, fov) |> Sprite.move(14.0, 0.0),
      degreeMark |> Sprite.move(20.0 + Sprite.measure(24.0, fov).width, 8.0),
      leftText(dim, 9.0, if shouldered(m) then "FOV" else "STANDBY") |> Sprite.move(14.0, 0.0 - 22.0),
      leftText(dim, 9.0, "BREATH") |> Sprite.move(100.0, 22.0),
      Sprite.rectangle(Color.rgb(0.2, 0.18, 0.26), 120.0, 4.0) |> Sprite.move(160.0, 4.0),
      Sprite.rectangle(if m.recover > 0.0 then pink else cyan, Math.max(0.5, breathW), 4.0)
        |> Sprite.move(100.0 + breathW * 0.5, 4.0),
      leftText(dim, 9.0, if holding(m) then "HOLDING" else if m.recover > 0.0 then "RECOVER" else "READY")
        |> Sprite.move(100.0, 0.0 - 22.0),
    ]) |> Sprite.move(0.0 - 380.0, 0.0 - 245.0),
    // score, top right
    Sprite.group([
      Sprite.rectangle(panelInk, 96.0, 52.0) |> Sprite.fade(0.85),
      leftText(dim, 9.0, "SCORE") |> Sprite.move(0.0 - 38.0, 12.0),
      leftText(pink, 22.0, pad4(m.score)) |> Sprite.move(0.0 - 38.0, 0.0 - 6.0),
    ]) |> Sprite.move(320.0, 250.0),
    // the last shot's callout, fading
    Sprite.text(cyan, 14.0, calloutText(m))
      |> Sprite.fade(Math.clamp01(m.callout))
      |> Sprite.moveY(0.0 - 150.0),
    // a small hip crosshair that fades out as the rifle comes up
    Sprite.group([
      Sprite.rectangle(dim, 10.0, 1.5) |> Sprite.moveX(0.0 - 9.0),
      Sprite.rectangle(dim, 10.0, 1.5) |> Sprite.moveX(9.0),
      Sprite.rectangle(dim, 1.5, 10.0) |> Sprite.moveY(9.0),
      Sprite.rectangle(dim, 1.5, 10.0) |> Sprite.moveY(0.0 - 9.0),
    ]) |> Sprite.fade(1.0 - m.shoulder),
  ])

// The range is built once and shared by the main view and the scope.
let draw = (m, tts) =>
  let range = rangeScene(m) in
  let main =
    Frame.createLit(
      (if m.xr then rig
       else Camera3D.firstPerson(v3(eye), Angle.radians(aimYaw(m)), Angle.radians(aimPitch(m)),
                                 Angle.degrees(hipFov)))
        |> Camera3D.clip(0.05, 700.0),
      Scene.group([range, if m.xr then heldRifle(m) else rifleNode(m)]),
      lights)
      |> Frame.withClearColor(sky)
      |> Frame.withFog(haze) in
  (if shouldered(m) then main |> Frame.withRenderTarget(scopeFeed, scopeFrame(m, range)) else main)
    |> Frame.with2D(Camera2D.create(800.0, 600.0), hud(m))

// Headset: request 120 Hz, and pin foveation off so the lens is never shaded
// coarsely wherever it sits in view. Desktop ignores both.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(120.0))
let settings = (model) => Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.Off))

// ------------------------------------------------------------------ expects
//
// `functor -d examples/sniper-scope test` runs these headlessly.

let snap = (held, pressed, buttons, clicked) =>
  { heldKeys: held,
    pressedKeys: pressed,
    releasedKeys: [],
    mouse: { x: 0.0, y: 0.0, surfaceWidth: 800.0, surfaceHeight: 600.0,
             buttons: buttons, pressed: clicked,
             released: { left: false, right: false, middle: false } },
    xr: Option.None }

let noButtons = { left: false, right: false, middle: false }
let rmb = { left: false, right: true, middle: false }
let lmb = { left: true, right: false, middle: false }
let idle = snap([], [], noButtons, noButtons)
let aimed = snap([], [], rmb, noButtons)
let breathing = snap([Key.Space], [], rmb, noButtons)

// Run n fixed steps under one input sample.
let run = (n, s, m0) =>
  List.fold((m, _i) => let (next, _fx) = tick(sampledInput(m, s), 1.0 / 60.0, 0.0) in next,
            m0, List.range(n))

let up = run(20.0, aimed, init)
let fireOnce = (m) => let (next, _fx) = tick(sampledInput(m, snap([], [], rmb, lmb)), 1.0 / 60.0, 0.0) in next

// Zoom: the wheel picks a preset and clamps at either end; Z toggles.
expect zoomFov(init) == wideFov
expect zoomFov(mouseWheel(init, 1.0)) == narrowFov
expect zoomFov(mouseWheel(mouseWheel(init, 1.0), 1.0)) == narrowFov
expect zoomFov(mouseWheel(mouseWheel(init, 1.0), 0.0 - 1.0)) == wideFov
expect zoomFov(mouseWheel(init, 0.0 - 1.0)) == wideFov
expect zoomFov(sampledInput(init, snap([], [Key.Z], noButtons, noButtons))) == narrowFov
expect zoomFov(sampledInput(sampledInput(init, snap([], [Key.Z], noButtons, noButtons)),
                            snap([], [Key.Z], noButtons, noButtons))) == wideFov

// Shouldering takes `shoulderTime`; the scope only goes live once it is up.
expect not shouldered(run(10.0, aimed, init))
expect shouldered(up)
expect not shouldered(run(20.0, idle, up))

// Fire: a click edge fires once when shouldered, and is spent either way.
expect fireOnce(up).shots == 1.0
expect fireOnce(up).cooldown == boltTime
expect not fireOnce(up).triggerEdge
expect fireOnce(init).shots == 0.0
expect not fireOnce(init).triggerEdge
// Holding the button is not a second edge, and the bolt gates the next click...
expect run(30.0, snap([], [], { left: true, right: true, middle: false }, noButtons), fireOnce(up)).shots == 1.0
expect fireOnce(run(30.0, aimed, fireOnce(up))).shots == 1.0
// ...until it has cycled.
expect fireOnce(run(61.0, aimed, fireOnce(up))).shots == 2.0

// Zeroing: the bore meets the line of sight at zeroRange, below it nearer
// and above it beyond.
expect impactOffset(zeroRange) == 0.0
expect Math.abs(impactOffset(50.0) + 0.025) < 0.000001
expect Math.abs(impactOffset(200.0) - 0.05) < 0.000001
expect (
  let t = (zeroRange - muzzleLocal.z) / boreDirLocal.z in
  Math.abs(muzzleLocal.y + boreDirLocal.y * t) < 0.000001
)

// Sway: bounded by swayAmp * steady, and absent at the hip.
expect List.range(240.0) |> List.all((i) =>
  let m = { up with clock: i * 0.05 } in
  Math.abs(swayYaw(m)) <= swayAmp && Math.abs(swayPitch(m)) <= 0.5 * swayAmp)
expect swayYaw({ init with clock: 1.7 }) == 0.0
// Breath: one second held steadies the aim to near heldSteady...
expect (
  let m = run(60.0, breathing, up) in
  holding(m) && m.steady < 0.3 && Math.abs(m.breath - 0.75) < 0.01
)
// ...holding past the reserve forces a recovery with extra sway, and no hold...
expect (
  let m = run(270.0, breathing, up) in
  m.recover > 0.0 && not holding(m) && m.steady > 1.0
)
// ...which ends after recoverTime, when the breath can be held again.
expect (
  let m = run(100.0, breathing, run(270.0, breathing, up)) in
  m.recover == 0.0 && holding(m)
)

// Hits: a plate rings and scores its distance; the backstop and the sky are
// misses; anything else on the way is an obstruction.
let rayHit = (tag, d) =>
  { hit: true, x: 0.0, y: 1.0, z: d, nx: 0.0, ny: 0.0, nz: 0.0 - 1.0,
    distance: d, tag: tag, triangle: 0.0 - 1.0 }
let landed = (h) => update(up, Landed(h))

expect landed(rayHit(plateTag(2.0), 199.1)).lastShot == Hit(2.0, 199.1)
expect landed(rayHit(plateTag(2.0), 199.1)).score == 200.0
expect landed(rayHit(plateTag(1.0), 99.1)).score == 100.0
expect (landed(rayHit(plateTag(2.0), 199.1)).plates |> List.map((p) => p.ring)) == [0.0, 0.0, 1.0]
expect List.length(landed(rayHit(plateTag(2.0), 199.1)).marks) == 1.0
expect landed(rayHit(backstopTag, 231.0)).lastShot == Miss(231.0)
expect landed(rayHit(backstopTag, 231.0)).score == 0.0
expect landed({ rayHit(groundTag, 0.0) with hit: false }).lastShot == Miss(shotRange)
expect landed({ rayHit(groundTag, 0.0) with hit: false }).marks == []
expect landed(rayHit(groundTag, 31.0)).lastShot == Blocked(31.0)
expect landed(rayHit(rightBermTag, 80.0)).lastShot == Blocked(80.0)
// A rung plate settles within ringTime — before the bolt cycles, so its
// fixed collider never disagrees with a swinging plate you can shoot at.
// The marks are capped at maxMarks.
expect ringTime < boltTime
expect (run(61.0, aimed, landed(rayHit(plateTag(2.0), 199.1))).plates |> List.map((p) => p.ring)) == [0.0, 0.0, 0.0]
expect List.length(List.fold((m, _i) => update(m, Landed(rayHit(groundTag, 30.0))), up, List.range(12.0)).marks) == maxMarks

// VR. A right hand at a fixed rig pose, with the trigger at `trigger` and the
// A / B buttons held or not; the left hand is idle.
let levelGrip = { position: { x: 0.15, y: 0.0 - 0.2, z: 0.0 - 0.3 }, orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 } }
let hand = (grip, trigger, a, b) =>
  { active: true, grip: Option.Some(grip), aim: Option.Some(grip), trigger: trigger, squeeze: 0.0,
    thumbstick: { x: 0.0, y: 0.0 }, primaryPressed: a, secondaryPressed: b,
    thumbstickPressed: false, menuPressed: false }
let lostHand = { hand(levelGrip, 0.0, false, false) with active: false, grip: Option.None, aim: Option.None }
let vr = (right) => { idle with xr: Option.Some({ head: Option.None, left: lostHand, right: right }) }
let rest = vr(hand(levelGrip, 0.0, false, false))
let pressA = vr(hand(levelGrip, 0.0, true, false))
let pressB = vr(hand(levelGrip, 0.0, false, true))
let pull = vr(hand(levelGrip, 1.0, false, false))
let lost = vr(lostHand)

let vrUp = run(20.0, rest, run(1.0, pressA, init))

// A headset sample switches to VR; the grip is the rig-mapped pose.
expect sampledInput(init, rest).xr && sampledInput(init, rest).tracked
expect sampledInput(init, rest).grip == Camera3D.mapTrackedPose(rig, levelGrip)
expect not sampledInput(sampledInput(init, rest), idle).xr
// Shoulder toggle: A toggles once per press, however long it is held.
expect shouldered(vrUp)
expect run(5.0, pressA, init).aiming
expect not run(1.0, pressA, run(1.0, rest, run(1.0, pressA, init))).aiming
expect not shouldered(run(20.0, rest, run(1.0, pressA, vrUp)))
// B (or Y) cycles the zoom once per press; holding one doesn't mask the other.
expect zoomFov(run(3.0, pressB, init)) == narrowFov
expect zoomFov(run(1.0, pressB, run(1.0, rest, run(1.0, pressB, init)))) == wideFov
expect (
  let pressY = { pressB with xr: Option.Some({ head: Option.None, left: hand(levelGrip, 0.0, false, true),
                                               right: hand(levelGrip, 0.0, false, true) }) } in
  zoomFov(run(1.0, pressY, run(1.0, pressB, init))) == wideFov
)
// Trigger: a pull fires once; holding it past the bolt does not fire again...
expect run(1.0, pull, vrUp).shots == 1.0
expect run(70.0, pull, vrUp).shots == 1.0
// ...a partial release (above triggerRelease) does not re-arm...
expect run(1.0, pull, run(1.0, vr(hand(levelGrip, 0.4, false, false)), run(70.0, pull, vrUp))).shots == 1.0
// ...a full release does.
expect run(1.0, pull, run(1.0, rest, run(70.0, pull, vrUp))).shots == 2.0
// Not shouldered, a pull is spent without firing.
expect run(1.0, pull, run(1.0, rest, init)).shots == 0.0
// Tracking loss: the pose freezes, the rifle unshoulders, and a pending pull
// is dropped...
expect (
  let m = sampledInput({ vrUp with triggerEdge: true }, lost) in
  not m.tracked && not m.aiming && not m.triggerEdge && not m.triggerArmed && m.grip == vrUp.grip
)
expect not shouldered(run(1.0, lost, vrUp))
// ...and a trigger still held when the hand (or the headset) returns waits
// for a release.
expect (
  let back = run(1.0, pull, sampledInput(vrUp, idle)) in
  not back.triggerArmed && not back.triggerEdge
)
expect (
  let back = run(1.0, vr(hand(levelGrip, 1.0, true, false)), run(1.0, lost, vrUp)) in
  run(30.0, pull, back).shots == 0.0 && run(1.0, pull, run(1.0, rest, run(30.0, pull, back))).shots == 1.0
)
// The EMA keeps weight k of the previous pose: 0 (the default) follows the
// controller exactly, and a jump past 90 degrees snaps.
let gripA = { position: vec(0.0, 0.0, 0.0), forward: vec(0.0, 0.0, 1.0), up: vec(0.0, 1.0, 0.0) }
let gripB = { position: vec(1.0, 0.0, 0.0), forward: vec(1.0, 0.0, 1.0), up: vec(0.0, 1.0, 0.0) }
let gripBack = { gripB with forward: vec(0.0, 0.0, 0.0 - 1.0) }
expect poseSmoothing == 0.0
expect smoothPose(0.0, gripA, gripB) == gripB
expect smoothPose(0.5, gripA, gripB).position == vec(0.5, 0.0, 0.0)
expect smoothPose(0.5, gripA, gripB).forward == vec(0.5, 0.0, 1.0)
expect smoothPose(0.5, gripA, gripBack) == gripBack
// 90 degrees apart, a blend would make forward == up: it snaps instead.
expect (
  let swapped = { gripA with forward: vec(0.0, 1.0, 0.0), up: vec(0.0, 0.0, 1.0) } in
  smoothPose(0.5, gripA, swapped) == swapped
)
expect (
  let moved = { levelGrip with position: { x: 0.3, y: 0.0, z: 0.0 - 0.4 } } in
  run(1.0, vr(hand(moved, 0.0, false, false)), vrUp).grip == Camera3D.mapTrackedPose(rig, moved)
)
// The VR bore leaves the muzzle below the scope axis and meets it at
// zeroRange, whatever the rifle's roll.
expect (
  let rolled = { levelGrip with orientation: { x: 0.0, y: 0.0, z: 0.38268343, w: 0.92387953 } } in
  let m = run(1.0, vr(hand(rolled, 0.0, false, false)), vrUp) in
  let origin = vadd(rifleOrigin(m), poseRot(m.grip, muzzleLocal)) in
  let dir = poseRot(m.grip, boreDirLocal) in
  let t = (zeroRange - muzzleLocal.z) / boreDirLocal.z in
  let gap = vsub(vadd(origin, vscale(t, dir)), vadd(rifleOrigin(m), vscale(zeroRange, unit(m.grip.forward)))) in
  dot(gap, gap) < 0.000001
)
