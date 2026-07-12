// examples/asteroids — a complete Asteroids clone in Functor Lang: menu,
// three waves, score, lives, win/lose, restart, and sound.
//
//   npm run fetch:assets                       # once: the ship model (CC0)
//   functor -d examples/asteroids run native
//
// Sounds are CC0 (Kenney) and checked in; sources in ASSETS.md. A missing
// ship.glb still runs (empty fallback + logged error) — fetch to see it.
//
// Controls: Left/Right (or A/D) rotate, Up (or W) thrusts, Space fires.
// Enter (or the Start button) starts; after a win/loss R or Enter plays
// again and M returns to the menu.
//
// The playfield is the XZ plane (Y-up, camera overhead); positions wrap
// at the field edges like the arcade original.

// ---------- tuning ----------
let worldX = 21.0        // half-extent of the playfield in x
let worldZ = 15.0        // half-extent in z
let turnSpeed = 3.6      // radians/second
let thrustAccel = 16.0   // units/second^2
let drag = 0.5           // fraction of velocity shed per second
let bulletSpeed = 26.0
let bulletLife = 1.0     // seconds
let shipRadius = 0.7
let respawnShield = 2.5  // seconds of invulnerability after (re)spawn
let finalWave = 3.0
let tau = Math.pi * 2.0

type Phase =
  | Menu
  | Playing
  | Won
  | Lost

type Msg =
  | Seeded(r: float)     // Effect.random result: begin a fresh run
  | StartClicked
  | RestartClicked

// ---------- spawning ----------
let radiusOf = (size) =>
  match size with
  | 3.0 => 2.2
  | 2.0 => 1.4
  | _ => 0.8

let pointsFor = (size) =>
  match size with
  | 3.0 => 20.0
  | 2.0 => 50.0
  | _ => 100.0

let waveRocks = (wave) => 2.0 + wave

// A large rock on a ring around the center (never on the ship), drifting
// in a randomly-picked direction with a random spin. Threads the seed:
// returns (rock, nextSeed).
let spawnRock = (seed) =>
  let (u1, s1) = Random.step(seed) in
  let (ring, s2) = Random.range(8.0, 14.0, s1) in
  let (u3, s3) = Random.step(s2) in
  let (speed, s4) = Random.range(1.5, 3.5, s3) in
  let (spin, s5) = Random.range(0.0 - 1.5, 1.5, s4) in
  // Store an INTEGER PRNG state as the rock's stable seed (s5 is a next-state,
  // always a large integer). A fractional [0,1) VALUE would truncate to
  // counter 0 in Random.step and reseed every rock identically. Thread one
  // step past it so the next rock's stream is disjoint.
  let (ignored, s6) = Random.step(s5) in
  let ang = u1 * tau in
  let dir = u3 * tau in
  ({ x: Math.sin(ang) * ring,
     z: Math.cos(ang) * ring,
     vx: Math.sin(dir) * speed,
     vz: Math.cos(dir) * speed,
     size: 3.0,
     spin: spin,
     seed: s5 }, s6)

// Spawn `count` rocks, threading one PRNG stream through them all — no
// correlated streams (the sin-hash bug the findings log flagged). Returns
// (rocks, nextSeed) so the caller can advance the wave's stream.
let spawnWave = (seed, count) =>
  List.range(count) |> List.fold((acc, _) =>
    let (rocks, s) = acc in
    let (rock, s2) = spawnRock(s) in
    ([rock, ..rocks], s2),
    ([], seed))

let newShip = {
  pos: { x: 0.0, z: 0.0 },
  vel: { x: 0.0, z: 0.0 },
  heading: 0.0,
}

let init =
  let (backdrop, ignored) = spawnWave(0.42, 5.0) in   // the menu's drifting backdrop
  { phase: Menu,
    ship: newShip,
    bullets: [],
    asteroids: backdrop,
    score: 0.0,
    lives: 3.0,
    wave: 1.0,
    held: { left: false, right: false, thrust: false },
    fireHeld: false,
    shield: 0.0,
    seed: 0.42 }

// ---------- geometry helpers ----------
let wrap = (v, limit) =>
  match v > limit with
  | true => 0.0 - limit
  | false => (match v < 0.0 - limit with | true => limit | false => v)

// Shortest separation on the wrapped (toroidal) field, so entities near
// opposite edges are neighbors — a bullet at the left edge hits a rock
// half-hidden at the right edge, like the arcade original.
let wrapDelta = (d, limit) =>
  match d > limit with
  | true => d - limit * 2.0
  | false => (match d < 0.0 - limit with | true => d + limit * 2.0 | false => d)

