// client.fun — Netpong: authoritative Pong over a REAL wire.
//   functor -d examples/netpong run native --entry server   # the authority
//   functor -d examples/netpong run native                  # a paddle dials it
//   functor -d examples/netpong run native                  # …and a second
// W/S or ↑/↓ steer your paddle (and take it off autopilot), Space toggles
// autopilot back on, R asks for a rematch once someone has won. Nobody has to
// connect: with no clients the server plays itself, so the sample is its own
// attract mode.
//
// Netpong is the roles-as-FILES multiplayer reference. Each role is its own
// buffer — `client.fun` here, `server.fun` beside it — over a shared
// `protocol.fun` and a shared `game.fun` renderer, with functor.json naming
// the two entry FILES. `examples/orbs` is the other half of the pair: both of
// ITS roles are inline `module` blocks of one `game.fun`, so one edit reloads
// both atomically. Reach for orbs while the whole game fits in one buffer;
// reach for THIS shape once the roles outgrow it or want to deploy apart.
//
// What this file owns is presentation, not truth. `target` is the newest
// server snapshot; `render` is what you actually see. `tick` moves the local
// paddle IMMEDIATELY off local input (prediction — no round trip), eases it
// back toward the server's copy (reconciliation), and interpolates the remote
// paddle and the ball between the 20 Hz snapshots. Both are ordinary model
// data, so the inspector and time travel can read the divergence.

type ConnState = | Offline | Online(id: float)

type Model = {
  conn: ConnState,
  seat: float,             // which paddle the server handed us; -1 = unseated
  status: string,
  target: Protocol.Snapshot,   // the newest server truth
  render: Protocol.Snapshot,   // what is drawn: predicted + interpolated
  axis: float,             // held steering, -1/0/+1, echoed to the server
  autopilot: bool,
  intentSeq: float,        // OUR sequence, so the server drops stale intents
  sendClock: float,        // 20 Hz intent cadence
  lastSnapshotSeq: float,  // THEIRS, so we drop snapshots that arrive late
}

type Msg =
  | Joined(id: float)
  | Packet(id: float, wire: Protocol.Wire)
  | Dropped
  | ConnErr(text: string)

let init: Model = {
  conn: Offline, seat: -1.0, status: "CONNECTING",
  target: Protocol.initialSnapshot(), render: Protocol.initialSnapshot(),
  axis: 0.0, autopilot: true,
  intentSeq: 0.0, sendClock: 0.0, lastSnapshotSeq: -1.0,
}

let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => Joined(id)
  | Net.Data(id, wire) => Packet(id, wire)
  | Net.Message(_, _) => ConnErr("unexpected text frame")
  | Net.Disconnected(_) => Dropped
  | Net.Error(_, message) => ConnErr(message)

let update = (m: Model, msg: Msg) =>
  match msg with
  | Joined(id) => ({ m with conn: Online(id), status: "CONNECTED", sendClock: 0.0 },
                   Effect.sendMsg(id, Protocol.PaddleIntent(m.intentSeq, m.axis)))
  | Packet(_, wire) =>
      (match wire with
       | Protocol.Welcome(seat) => { m with seat: seat, status: "SYNCED" }
       // Snapshots are the whole world, so an out-of-order one is not a merge
       // problem — it is simply older truth. Compare sequence and drop it.
       | Protocol.State(s) =>
           if s.seq > m.lastSnapshotSeq
           then { m with target: s, lastSnapshotSeq: s.seq, status: "LIVE" }
           else m
       | _ => m)
  | Dropped => { m with conn: Offline, seat: -1.0, status: "RECONNECTING" }
  | ConnErr(message) => { m with conn: Offline, status: Text.concat("NET ERROR: ", message) }

// One declarative line is the whole transport. `Sub.connect` keeps the
// connection up: dial before the server exists, or restart the server
// underneath us, and it retries with bounded backoff — the failure arrives as
// an ordered `Net.Error`, then `Net.Connected` when it lands.
let subscriptions = (m: Model) => Sub.connect(Protocol.serverUrl, toMsg)

let keyHeld = (key: Key.t, snapshot: Input.snapshot): bool =>
  snapshot.heldKeys |> List.any((held) => held == key)

let sampledInput = (m: Model, sample: Input.snapshot): Model =>
  let up = keyHeld(Key.W, sample) || keyHeld(Key.Up, sample) in
  let down = keyHeld(Key.S, sample) || keyHeld(Key.Down, sample) in
  let toggleAuto = sample.pressedKeys |> List.any((key) => key == Key.Space) in
  let axis = (if up then 1.0 else 0.0) - (if down then 1.0 else 0.0) in
  let toggled = if toggleAuto then { m with autopilot: not m.autopilot } else m in
  if up || down then { toggled with axis: axis, autopilot: false }
  else if toggled.autopilot then toggled
  else { toggled with axis: 0.0 }

