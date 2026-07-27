// client.fun — 2D asteroids, structured for multiplayer.
//
//   functor -d examples/asteroids-mp run native
//
// Controls: Left/Right (or A/D) rotate, Up (or W) thrusts, hold Space to
// fire (the shared cooldown is the fire rate), R or Enter restarts after
// a game over.
//
// This is a playable asteroids arena that HOSTS the authoritative
// simulation in-process. The project splits four ways (`file = module`):
//
//   protocol.fun  the shared `Protocol.Wire` ADT a real transport will carry
//                 (Join/Steer/Snapshot) plus the arena constants,
//   server.fun    the authoritative `World` and its PURE step functions,
//   bot.fun       a tiny scripted pilot standing in for a remote peer,
//   client.fun    this file — input, the two joined pilots, and rendering.
//
// TWO pilots join the world through the real protocol path: YOU (cyan),
// and a tiny bot (pink) standing in for a remote peer. Every tick each
// one's Intent is folded in as a `Protocol.Steer` via `Server.recv` —
// keyed by the sender's connection pid, exactly as arriving packets will
// be — then `Server.step` advances the world once. When the netsim
// transport lands, the world moves behind the wire and this file keeps
// only its own Steer; nothing else changes shape.
//
// THE CONFIRMED GHOST: the faint outline trailing your ship is where the
// server's CONFIRMATION of your ship would currently be — your own ship,
// `Protocol.rttTicks` (~133 ms) ago, read from a short ring buffer of past
// ship states. The solid ship is the optimistic prediction; the ghost is
// what the network would have acknowledged so far. Its "confirmed" label
// appears only while the two are visibly apart (accelerate or turn hard).

// ---------- palette ----------
let cyan = Color.rgb(0.35, 0.95, 1.0)
let pink = Color.rgb(1.0, 0.35, 0.75)
let p2 = Color.rgb(0.91, 0.35, 0.72)    // player 2 — the site's --scrub-p2 pink (#e858b8)
let rockColor = Color.rgb(0.62, 0.58, 0.52)
let rockCore = Color.rgb(0.35, 0.32, 0.3)
let bulletColor = Color.rgb(1.0, 0.9, 0.35)
let starColor = Color.rgb(0.75, 0.8, 1.0)
let bg = Color.rgb(0.02, 0.025, 0.06)

// ---------- model ----------
type Phase =
  | Playing
  | GameOver

// The model: phase, the two pids this client knows (its own and the
// bot's), the authoritative World, MY held intent (sampled each tick),
// `history` — the ghost's ring buffer of my last rttTicks+1 ship states,
// newest first — plus lives, the respawn shield, and the run's seed.
let freshRun = (seed: Random.Seed) =>
  let (w1, meWelcome) = Server.join(Server.newWorld(seed)) in
  let (w2, botWelcome) = Server.join(w1) in
  { phase: Playing,
    myPid: Protocol.welcomePid(meWelcome),
    botPid: Protocol.welcomePid(botWelcome),
    world: w2, intent: Server.coast, history: [],
    lives: 3.0, shield: 2.0, seed: seed }

// A fixed seed keeps runs reproducible (swap in Effect.random for variety).
let init = freshRun(Random.seed(0.42))

// ---------- input ----------
// ALL controls are held LEVELS read from `sampledInput` once per tick —
// fire included: holding Space fires at the shared cooldown's rate, the
// same rule the bot (and any remote peer) plays under. This sampled
// record is precisely what a transport forwards as a `Protocol.Steer`.
let holds = (k: Key.t, keys: List<Key.t>): bool =>
  keys |> List.any((h) => h == k)

let sampledInput = (m, snap: Input.snapshot) =>
  let keys = snap.heldKeys in
  let left = holds(Key.Left, keys) || holds(Key.A, keys) in
  let right = holds(Key.Right, keys) || holds(Key.D, keys) in
  let turn = (if left then 1.0 else 0.0) - (if right then 1.0 else 0.0) in
  { m with intent: { turn: turn,
                     thrust: holds(Key.Up, keys) || holds(Key.W, keys),
                     fire: holds(Key.Space, keys) } }

