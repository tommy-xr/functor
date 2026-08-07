// server.fun — the authority: it owns the only real match.
//
// Paddle positions, ball kinematics, scores, serve timing, the win and the
// rematch all live HERE. A client never states a fact; it sends a sequenced
// PaddleIntent ("I am holding up"), and this file decides what that means.
// Every 50 ms the settled world is broadcast to every connection as one
// Snapshot — the whole world, not a diff, so a dropped packet costs nothing
// but a frame of smoothness.
//
// An unoccupied seat is played by `aiY`, which is why the server alone is an
// AI-vs-AI attract mode and one client is a game against the house. Both roles
// draw the same `Game.view`, so this window shows exactly the truth the
// clients are trying to converge on.

type Player = { cid: float, seat: float, axis: float, intentSeq: float }

type Model = {
  players: List<Player>,
  leftY: float,
  rightY: float,
  ballX: float,
  ballY: float,
  ballVx: float,
  ballVy: float,
  leftScore: float,
  rightScore: float,
  phase: Protocol.Phase,
  hitPulse: float,
  scorePulse: float,
  trail: List<Protocol.Point>,
  snapshotClock: float,
  snapshotSeq: float,
  serverTime: float,
}

type Msg =
  | Joined(cid: float)
  | Packet(cid: float, wire: Protocol.Wire)
  | Left(cid: float)
  | NetErr(text: string)

let fresh = (): Model => {
  players: [],
  leftY: 0.0, rightY: 0.0,
  ballX: 0.0, ballY: 0.0, ballVx: -9.0, ballVy: 4.2,
  leftScore: 0.0, rightScore: 0.0,
  phase: Protocol.Serving(2.2), hitPulse: 0.0, scorePulse: 0.0,
  trail: [], snapshotClock: 0.0, snapshotSeq: 0.0, serverTime: 0.0,
}

let init: Model = fresh()

let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(cid) => Joined(cid)
  | Net.Data(cid, wire) => Packet(cid, wire)
  | Net.Message(_, _) => NetErr("unexpected text frame")
  | Net.Disconnected(cid) => Left(cid)
  | Net.Error(_, message) => NetErr(message)

let playerForSeat = (seat: float, players: List<Player>): Option.t<Player> =>
  players |> List.find((p) => p.seat == seat)

// A fresh match, but NOT a fresh sequence: `snapshotSeq` and `serverTime`
// carry over, because the clients already connected reject anything that isn't
// newer than what they have. Resetting the counter would make the new match
// look stale and freeze every screen but this one.
let rematch = (m: Model): Model =>
  { fresh() with players: m.players, snapshotSeq: m.snapshotSeq, serverTime: m.serverTime }

let axisFor = (seat: float, players: List<Player>): Option.t<float> =>
  playerForSeat(seat, players) |> Option.map((p) => p.axis)

let update = (m: Model, msg: Msg) =>
  match msg with
  // Seats are assigned, never claimed: the first two connections get 0 and 1.
  // A third is accepted by the transport but takes no seat, gets no `Welcome`,
  // and is not in the broadcast list — deliberately the smallest possible
  // policy. Spectators would be one more list and one more send.
  | Joined(cid) =>
      let occupied0 = playerForSeat(0.0, m.players) |> Option.isSome in
      let occupied1 = playerForSeat(1.0, m.players) |> Option.isSome in
      let seat = if not occupied0 then 0.0 else if not occupied1 then 1.0 else -1.0 in
      if seat < 0.0 then m
      else
        let player: Player = { cid: cid, seat: seat, axis: 0.0, intentSeq: -1.0 } in
        ({ m with players: [player, ..m.players] },
         Effect.sendMsg(cid, Protocol.Welcome(seat)))
  | Packet(cid, wire) =>
      (match wire with
       // Keyed by the CONNECTION it arrived on, never by a seat in the
       // payload — so a client can only ever steer its own paddle. The
       // sequence drops intents that overtook each other in flight, and the
       // clamp means a doctored axis is still just -1..1.
       | Protocol.PaddleIntent(seq, axis) =>
           { m with players:
               m.players |> List.map((p) =>
                 if p.cid == cid && seq > p.intentSeq
                 then { p with axis: Math.clamp(-1.0, 1.0, axis), intentSeq: seq }
                 else p) }
       | Protocol.Rematch =>
           let senderIsPlayer = m.players |> List.any((p) => p.cid == cid) in
           if senderIsPlayer then
             (match m.phase with
              | Protocol.Won(_) => rematch(m)
              | _ => m)
           else m
       | _ => m)
  | Left(cid) => { m with players: m.players |> List.filter((p) => p.cid != cid) }
  | NetErr(_) => m

