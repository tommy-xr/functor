// client.fun — 2D asteroids, structured for multiplayer.
//
//   functor -d examples/asteroids-mp run native
//
// Controls: Left/Right (or A/D) rotate, Up (or W) thrusts, Space fires,
// R or Enter restarts after a game over.
//
// This is a fully playable single-pane asteroids — but it is shaped like a
// multiplayer CLIENT. The project splits three ways (`file = module`):
//
//   protocol.fun  the shared `Protocol.Wire` ADT a real transport will carry
//                 (Join/Steer/Snapshot) plus the arena constants,
//   server.fun    the authoritative simulation as PURE step functions,
//   client.fun    this file — input, prediction, and rendering.
//
// The client "predicts" by running the SAME pure `Server.*` steppers the
// authoritative world runs, so when the netsim transport lands, what you see
// here becomes the client-side prediction of the server's snapshots.
//
// THE INTENT GHOST: the faint outline trailing your ship is where the
// SERVER-CONFIRMED ship would be — your own ship, `Protocol.rttTicks`
// (~133 ms) ago, read from a short ring buffer of past ship states. The
// solid ship is the client's optimistic prediction; the ghost is what the
// network has actually acknowledged. (Simulated RTT until the transport
// lands — then the ghost becomes the real snapshot position.)

// ---------- palette ----------
let cyan = Color.rgb(0.35, 0.95, 1.0)
let pink = Color.rgb(1.0, 0.35, 0.75)
let rockColor = Color.rgb(0.62, 0.58, 0.52)
let rockCore = Color.rgb(0.35, 0.32, 0.3)
let bulletColor = Color.rgb(1.0, 0.9, 0.35)
let starColor = Color.rgb(0.75, 0.8, 1.0)
let bg = Color.rgb(0.02, 0.025, 0.06)

// ---------- model ----------
type Phase =
  | Playing
  | GameOver

// The model: phase, the predicted ship + its intent (held controls, sampled
// every tick), bullets/rocks (Protocol records), score/lives/wave, the
// respawn shield, the RNG seed — and `history`, the prediction ring buffer:
// the last rttTicks+1 ship states, newest first. The intent ghost reads its
// oldest entry — the "server-confirmed" pose.
let coast = { turn: 0.0, thrust: false, fire: false }
let myShip = Server.newShip(0.0)

let freshRun = (seed: Random.Seed) =>
  { phase: Playing, ship: myShip, intent: coast, fireLatch: false,
    bullets: [], rocks: Server.spawnWave(seed, 4.0),
    score: 0.0, lives: 3.0, wave: 1.0, shield: 2.0,
    seed: seed, history: [] }

// A fixed seed keeps runs reproducible (swap in Effect.random for variety).
let init = freshRun(Random.seed(0.42))

// ---------- input ----------
// Held controls come from `sampledInput` — the per-tick LEVEL state — so
// rotation and thrust read the keyboard exactly once per simulation step
// (this sampled record is also precisely what a transport would forward to
// the server as a `Protocol.Steer`).
let holds = (k: Key.t, keys: List<Key.t>): bool =>
  keys |> List.any((h) => h == k)

let sampledInput = (m, snap: Input.snapshot) =>
  let keys = snap.heldKeys in
  let left = holds(Key.Left, keys) || holds(Key.A, keys) in
  let right = holds(Key.Right, keys) || holds(Key.D, keys) in
  let thrust = holds(Key.Up, keys) || holds(Key.W, keys) in
  let turn = (if left then 1.0 else 0.0) - (if right then 1.0 else 0.0) in
  { m with intent: { turn: turn, thrust: thrust, fire: m.intent.fire } }

// Firing is an EDGE: GLFW key repeats arrive as isDown = true, so it
// latches on the first press and re-arms on release.
let input = (m, key: Key.t, isDown: bool) =>
  match m.phase with
  | Playing =>
    (match key with
     | Key.Space =>
       (if isDown && not m.fireLatch then
          { m with fireLatch: true,
                   bullets: [Server.fireBullet(m.ship), ..m.bullets] }
        else if not isDown then { m with fireLatch: false }
        else m)
     | _ => m)
  | GameOver =>
    (if isDown && (key == Key.R || key == Key.Enter)
     then freshRun(m.seed)
     else m)

