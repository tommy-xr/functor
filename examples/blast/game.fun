// blast — Physics.explode and Physics.forceVolume.
//
// A pile of crates sits on a slab; E sets off a blast at its foot, a radial
// impulse on every dynamic body in range, lifted so the crates fly up and
// out. Beside it a fan blows a gusty draft along a lane of paper cards: a
// force volume declared in the physics scene, which pushes and gusts every
// body inside it each step. SPACE rebuilds the pile and the cards.
//
//   functor -d examples/blast run native

let groundTag = Physics.tag("ground")
let fanTag = Physics.tag("fan")
// Tags carry the SPACE generation, so a rebuild spawns fresh, still bodies
// rather than teleporting spinning ones.
let bodyTag = (kind, drop, i) =>
  Physics.tag(Text.concat(kind, Text.concat(Text.fromFloat(drop), Text.concat("-", Text.fromFloat(i)))))
let crateTag = (drop, i) => bodyTag("crate-", drop, i)
let cardTag = (drop, i) => bodyTag("card-", drop, i)

// The pile: `side` × `side` crates, `layers` high.
let side = 5.0
let layers = 2.0
let crateCount = side * side * layers
let crateSize = 0.8
let pileAt = Vec3.make(-2.5, 0.0, 1.5)

let crateSpot = (i) =>
  let x = Math.mod(i, side) in
  let z = Math.mod(Math.floor(i / side), side) in
  let y = Math.floor(i / (side * side)) in
  let gap = crateSize + 0.04 in
  Vec3.make(Vec3.x(pileAt) + (x - (side - 1.0) / 2.0) * gap,
            crateSize / 2.0 + y * (crateSize + 0.01),
            Vec3.z(pileAt) + (z - (side - 1.0) / 2.0) * gap)

let crateBody = (drop, i) =>
  Physics.dynamic(crateTag(drop, i), Physics.box(crateSize, crateSize, crateSize))
    |> Physics.at(crateSpot(i))
    |> Physics.friction(0.7)

// The fan's lane runs along -Z from the fan housing.
let lane = Vec3.make(4.5, 1.0, 0.5)
let laneSize = Vec3.make(2.6, 2.0, 9.0)
let cardCount = 14.0

// Paper: light and flat, with damping so it drifts instead of dropping.
let cardBody = (drop, i) =>
  Physics.dynamic(cardTag(drop, i), Physics.box(0.42, 0.03, 0.3))
    |> Physics.at(Vec3.make(Vec3.x(lane) - 0.9 + Math.mod(i, 4.0) * 0.6,
                            0.05 + Math.floor(i / 4.0) * 0.05,
                            Vec3.z(lane) + 2.5 - Math.floor(i / 4.0) * 0.9))
    |> Physics.rotateY(Angle.degrees(i * 37.0))
    |> Physics.mass(0.03)
    |> Physics.linearDamping(1.2)
    |> Physics.angularDamping(0.6)

// A draft along -Z that nearly floats a card (0.29 N of weight); the gusts
// lift it off the floor.
let draft = Physics.forceVolume(fanTag, {
  center: lane, size: laneSize,
  force: Vec3.make(0.0, 0.26, -0.15),
  turbulence: 0.12, drag: 1.0 })

let blastAt = Vec3.make(Vec3.x(pileAt), 0.2, Vec3.z(pileAt))

let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0),
    [Physics.fixed(groundTag, Physics.box(30.0, 0.4, 30.0)) |> Physics.at(Vec3.make(0.0, -0.2, 0.0)),
     draft]
      |> List.append(List.range(crateCount) |> List.map((i) => crateBody(model.drop, i)))
      |> List.append(List.range(cardCount) |> List.map((i) => cardBody(model.drop, i))))

let init = { drop: 0.0, spin: 0.0, awake: 0.0 }

// A debug readout for `/state`: a sleeping body's velocity is exactly zero,
// so `awake` counts the crates still simulating after a blast.
let moving = (tag) =>
  let v = Physics.linearVelocity(tag) in
  v.x != 0.0 || v.y != 0.0 || v.z != 0.0

let tick = (model, dt, tts) =>
  { model with
      spin: model.spin + dt * 900.0,
      awake: List.length(List.filter((i) => moving(crateTag(model.drop, i)), List.range(crateCount))) }

// Key-ups never repeat, so acting on release needs no latch.
let input = (model, key, isDown) =>
  if isDown then model
  else
    match key with
    | Key.E =>
      (model, Physics.explodeWith(blastAt, 4.5, 4.0, { falloff: Physics.Linear, lift: 0.7 }))
    | Key.Space => { model with drop: model.drop + 1.0 }
    | _ => model

let crateVisual = (drop, i) =>
  Scene.cube()
    |> Scene.scale(crateSize)
    |> Scene.lit(Color.rgb(Math.clamp01(0.5 + 0.02 * Math.mod(i, 9.0)), 0.36, 0.22))
    |> Physics.transformed(crateTag(drop, i))

let cardVisual = (drop, i) =>
  Scene.cube()
    |> Scene.scaleXYZ(0.42, 0.03, 0.3)
    |> Scene.lit(Color.rgb(0.92, 0.91, 0.86))
    |> Physics.transformed(cardTag(drop, i))

// The fan housing at the lane's +Z end, blades spinning, and the draft as a
// faint glow (lifted off the ground plane, so the two don't z-fight).
let fanVisual = (model) =>
  let back = Vec3.z(lane) + Vec3.z(laneSize) / 2.0 in
  let blade = (a) =>
    Scene.cube() |> Scene.scaleXYZ(1.8, 0.22, 0.04) |> Scene.rotateZ(Angle.degrees(model.spin + a))
      |> Scene.lit(Color.rgb(0.75, 0.78, 0.82)) in
  Scene.group([
    Scene.cube() |> Scene.scaleXYZ(2.4, 2.2, 0.3) |> Scene.translate(Vec3.make(0.0, 0.0, 0.25))
      |> Scene.lit(Color.rgb(0.16, 0.17, 0.2)),
    blade(0.0),
    blade(90.0),
  ]) |> Scene.translate(Vec3.make(Vec3.x(lane), Vec3.y(lane), back))

let draftVisual =
  Scene.cube()
    |> Scene.scaleXYZ(Vec3.x(laneSize), Vec3.y(laneSize) - 0.04, Vec3.z(laneSize))
    |> Scene.additive(Color.rgb(0.015, 0.035, 0.05))
    |> Scene.translate(Vec3.add(lane, Vec3.make(0.0, 0.02, 0.0)))

let draw = (model, tts) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(9.0, 6.5, -9.0), Vec3.make(1.0, 0.8, 1.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(30.0) |> Scene.lit(Color.rgb(0.5, 0.53, 0.57)),
      Scene.group(List.range(crateCount) |> List.map((i) => crateVisual(model.drop, i))),
      Scene.group(List.range(cardCount) |> List.map((i) => cardVisual(model.drop, i))),
      fanVisual(model),
      draftVisual,
    ]),
    [
      Light.ambient(Color.rgb(0.14, 0.14, 0.17)),
      Light.directional(Vec3.make(0.5, -1.0, 0.35), Color.rgb(1.0, 0.98, 0.95), 0.9) |> Light.castShadows,
    ])

expect Math.abs(Vec3.y(crateSpot(0.0)) - crateSize / 2.0) < 0.0001
expect crateTag(1.0, 2.0) == Physics.tag("crate-1-2")
expect crateCount == 50.0