let subscriptions = (m: Model) => Sub.listen(Protocol.bind, toMsg)

let approach = (current: float, target: float, speed: float, dt: float): float =>
  let delta = target - current in
  current + Math.clamp(0.0 - speed * dt, speed * dt, delta)

let aiY = (ballY: float, current: float, dt: float): float =>
  approach(current, Math.clamp(0.0 - Protocol.paddleYLimit, Protocol.paddleYLimit, ballY),
           Protocol.paddleSpeed * 0.82, dt)

let movePaddle = (seat: float, current: float, ballY: float, players: List<Player>, dt: float): float =>
  match axisFor(seat, players) with
  | Option.Some(axis) =>
      Math.clamp(0.0 - Protocol.paddleYLimit, Protocol.paddleYLimit,
            current + axis * Protocol.paddleSpeed * dt)
  | Option.None => aiY(ballY, current, dt)

let servedBall = (leftScore: float, rightScore: float): (float, float) =>
  let sign = if Math.mod(leftScore + rightScore, 2.0) == 0.0 then -1.0 else 1.0 in
  (sign * (9.2 + (leftScore + rightScore) * 0.18),
   if Math.mod(leftScore + rightScore, 3.0) == 0.0 then 4.6 else -4.1)

// The model, minus everything a viewer does not need: velocities and the
// server clock stay here.
let snapshot = (m: Model): Protocol.Snapshot => {
  seq: m.snapshotSeq,
  leftY: m.leftY, rightY: m.rightY,
  ballX: m.ballX, ballY: m.ballY,
  leftScore: m.leftScore, rightScore: m.rightScore,
  phase: m.phase, hitPulse: m.hitPulse, scorePulse: m.scorePulse, trail: m.trail,
}

let withPoint = (winner: float, m: Model): Model =>
  let ls = m.leftScore + (if winner == 0.0 then 1.0 else 0.0) in
  let rs = m.rightScore + (if winner == 1.0 then 1.0 else 0.0) in
  let won = ls >= Protocol.winningScore || rs >= Protocol.winningScore in
  { m with
      leftScore: ls, rightScore: rs,
      ballX: 0.0, ballY: 0.0,
      phase: if won then Protocol.Won(winner) else Protocol.Serving(1.65),
      scorePulse: 1.0, hitPulse: 0.0, trail: [] }

let stepRally = (m: Model, dt: float): Model =>
  let x = m.ballX + m.ballVx * dt in
  let rawY = m.ballY + m.ballVy * dt in
  let wall = Math.abs(rawY) + Protocol.ballRadius > Protocol.courtHalfHeight in
  let y = if wall then Math.sign(rawY) * (Protocol.courtHalfHeight - Protocol.ballRadius) else rawY in
  let vy = if wall then 0.0 - m.ballVy else m.ballVy in
  let leftHit = m.ballVx < 0.0 && x - Protocol.ballRadius <= 0.0 - Protocol.paddleX
    && m.ballX > 0.0 - Protocol.paddleX
    && Math.abs(y - m.leftY) <= Protocol.paddleHalfHeight + Protocol.ballRadius in
  let rightHit = m.ballVx > 0.0 && x + Protocol.ballRadius >= Protocol.paddleX
    && m.ballX < Protocol.paddleX
    && Math.abs(y - m.rightY) <= Protocol.paddleHalfHeight + Protocol.ballRadius in
  let hit = leftHit || rightHit in
  let paddleY = if leftHit then m.leftY else m.rightY in
  let vx = if hit then (0.0 - m.ballVx) * 1.035 else m.ballVx in
  let hitVy = if hit then vy * 0.62 + (y - paddleY) * 4.2 else vy in
  let hitX = if leftHit then 0.0 - Protocol.paddleX + Protocol.ballRadius
             else if rightHit then Protocol.paddleX - Protocol.ballRadius else x in
  let moved = { m with
    ballX: hitX, ballY: y, ballVx: vx, ballVy: Math.clamp(-10.5, 10.5, hitVy),
    hitPulse: if hit then 1.0 else Math.max(0.0, m.hitPulse - dt * 3.8),
    scorePulse: Math.max(0.0, m.scorePulse - dt * 2.4),
    trail: [Protocol.point(hitX, y), ..m.trail] |> List.take(15.0) } in
  if hitX < 0.0 - Protocol.courtHalfWidth then withPoint(1.0, moved)
  else if hitX > Protocol.courtHalfWidth then withPoint(0.0, moved)
  else moved

