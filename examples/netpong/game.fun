// gallery: Netpong — authoritative multiplayer Pong with prediction, interpolation, and neon AI attract mode.
// gallery-controls: W/S or ↑/↓ steer · Space toggles autopilot · R requests a rematch

// game.fun — the renderer both roles call, as `Game.view`.
//
// It is not an entry point: functor.json names client.fun and server.fun, and
// this file is just their shared sibling. It takes a plain
// `Protocol.Snapshot` and knows nothing about who produced it — which is why
// the server window and every client window draw the same court, and why a
// client can hand it PREDICTED state without the renderer caring.
//
// Asset-free 2D: every pixel below is a `Sprite` primitive.

let navy = Color.rgb(0.012, 0.018, 0.065)
let deep = Color.rgb(0.028, 0.035, 0.115)
let cyan = Color.rgb(0.12, 0.94, 1.0)
let pink = Color.rgb(1.0, 0.18, 0.68)
let white = Color.rgb(0.92, 0.98, 1.0)
let violet = Color.rgb(0.48, 0.25, 0.94)
let muted = Color.rgb(0.35, 0.48, 0.68)
let camera = Camera2D.create(32.0, 20.0)

let stars = (): List<Sprite.t> =>
  List.range(42.0)
  |> List.map((i) =>
    let x = -15.4 + Math.mod(i * 8.17, 30.8) in
    let y = -9.4 + Math.mod(i * 5.31, 18.8) in
    Sprite.circle(if Math.mod(i, 4.0) == 0.0 then pink else cyan,
                  if Math.mod(i, 3.0) == 0.0 then 0.055 else 0.032)
    |> Sprite.fade(0.2 + Math.mod(i, 5.0) * 0.055)
    |> Sprite.move(x, y))

let trailSprites = (trail: List<Protocol.Point>): List<Sprite.t> =>
  trail |> List.indexedMap((i, p) =>
    let size = Math.max(0.035, 0.27 - i * 0.017) in
    Sprite.circle(cyan, size)
    |> Sprite.fade(Math.max(0.03, 0.34 - i * 0.022))
    |> Sprite.move(p.x, p.y))

let paddle = (x: float, y: float, color: Color.t, pulse: float): Sprite.t =>
  let glow = 0.12 + pulse * 0.22 in
  Sprite.group([
    Sprite.rectangle(color, 0.78 + pulse * 0.18, 3.25 + pulse * 0.3)
      |> Sprite.fade(glow),
    Sprite.rectangle(color, 0.42, 2.9),
    Sprite.rectangle(white, 0.12, 2.35) |> Sprite.fade(0.72),
  ]) |> Sprite.move(x, y)

let scanLines = (): List<Sprite.t> =>
  List.range(16.0)
  |> List.map((i) =>
    Sprite.rectangle(violet, 25.8, 0.025)
    |> Sprite.fade(0.12)
    |> Sprite.moveY(-7.0 + i * 0.9))

let view = (s: Protocol.Snapshot, status: string, seat: float, autopilot: bool): Frame.t =>
  let phaseText = Protocol.phaseLabel(s.phase) in
  let pulse = Math.max(s.hitPulse, s.scorePulse) in
  let centerMarks =
    List.range(11.0)
    |> List.map((i) =>
      Sprite.rectangle(muted, 0.08, 0.6)
      |> Sprite.fade(0.42)
      |> Sprite.moveY(-5.0 + i)) in
  let ball = Sprite.group([
    Sprite.circle(cyan, 0.92 + pulse * 0.25) |> Sprite.fade(0.055 + pulse * 0.04),
    Sprite.circle(cyan, 0.55) |> Sprite.fade(0.18),
    Sprite.circle(white, Protocol.ballRadius),
    Sprite.circle(cyan, 0.12) |> Sprite.move(-0.1, 0.12),
  ]) |> Sprite.move(s.ballX, s.ballY) in
  let score = Sprite.group([
    Sprite.text(cyan, 1.5, Text.fixed(s.leftScore, 0.0)) |> Sprite.move(-4.2, 7.75),
    Sprite.text(muted, 0.52, "FIRST TO 5") |> Sprite.move(0.0, 7.8),
    Sprite.text(pink, 1.5, Text.fixed(s.rightScore, 0.0)) |> Sprite.move(4.2, 7.75),
  ]) in
  let banner =
    if phaseText == "" then Sprite.blank()
    else Sprite.group([
      Sprite.rectangle(deep, 11.5, 1.55) |> Sprite.fade(0.94),
      Sprite.rectangle(cyan, 11.1, 0.055) |> Sprite.moveY(0.73),
      Sprite.text(white, 0.66, phaseText),
      (match s.phase with
       | Protocol.Won(_) => Sprite.text(muted, 0.28, "PRESS R TO REMATCH") |> Sprite.moveY(-0.47)
       | _ => Sprite.blank()),
    ]) |> Sprite.moveY(-0.15) in
  let mode =
    if seat < 0.0 then "AUTHORITATIVE SERVER  //  AI ATTRACT"
    else $"SEAT {Text.fixed(seat + 1.0, 0.0)}  //  {if autopilot then "AUTOPILOT" else "MANUAL"}  //  {status}" in
  Frame.create2D(camera, Sprite.group([
    Sprite.rectangle(navy, 32.0, 20.0),
    Sprite.group(stars()),
    Sprite.rectangle(deep, 27.1, 15.0),
    Sprite.rectangle(navy, 26.3, 14.2),
    Sprite.group(scanLines()),
    Sprite.rectangle(cyan, 26.2, 0.08) |> Sprite.fade(0.55) |> Sprite.moveY(7.05),
    Sprite.rectangle(pink, 26.2, 0.08) |> Sprite.fade(0.55) |> Sprite.moveY(-7.05),
    Sprite.group(centerMarks),
    Sprite.group(trailSprites(s.trail)),
    paddle(0.0 - Protocol.paddleX, s.leftY, cyan, s.hitPulse),
    paddle(Protocol.paddleX, s.rightY, pink, s.hitPulse),
    ball,
    score,
    banner,
    Sprite.text(cyan, 0.7, "N E T P O N G") |> Sprite.moveY(9.0),
    Sprite.text(muted, 0.3, mode) |> Sprite.moveY(-8.65),
    Sprite.text(muted, 0.25, "SERVER TRUTH  //  CLIENT PREDICTION  //  SNAPSHOT INTERPOLATION")
      |> Sprite.moveY(-9.18),
    Sprite.rectangle(if s.scorePulse > 0.0 then pink else cyan, 31.0, 19.0)
      |> Sprite.fade(s.scorePulse * 0.09),
  ]))