let dist2 = (ax, az, bx, bz) =>
  let dx = wrapDelta(ax - bx, worldX) in
  let dz = wrapDelta(az - bz, worldZ) in
  dx * dx + dz * dz

let sq = (v) => v * v

// Rotate a 2D (x, z) vector by ang radians.
let rotVec = (vx, vz, ang) =>
  { x: vx * Math.cos(ang) - vz * Math.sin(ang),
    z: vx * Math.sin(ang) + vz * Math.cos(ang) }

// ---------- collisions ----------
let bulletHitsRock = (a, b) =>
  dist2(a.x, a.z, b.x, b.z) < sq(radiusOf(a.size))

// One-to-one assignment in a single pass: each rock consumes at most one
// bullet and each bullet kills at most one rock (classic behavior).
// Returns (bulletsLeft, keptRocks, struckRocks).
let assignHits = (bullets, rocks) =>
  rocks |> List.fold((acc, a) =>
    let (bs, kept, struck) = acc in
    (match bs |> List.any((b) => bulletHitsRock(a, b)) with
     | true => (Lib.removeFirst((b) => bulletHitsRock(a, b), bs), kept, [a, ..struck])
     | false => (bs, [a, ..kept], struck)),
    (bullets, [], []))

// ---------- splitting ----------
// A destroyed rock (sizes 3/2) splits into two children of the next size
// down, flung off the parent's course by opposite random angles. Each
// child draws its spin + seed from the parent's seed stream.
let childRock = (a, ang, seed) =>
  let v = rotVec(a.vx * 1.35, a.vz * 1.35, ang) in
  let (spin, s1) = Random.range(0.0 - 2.0, 2.0, seed) in
  // Store an integer seed for the child (see spawnRock); thread one step on.
  let (ignored, s2) = Random.step(s1) in
  ({ x: a.x,
     z: a.z,
     vx: v.x,
     vz: v.z,
     size: a.size - 1.0,
     spin: spin,
     seed: s1 }, s2)

let splitRock = (a) =>
  match a.size with
  | 1.0 => []
  | _ =>
    // Draw split angles from a sub-stream distinct from rockScene's (which
    // reads a.seed directly), so a rock's shape and its split stay independent.
    let (ang1, s1) = Random.range(0.7, 1.7, a.seed + 1.0) in
    let (ang2, s2) = Random.range(0.7, 1.7, s1) in
    let (c1, s3) = childRock(a, ang1, s2) in
    let (c2, ignored) = childRock(a, 0.0 - ang2, s3) in
    [c1, c2]

// ---------- effects ----------
let fxOf = (list) =>
  match list with
  | [] => Effect.none()
  | _ => Effect.batch(list)

// ---------- a fresh run ----------
let freshRun = (model, seed) =>
  let (rocks, seed2) = spawnWave(seed, waveRocks(1.0)) in
  { model with
      phase: Playing,
      ship: newShip,
      bullets: [],
      asteroids: rocks,
      score: 0.0,
      lives: 3.0,
      wave: 1.0,
      held: { left: false, right: false, thrust: false },
      fireHeld: false,
      shield: respawnShield,
      seed: seed2 }

let update = (model, msg) =>
  match msg with
  | Seeded(r) => freshRun(model, r)
  | StartClicked => (model, Effect.random(Seeded))
  | RestartClicked => (model, Effect.random(Seeded))

// ---------- input ----------
let setHeld = (held, key, isDown) =>
  match key with
  | "Left" => { held with left: isDown }
  | "A" => { held with left: isDown }
  | "Right" => { held with right: isDown }
  | "D" => { held with right: isDown }
  | "Up" => { held with thrust: isDown }
  | "W" => { held with thrust: isDown }
  | _ => held

// Spawn a bullet at the ship's nose, inheriting the ship's velocity.
let fire = (model) =>
  let s = model.ship in
  let b = { x: s.pos.x + Math.sin(s.heading) * 1.0,
            z: s.pos.z + Math.cos(s.heading) * 1.0,
            vx: s.vel.x + Math.sin(s.heading) * bulletSpeed,
            vz: s.vel.z + Math.cos(s.heading) * bulletSpeed,
            ttl: bulletLife } in
  ({ model with bullets: [b, ..model.bullets], fireHeld: true },
   Effect.play("laser.ogg"))