let input = (m, key: Key.t, isDown: bool) =>
  match m.phase with
  | Playing => m
  | GameOver =>
    (if isDown && (key == Key.R || key == Key.Enter)
     then freshRun(m.seed)
     else m)

// The bot's steering lives in bot.fun (`Bot.intent` — nearest rock, aim,
// thrust, hold fire); here it is just another peer whose Intent arrives
// through the same wire path as yours.
let botSteer = (m) =>
  match Server.pilotOf(m.botPid, m.world) with
  | Option.None => Server.coast
  | Option.Some(p) => Bot.intent(p.ship, m.world.rocks)

// ---------- simulation ----------
// One tick: both intents arrive as Steers keyed by their connection pid,
// the world advances once, and my new ship state feeds the ghost buffer.
let tickPlaying = (m, dt: float) =>
  let world =
    m.world
      |> Server.recv(m.myPid, Protocol.Steer(m.intent))
      |> Server.recv(m.botPid, Protocol.Steer(botSteer(m)))
      |> Server.step(dt) in
  match Server.pilotOf(m.myPid, world) with
  | Option.None => { m with world: world }    // unreachable: step never drops pilots
  | Option.Some(me) =>
    let history = Protocol.pushHistory(me.ship, m.history) in
    let shield = Math.max(0.0, m.shield - dt) in
    let hit =
      shield == 0.0
        && (world.rocks |> List.any((r) => Server.rockHitsShip(me.ship, r))) in
    if not hit then
      { m with world: world, history: history, shield: shield }
    else if m.lives <= 1.0 then
      // Out of ships. My pilot coasts (and stops rendering); the bot keeps
      // clearing rocks behind the game-over card.
      { m with phase: GameOver, lives: 0.0, history: [],
               world: world |> Server.recv(m.myPid, Protocol.Steer(Server.coast)) }
    else
      // Hit: lose a life; the "server" re-centers my ship behind a fresh
      // shield, and the ghost buffer restarts with it.
      { m with world: Server.respawn(m.myPid, world), history: [],
               lives: m.lives - 1.0, shield: 2.5 }

let tick = (m, dt: float, tts: float) =>
  match m.phase with
  | Playing => tickPlaying(m, dt)
  | GameOver =>
    // The world keeps running behind the card — the bot is still a live
    // peer, steering and firing through the same wire path.
    { m with world: m.world
                      |> Server.recv(m.botPid, Protocol.Steer(botSteer(m)))
                      |> Server.step(dt) }

// ---------- wrap-aware placement ----------
// Collision (Protocol.dist2) is wrap-aware, so rendering must be too: an
// entity within its radius of an arena edge is drawn AGAIN on the opposite
// side (the arcade standard) — nothing can hit you from off-screen.
let wrapOffsets = (rad: float, v: float, limit: float): List<float> =>
  if v > limit - rad then [0.0, 0.0 - limit * 2.0]
  else if v < rad - limit then [0.0, limit * 2.0]
  else [0.0]

let wrapPlace = (rad: float, x: float, y: float, sprite: Sprite.t): List<Sprite.t> =>
  wrapOffsets(rad, x, Protocol.halfW) |> List.concatMap((ox) =>
    wrapOffsets(rad, y, Protocol.halfH) |> List.map((oy) =>
      sprite |> Sprite.move(x + ox, y + oy)))

// ---------- rendering ----------
// The ship triangle, nose up (+y at angle 0). Sprite.polygon points are
// used verbatim, so building it around the origin lets Sprite.rotate/move
// place it — the same convention Server.stepShip integrates in.
let nose = { x: 0.0, y: 1.4 }
let tailL = { x: -0.9, y: -1.0 }
let tailR = { x: 0.9, y: -1.0 }

let shipBody = (color: Color.t, thrusting: bool, tts: float) =>
  let flame =
    if thrusting && Math.sin(tts * 30.0) > -0.4 then
      [Sprite.polygon(pink, [{ x: -0.4, y: -1.0 }, { x: 0.4, y: -1.0 },
                             { x: 0.0, y: -1.9 }])]
    else [] in
  Sprite.group([Sprite.polygon(color, [nose, tailL, tailR]), ..flame])

// My ship blinks behind the respawn shield (and hides on game over); the
// bot draws whenever alive. 1.9 covers the triangle plus its flame, so a
// wrap copy appears as soon as any part of a ship crosses an edge.
let shipReach = 1.9