// ---------- simulation ----------
// One predicted tick: run the SAME pure steppers the server runs
// (Server.stepShip / stepBullet / stepRock / splitRock), then remember the
// new ship state in the ghost's ring buffer.
let tickPlaying = (m, dt: float) =>
  let ship = Server.stepShip(m.intent, dt, m.ship) in
  let history = Protocol.pushHistory(ship, m.history) in
  let bullets =
    m.bullets
      |> List.map((b) => Server.stepBullet(dt, b))
      |> List.filter((b) => b.ttl > 0.0) in
  let rocks = m.rocks |> List.map((r) => Server.stepRock(dt, r)) in
  // Bullet/rock hits: struck rocks split, surviving bullets fly on.
  let hitR = (r) =>
    bullets |> List.any((b) => Server.bulletHitsRock(r, b)) in
  let struck = rocks |> List.filter(hitR) in
  let kept = rocks |> List.filter((r) => not hitR(r)) in
  let survivors =
    bullets |> List.filter((b) =>
      not (rocks |> List.any((r) => Server.bulletHitsRock(r, b)))) in
  let remaining = kept |> List.append(struck |> List.concatMap(Server.splitRock)) in
  let gained = struck |> List.fold((acc, r) => acc + Protocol.pointsFor(r.size), 0.0) in
  let shield = Math.max(0.0, m.shield - dt) in
  let base = { m with ship: ship, history: history, bullets: survivors,
                      rocks: remaining, score: m.score + gained, shield: shield } in
  if shield == 0.0 && (remaining |> List.any((r) => Server.rockHitsShip(ship, r))) then
    // Hit: lose a life; respawn at the center behind a fresh shield (and a
    // fresh ghost buffer — the "server" respawned us too).
    (if m.lives <= 1.0 then { base with lives: 0.0, phase: GameOver }
     else { base with lives: m.lives - 1.0, ship: myShip, history: [],
                      shield: 2.5 })
  else if List.isEmpty(remaining) then
    // Wave cleared: thread the seed forward and spawn a bigger one.
    let (_, nextSeed) = Random.step(m.seed) in
    { base with wave: m.wave + 1.0, seed: nextSeed,
                shield: Math.max(1.5, shield),
                rocks: Server.spawnWave(nextSeed, 3.0 + m.wave) }
  else base

let tick = (m, dt: float, tts: float) =>
  match m.phase with
  | Playing => tickPlaying(m, dt)
  | GameOver =>
    // Keep the field drifting behind the game-over card.
    { m with rocks: m.rocks |> List.map((r) => Server.stepRock(dt, r)),
             bullets: m.bullets
                        |> List.map((b) => Server.stepBullet(dt, b))
                        |> List.filter((b) => b.ttl > 0.0) }

// ---------- rendering ----------
// The ship triangle, nose up (+y at angle 0). Sprite.polygon points are
// used verbatim, so building it around the origin lets Sprite.rotate/move
// place it — the same convention Server.stepShip integrates in.
let nose = { x: 0.0, y: 1.4 }
let tailL = { x: -0.9, y: -1.0 }
let tailR = { x: 0.9, y: -1.0 }

let shipBody = (thrusting: bool, tts: float) =>
  let flame =
    if thrusting && Math.sin(tts * 30.0) > -0.4 then
      [Sprite.polygon(pink, [{ x: -0.4, y: -1.0 }, { x: 0.4, y: -1.0 },
                             { x: 0.0, y: -1.9 }])]
    else [] in
  Sprite.group([Sprite.polygon(cyan, [nose, tailL, tailR]), ..flame])

// THE INTENT GHOST: the same triangle as an outline (three lines — there is
// no unfilled polygon), faded, drawn at the ring buffer's OLDEST ship state:
// where the server's confirmation would currently be, rttTicks behind the
// prediction you steer.
let ghostOutline =
  Sprite.group([
    Sprite.line(cyan, 0.12, nose, tailL),
    Sprite.line(cyan, 0.12, tailL, tailR),
    Sprite.line(cyan, 0.12, tailR, nose),
  ])