// GLFW key repeats arrive as isDown = true, so firing latches on the
// rising edge (fireHeld clears on release). Literal tuple patterns map
// (key, isDown) directly.
let inputPlaying = (model, key, isDown) =>
  let m = { model with held: setHeld(model.held, key, isDown) } in
  match (key, isDown) with
  | ("Space", true) => (match m.fireHeld with | true => m | false => fire(m))
  | ("Space", false) => { m with fireHeld: false }
  | ("M", true) => { m with phase: Menu }
  | (_, _) => m

let input = (model, key, isDown) =>
  match model.phase with
  | Menu =>
    (match (key, isDown) with
     | ("Enter", true) => (model, Effect.random(Seeded))
     | ("Space", true) => (model, Effect.random(Seeded))
     | (_, _) => model)
  | Playing => inputPlaying(model, key, isDown)
  | _ =>
    (match (key, isDown) with
     | ("Enter", true) => (model, Effect.random(Seeded))
     | ("R", true) => (model, Effect.random(Seeded))
     | ("M", true) => { model with phase: Menu }
     | (_, _) => model)

// ---------- simulation ----------
let axis = (neg, pos) =>
  (match pos with | true => 1.0 | false => 0.0)
    - (match neg with | true => 1.0 | false => 0.0)

let stepShip = (ship, held, dt) =>
  let heading = ship.heading + axis(held.left, held.right) * turnSpeed * dt in
  let acc = (match held.thrust with | true => thrustAccel | false => 0.0) in
  // Floored at 0 so a pathological frame (dt > 2s) can't reverse velocity.
  let keep = Math.max(0.0, 1.0 - drag * dt) in
  let vx = (ship.vel.x + Math.sin(heading) * acc * dt) * keep in
  let vz = (ship.vel.z + Math.cos(heading) * acc * dt) * keep in
  { pos: { x: wrap(ship.pos.x + vx * dt, worldX),
           z: wrap(ship.pos.z + vz * dt, worldZ) },
    vel: { x: vx, z: vz },
    heading: heading }

let moveRocks = (rocks, dt) =>
  rocks |> List.map((a) =>
    { a with x: wrap(a.x + a.vx * dt, worldX + 2.5),
             z: wrap(a.z + a.vz * dt, worldZ + 2.5) })

let moveBullets = (bullets, dt) =>
  bullets
    |> List.map((b) => { b with x: wrap(b.x + b.vx * dt, worldX + 0.5),
                                z: wrap(b.z + b.vz * dt, worldZ + 0.5),
                                ttl: b.ttl - dt })
    |> List.filter((b) => b.ttl > 0.0)

let rockHitsShip = (ship, a) =>
  dist2(a.x, a.z, ship.pos.x, ship.pos.z) < sq(radiusOf(a.size) + shipRadius)

let tickPlaying = (model, dt, tts) =>
  let ship = stepShip(model.ship, model.held, dt) in
  let movedB = moveBullets(model.bullets, dt) in
  let movedA = moveRocks(model.asteroids, dt) in
  let hits = assignHits(movedB, movedA) in
  let (keptB, kept, struck) = hits in
  let children = struck |> List.map(splitRock) |> List.flatten in
  let rocks = kept |> List.append(children) in
  let gained = struck |> List.fold((acc, a) => acc + pointsFor(a.size), 0.0) in
  let anyKill = not (List.isEmpty(struck)) in
  let killFx = (match anyKill with
                | true => [Effect.play("explosion.ogg")]
                | false => []) in
  let shield = Math.max(0.0, model.shield - dt) in
  let shipHit = shield == 0.0 && (rocks |> List.any((a) => rockHitsShip(ship, a))) in
  let base = { model with
                 ship: ship,
                 bullets: keptB,
                 asteroids: rocks,
                 score: model.score + gained,
                 shield: shield } in
  match shipHit with
  | true =>
    (let lives = model.lives - 1.0 in
     match lives < 1.0 with
     | true => ({ base with lives: 0.0, phase: Lost },
                fxOf(killFx |> List.append([Effect.play("ship-explosion.ogg")])))
     | false => ({ base with lives: lives, ship: newShip, shield: respawnShield },
                 fxOf(killFx |> List.append([Effect.play("ship-explosion.ogg")]))))
  | false =>
    (match List.isEmpty(rocks) with
     | true =>
       (match model.wave < finalWave with
        | true =>
          (let (next, nextSeed) = spawnWave(model.seed, waveRocks(model.wave + 1.0)) in
           ({ base with wave: model.wave + 1.0,
                        seed: nextSeed,
                        shield: Math.max(1.5, shield),
                        asteroids: next },
            fxOf(killFx)))
        | false => ({ base with phase: Won }, fxOf(killFx)))
     | false => (base, fxOf(killFx)))