// One pure step of the whole match. `dt` is BOUNDED, so a stalled frame (a
// breakpoint, a dragged window) advances the world by one step instead of
// tunnelling the ball through a paddle — the expect below pins exactly that.
let stepWorld = (dt: float, m: Model): Model =>
  let boundedDt = Math.clamp(0.0, 0.05, dt) in
  let leftY = movePaddle(0.0, m.leftY, m.ballY, m.players, boundedDt) in
  let rightY = movePaddle(1.0, m.rightY, m.ballY, m.players, boundedDt) in
  let base = { m with leftY: leftY, rightY: rightY,
                       serverTime: m.serverTime + boundedDt,
                       snapshotClock: m.snapshotClock + boundedDt } in
  match base.phase with
  | Protocol.Serving(seconds) =>
      if seconds - boundedDt <= 0.0 then
        let (vx, vy) = servedBall(base.leftScore, base.rightScore) in
        { base with phase: Protocol.Rally, ballVx: vx, ballVy: vy,
                    ballX: 0.0, ballY: 0.0, trail: [] }
      else { base with phase: Protocol.Serving(seconds - boundedDt),
                       hitPulse: Math.max(0.0, base.hitPulse - boundedDt * 3.8),
                       scorePulse: Math.max(0.0, base.scorePulse - boundedDt * 2.4) }
  | Protocol.Rally => stepRally(base, boundedDt)
  | Protocol.Won(_) => { base with hitPulse: Math.max(0.0, base.hitPulse - boundedDt * 3.8),
                                   scorePulse: Math.max(0.18, base.scorePulse - boundedDt * 1.2) }

// Simulate every frame, broadcast at 20 Hz — one `Effect.sendMsg` per
// connection, batched into a single effect. Like the client's send clock, the
// remainder is kept so the cadence doesn't drift with the frame rate.
let tick = (m: Model, dt: float, tts: float) =>
  let stepped = stepWorld(dt, m) in
  if stepped.snapshotClock >= 0.05 then
    let ready = { stepped with snapshotClock: stepped.snapshotClock - 0.05,
                               snapshotSeq: stepped.snapshotSeq + 1.0 } in
    let snap = snapshot(ready) in
    let sends = ready.players |> List.map((p) => Effect.sendMsg(p.cid, Protocol.State(snap))) in
    (ready, Effect.batch(sends))
  else stepped

// Seat -1 tells the shared renderer to draw the authority's banner instead of
// a player's HUD, so the status/autopilot arguments are a player's business.
let draw = (m: Model, tts: float): Frame.t => Game.view(snapshot(m), "", -1.0, false)

expect (
  let m = { fresh() with phase: Protocol.Rally, ballX: 0.0, ballY: 0.0,
                         ballVx: 10.0, ballVy: 0.0 } in
  let a = stepWorld(0.05, m) in
  let b = stepWorld(5.0, m) in
  a == b && Math.abs(a.ballX - 0.5) < 0.0001)
expect (
  let m = { fresh() with phase: Protocol.Rally,
                         ballX: 12.95, ballY: 6.0, ballVx: 5.0, ballVy: 0.0,
                         leftScore: 4.0 } in
  let scored = stepWorld(0.05, m) in
  scored.leftScore == 5.0 && scored.phase == Protocol.Won(0.0))
expect (
  let won = { fresh() with phase: Protocol.Won(1.0), leftScore: 2.0, rightScore: 5.0,
                           snapshotSeq: 220.0, serverTime: 18.0 } in
  let reset = rematch(won) in
  reset.leftScore == 0.0 && reset.rightScore == 0.0
    && reset.phase == Protocol.Serving(2.2)
    && reset.snapshotSeq == 220.0 && reset.serverTime == 18.0)