let ghostSprites = (m) =>
  match List.nth(Protocol.rttTicks, m.history) with
  | Option.None => []       // buffer still filling (right after a respawn)
  | Option.Some(g) =>
    [Sprite.group([
       ghostOutline
         |> Sprite.rotate(Angle.radians(g.angle))
         |> Sprite.move(g.x, g.y),
       Sprite.text(cyan, 0.7, "SERVER") |> Sprite.move(g.x, g.y - 2.2),
     ]) |> Sprite.fade(0.35)]

// Rocks are filled circles with a darker core so the size classes read.
let rockSprite = (r) =>
  let rad = Protocol.radiusOf(r.size) in
  Sprite.group([
    Sprite.circle(rockColor, rad),
    Sprite.circle(rockCore, rad * 0.55),
  ]) |> Sprite.move(r.x, r.y)

let bulletSprite = (b) =>
  Sprite.circle(bulletColor, 0.18) |> Sprite.move(b.x, b.y)

// A fixed backdrop of faint stars, each drawn from its own forked stream.
let starSeed = Random.seed(9.1)
let stars =
  List.range(40.0) |> List.map((n) =>
    let (x01, s1) = Random.step(starSeed |> Random.fork(n)) in
    let (y01, s2) = Random.step(s1) in
    let (tw, _) = Random.step(s2) in
    Sprite.circle(starColor, 0.06 + tw * 0.07)
      |> Sprite.fade(0.25 + tw * 0.5)
      |> Sprite.move((x01 * 2.0 - 1.0) * Protocol.halfW,
                     (y01 * 2.0 - 1.0) * Protocol.halfH))

// HUD text, left-aligned via Sprite.measure (text is centered on its box,
// so the left edge lands at x + width/2).
let hudLine = (color: Color.t, y: float, s: string) =>
  let w = Sprite.measure(1.1, s).width in
  Sprite.text(color, 1.1, s)
    |> Sprite.move(1.0 - Protocol.halfW + w * 0.5, y)

let hud = (m) =>
  Sprite.group([
    hudLine(bulletColor, Protocol.halfH - 1.2,
            Text.concat("SCORE ", Text.fixed(m.score, 0.0))),
    hudLine(cyan, Protocol.halfH - 2.6,
            Text.concat("SHIPS ", Text.fixed(m.lives, 0.0))),
  ])

let gameOverCard = () =>
  Sprite.group([
    Sprite.text(pink, 2.4, "GAME OVER") |> Sprite.move(0.0, 1.5),
    Sprite.text(cyan, 1.0, "PRESS R TO RESTART") |> Sprite.move(0.0, -1.0),
  ])

// The respawn shield blinks the ship, arcade-style.
let shipSprites = (m, tts: float) =>
  match m.phase with
  | GameOver => []
  | Playing =>
    (if m.shield > 0.001 && Math.sin(tts * 18.0) <= 0.0 then []
     else [shipBody(m.intent.thrust, tts)
             |> Sprite.rotate(Angle.radians(m.ship.angle))
             |> Sprite.move(m.ship.x, m.ship.y)])

let draw = (m, tts: float) =>
  let card = (match m.phase with | GameOver => [gameOverCard()] | Playing => []) in
  Frame.create2D(
    Camera2D.create(Protocol.halfW * 2.0, Protocol.halfH * 2.0),
    Sprite.group(
      stars
        |> List.append(m.rocks |> List.map(rockSprite))
        |> List.append(m.bullets |> List.map(bulletSprite))
        |> List.append(ghostSprites(m))          // ghost under the live ship
        |> List.append(shipSprites(m, tts))
        |> List.append([hud(m)])
        |> List.append(card)))
    |> Frame.withClearColor(bg)

// The pure logic lives (and is `expect`-tested) in protocol.fun and
// server.fun — run `functor -d examples/asteroids-mp test` to see them.