let tick = (model, dt, tts) =>
  match model.phase with
  | Playing => tickPlaying(model, dt, tts)
  | _ => { model with asteroids: moveRocks(model.asteroids, dt),
                      // keep aging in-flight bullets so a quit-to-menu /
                      // game-over doesn't freeze them on screen
                      bullets: moveBullets(model.bullets, dt) }

// ---------- rendering ----------
// Stars as tiny upward-facing planes (spheres this small render as
// speckle clusters from overhead). One PRNG stream threads all 60 stars,
// so brightness/size/position are independent (no closed-curve clumping).
let starfield = () =>
  let (stars, ignored) = List.range(60.0) |> List.fold((acc, _) =>
    let (out, seed) = acc in
    let (glow, s1) = Random.range(0.45, 1.0, seed) in
    let (scale, s2) = Random.range(0.14, 0.28, s1) in
    let (fx, s3) = Random.range(0.0 - 1.0, 1.0, s2) in
    let (fz, s4) = Random.range(0.0 - 1.0, 1.0, s3) in
    ([Scene.plane()
        |> Scene.scale(scale)
        |> Scene.emissive(glow, glow, glow * 1.08)
        |> Scene.translate(fx * (worldX + 8.0), 0.0 - 4.0, fz * (worldZ + 8.0)), ..out],
     s4),
    ([], 7.7)) in
  stars

let rockScene = (a, tts) =>
  let r = radiusOf(a.size) in
  let (sx, s1) = Random.range(0.75, 1.25, a.seed) in
  let (sy, s2) = Random.range(0.75, 1.25, s1) in
  let (sz, ignored) = Random.range(0.75, 1.25, s2) in
  Scene.sphere()
    |> Scene.scaleXYZ(r * sx, r * sy, r * sz)
    |> Scene.rotateY(Angle.radians(tts * a.spin))
    |> Scene.lit(0.62, 0.55, 0.47)
    |> Scene.translate(a.x, 0.0, a.z)

let bulletScene = (b) =>
  Scene.sphere()
    |> Scene.scale(0.14)
    |> Scene.emissive(1.0, 0.9, 0.3)
    |> Scene.translate(b.x, 0.0, b.z)

// The ship: Kenney's craft_racer (CC0, see ASSETS.md; fetched, not checked
// in) with an emissive flame while thrusting. The group faces +Z at
// heading 0 (forward = (sin h, cos h)).
let shipBody = (thrusting) =>
  let flame =
    (match thrusting with
     | true => [Scene.cube()
                  |> Scene.scaleXYZ(0.16, 0.16, 0.6)
                  |> Scene.translate(0.0, 0.0, 0.0 - 0.85)
                  |> Scene.emissive(1.0, 0.55, 0.1)]
     | false => []) in
  Scene.group([
    // Kenney crafts model nose-toward--Z; flip to face our +Z forward.
    // The glb's craft_racer node carries a baked [2, 0, 1.5] placement
    // translation (kit-scene leftover) — counter it first or the body
    // renders displaced from the ship's true position.
    Scene.model("ship.glb")
      |> Scene.translate(0.0 - 2.0, 0.0, 0.0 - 1.5)
      |> Scene.rotateY(Angle.degrees(180.0))
      |> Scene.scale(1.5),
  ] |> List.append(flame))

// Hidden entirely on Menu/Lost; blinks while the respawn shield runs.
let shipScenes = (model, tts) =>
  let visible = model.shield < 0.001 || Math.sin(tts * 18.0) > 0.0 in
  match visible with
  | false => []
  | true =>
    [shipBody(model.held.thrust && model.phase == Playing)
       |> Scene.rotateY(Angle.radians(model.ship.heading))
       |> Scene.translate(model.ship.pos.x, 0.0, model.ship.pos.z)]

// Centered arcade-style screens, built from Font's emissive cube glyphs
// in scene space. They vanish the frame the phase changes.
let pressEnter = (tts, z) =>
  match Math.sin(tts * 4.0) > 0.0 - 0.3 with   // arcade blink, mostly on
  | true => [Font.word(0.3, 0.0, z,
               [Font.gP, Font.gR, Font.gE, Font.gS, Font.gS, Font.gSpace,
                Font.gE, Font.gN, Font.gT, Font.gE, Font.gR])
               |> Scene.emissive(0.92, 0.92, 0.92)]
  | false => []