let input = (m: Model, key: Key.t, isDown: bool) =>
  if isDown && key == Key.R then
    match m.conn with
    | Online(id) => (m, Effect.sendMsg(id, Protocol.Rematch))
    | Offline => m
  else m

let lerp = (from: float, target: float, amount: float): float =>
  from + (target - from) * Math.clamp(0.0, 1.0, amount)

let autoAxis = (m: Model): float =>
  let ownY = if m.seat == 1.0 then m.render.rightY else m.render.leftY in
  let delta = m.target.ballY - ownY in
  if Math.abs(delta) < 0.32 then 0.0 else Math.sign(delta)

// The heart of the sample: `target` (server truth) -> `render` (what you see).
// OUR paddle is predicted — it moves this frame off local input, then eases
// toward the server's copy so a correction is a slide, not a snap. Everything
// else is interpolated toward the target. A phase change or a score means the
// world DISCONTINUED, so ball and trail snap instead of sweeping across the
// court through positions that never happened.
let smooth = (m: Model, dt: float, axis: float): Protocol.Snapshot =>
  let k = Math.clamp(0.0, 1.0, dt * 13.0) in
  let predictedLeft =
    if m.seat == 0.0
    then Math.clamp(0.0 - Protocol.paddleYLimit, Protocol.paddleYLimit,
               m.render.leftY + axis * Protocol.paddleSpeed * dt)
    else lerp(m.render.leftY, m.target.leftY, k) in
  let predictedRight =
    if m.seat == 1.0
    then Math.clamp(0.0 - Protocol.paddleYLimit, Protocol.paddleYLimit,
               m.render.rightY + axis * Protocol.paddleSpeed * dt)
    else lerp(m.render.rightY, m.target.rightY, k) in
  let continuous = m.render.phase == Protocol.Rally && m.target.phase == Protocol.Rally
    && m.render.leftScore == m.target.leftScore && m.render.rightScore == m.target.rightScore in
  { m.target with
      leftY: if m.seat == 0.0 then lerp(predictedLeft, m.target.leftY, dt * 2.2) else predictedLeft,
      rightY: if m.seat == 1.0 then lerp(predictedRight, m.target.rightY, dt * 2.2) else predictedRight,
      ballX: if continuous then lerp(m.render.ballX, m.target.ballX, k) else m.target.ballX,
      ballY: if continuous then lerp(m.render.ballY, m.target.ballY, k) else m.target.ballY,
      trail: if continuous then
               [Protocol.point(lerp(m.render.ballX, m.target.ballX, k),
                               lerp(m.render.ballY, m.target.ballY, k)), ..m.render.trail]
               |> List.take(15.0)
             else m.target.trail }

// Render every frame; send at 20 Hz. Subtracting the interval (rather than
// zeroing the clock) keeps the leftover, so the cadence doesn't drift with the
// frame rate — but while offline the clock is capped at one interval, or a
// long disconnect would burst a queue of intents the moment it reconnects.
let tick = (m: Model, dt: float, tts: float) =>
  let axis = if m.autopilot then autoAxis(m) else m.axis in
  let nextClock = m.sendClock + dt in
  let next = { m with render: smooth(m, dt, axis), axis: axis, sendClock: nextClock } in
  if nextClock >= 0.05 then
    match m.conn with
    | Online(id) =>
        let seq = m.intentSeq + 1.0 in
        ({ next with sendClock: nextClock - 0.05, intentSeq: seq },
         Effect.sendMsg(id, Protocol.PaddleIntent(seq, axis)))
    | Offline => { next with sendClock: Math.min(0.05, nextClock) }
  else next

let draw = (m: Model, tts: float): Frame.t => Game.view(m.render, m.status, m.seat, m.autopilot)

expect Math.abs(lerp(0.0, 10.0, 0.25) - 2.5) < 0.0001
expect (
  let m = { init with seat: 0.0, axis: 1.0, autopilot: false } in
  let predicted = smooth(m, 0.05, 1.0) in
  predicted.leftY > 0.4)
expect (
  let old = { Protocol.initialSnapshot() with phase: Protocol.Rally,
                                                   ballX: 12.0,
                                                   trail: [Protocol.point(11.0, 1.0)] } in
  let target = { Protocol.initialSnapshot() with phase: Protocol.Serving(1.0) } in
  let m = { init with render: old, target: target } in
  let reset = smooth(m, 0.05, 0.0) in
  reset.ballX == 0.0 && List.isEmpty(reset.trail))