let shipSprites = (m, tts: float) =>
  let showMine =
    (match m.phase with
     | GameOver => false
     | Playing => not (m.shield > 0.001 && Math.sin(tts * 18.0) <= 0.0)) in
  m.world.pilots |> List.concatMap((p) =>
    let mine = p.ship.pid == m.myPid in
    if mine && not showMine then []
    else
      shipBody((if mine then cyan else p2), p.intent.thrust, tts)
        |> Sprite.rotate(Angle.radians(p.ship.angle))
        |> wrapPlace(shipReach, p.ship.x, p.ship.y))

// THE CONFIRMED GHOST: the same triangle as an outline (three lines —
// there is no unfilled polygon), always faint, drawn at the ring buffer's
// OLDEST entry: where the server's confirmation of my ship would currently
// be, rttTicks behind the prediction you steer. The "confirmed" label only
// appears while the ghost is visibly separated from the predicted ship, so
// it never reads as the ship's nametag.
let ghostOutline =
  Sprite.group([
    Sprite.line(cyan, 0.12, nose, tailL),
    Sprite.line(cyan, 0.12, tailL, tailR),
    Sprite.line(cyan, 0.12, tailR, nose),
  ])

let ghostSprites = (m) =>
  match m.phase with
  | GameOver => []    // no prediction to confirm — the buffer was cleared
  | Playing =>
    (match List.nth(Protocol.rttTicks, m.history) with
     | Option.None => []    // buffer still filling (right after a respawn)
     | Option.Some(g) =>
       let separation =
         (match List.head(m.history) with
          | Option.None => 0.0
          | Option.Some(now) => Math.sqrt(Protocol.dist2(now.x, now.y, g.x, g.y))) in
       let label =
         if separation > 0.8 then
           [Sprite.text(cyan, 0.7, "confirmed") |> Sprite.move(g.x, g.y - 2.2)]
         else [] in
       [Sprite.group([
          ghostOutline
            |> Sprite.rotate(Angle.radians(g.angle))
            |> Sprite.move(g.x, g.y),
          ..label]) |> Sprite.fade(0.25)])

// Rocks are filled circles with a darker core so the size classes read.
let rockSprites = (r) =>
  let rad = Protocol.radiusOf(r.size) in
  Sprite.group([
    Sprite.circle(rockColor, rad),
    Sprite.circle(rockCore, rad * 0.55),
  ]) |> wrapPlace(rad, r.x, r.y)

let bulletSprites = (b) =>
  Sprite.circle(bulletColor, 0.18) |> wrapPlace(0.18, b.x, b.y)

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
  let myPoints =
    (match Server.pilotOf(m.myPid, m.world) with
     | Option.None => 0.0
     | Option.Some(p) => p.points) in
  Sprite.group([
    hudLine(bulletColor, Protocol.halfH - 1.2,
            Text.concat("SCORE ", Text.fixed(myPoints, 0.0))),
    hudLine(cyan, Protocol.halfH - 2.6,
            Text.concat("SHIPS ", Text.fixed(m.lives, 0.0))),
  ])

let gameOverCard = () =>
  Sprite.group([
    Sprite.text(pink, 2.4, "GAME OVER") |> Sprite.move(0.0, 1.5),
    Sprite.text(cyan, 1.0, "PRESS R TO RESTART") |> Sprite.move(0.0, -1.0),
  ])

let draw = (m, tts: float) =>
  let card = (match m.phase with | GameOver => [gameOverCard()] | Playing => []) in
  Frame.create2D(
    Camera2D.create(Protocol.halfW * 2.0, Protocol.halfH * 2.0),
    Sprite.group(
      stars
        |> List.append(m.world.rocks |> List.concatMap(rockSprites))
        |> List.append(m.world.bullets |> List.concatMap(bulletSprites))
        |> List.append(ghostSprites(m))          // ghost under the live ships
        |> List.append(shipSprites(m, tts))
        |> List.append([hud(m)])
        |> List.append(card)))
    |> Frame.withClearColor(bg)

// The pure logic lives (and is `expect`-tested) in protocol.fun,
// server.fun, and bot.fun — run `functor -d examples/asteroids-mp test`.