// Titles float at y=2.5, above the rock plane, so a drifting rock passes
// under the letters instead of hiding them.
let titleScenes = (model, tts) =>
  let screens =
    (match model.phase with
     | Playing => []
     | Menu =>
       [Font.word(0.72, 0.0, 0.0 - 6.0,
          [Font.gA, Font.gS, Font.gT, Font.gE, Font.gR, Font.gO, Font.gI, Font.gD, Font.gS])
          |> Scene.emissive(0.55, 1.0, 0.65),
        ..pressEnter(tts, 0.0 - 1.5)]
     | Won =>
       [Font.word(0.55, 0.0, 0.0 - 5.0,
          [Font.gY, Font.gO, Font.gU, Font.gSpace, Font.gW, Font.gI, Font.gN])
          |> Scene.emissive(0.55, 1.0, 0.65),
        ..pressEnter(tts, 0.0 - 1.5)]
     | Lost =>
       [Font.word(0.55, 0.0, 0.0 - 5.0,
          [Font.gG, Font.gA, Font.gM, Font.gE, Font.gSpace, Font.gO, Font.gV, Font.gE, Font.gR])
          |> Scene.emissive(1.0, 0.45, 0.35),
        ..pressEnter(tts, 0.0 - 1.5)]) in
  screens |> List.map((s) => s |> Scene.translate(0.0, 2.5, 0.0))

let draw = (model, tts) =>
  let rocks = model.asteroids |> List.map((a) => rockScene(a, tts)) in
  let bullets = model.bullets |> List.map(bulletScene) in
  let ship =
    (match model.phase with
     | Menu => []
     | Lost => []
     | _ => shipScenes(model, tts)) in
  Frame.createLit(
    Camera.lookAt(0.0, 46.0, 10.0, 0.0, 0.0, 0.0),
    Scene.group([Scene.group(starfield()), Scene.group(rocks),
                 Scene.group(bullets), Scene.group(ship),
                 Scene.group(titleScenes(model, tts))]),
    [Light.ambient(0.3, 0.3, 0.38),
     Light.directional(-0.45, -1.0, -0.3, 1.0, 0.97, 0.9, 1.1)])
    // Deep-space background: an explicit clear color (no fog trick needed).
    |> Frame.withClearColor(0.01, 0.012, 0.035)

// ---------- sound ----------
// One-shots (laser/explosions) fire as Effects above; the thrust loop is
// a reconciled soundscape voice that exists only while thrusting.
let soundScape = (model) =>
  match model.phase with
  | Playing =>
    (match model.held.thrust with
     | true => AudioScene.create([
         AudioSource.ambient("thrust", "thrust-loop.ogg") |> AudioSource.gain(0.45)])
     | false => AudioScene.empty())
  | _ => AudioScene.empty()

// ---------- HUD / menus ----------
let f0 = (n) => Text.fixed(n, 0.0)

let hudUi = (model) =>
  Ui.column([
    Ui.textColor(1.0, 0.9, 0.3, Text.concat("SCORE  ", f0(model.score))),
    Ui.text(Text.concat("LIVES  ", f0(model.lives))),
    Ui.text(Text.concat("WAVE   ", Text.concat(f0(model.wave), Text.concat(" / ", f0(finalWave))))),
  ]) |> Ui.panel(Ui.topLeft())

// The big arcade title/prompt is scene-space (titleScenes); this centered
// panel carries the controls reference and a clickable Start.
let menuUi = (model) =>
  Ui.column([
    Ui.text("Left/Right or A/D  rotate"),
    Ui.text("Up or W            thrust"),
    Ui.text("Space              fire"),
    Ui.button("Start", StartClicked),
  ]) |> Ui.panel(Ui.center())

let endUi = (title, model) =>
  Ui.column([
    Ui.textColor(1.0, 0.5, 0.4, title),
    Ui.text(Text.concat("Final score  ", f0(model.score))),
    Ui.text(""),
    Ui.text("R or Enter to play again, M for menu"),
    Ui.button("Play again", RestartClicked),
  ]) |> Ui.panel(Ui.center())

let ui = (model) =>
  match model.phase with
  | Menu => menuUi(model)
  | Playing => hudUi(model)
  | Won => endUi("YOU WIN", model)
  | Lost => endUi("GAME OVER", model)
