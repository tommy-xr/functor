// gallery: Neon Breakout — a polished, deterministic 2D brick breaker built from pure Sprite primitives.
// gallery-controls: A/D or ←/→ move · pointer steers · Space/click launches and restarts

type Phase =
  | Ready
  | Playing
  | Won
  | GameOver

type Brick = {
  id: float,
  x: float,
  y: float,
  row: float,
}

type Ball = {
  x: float,
  y: float,
  vx: float,
  vy: float,
}

type Model = {
  phase: Phase,
  paddleX: float,
  moveAxis: float,
  pointerActive: bool,
  ball: Ball,
  bricks: List<Brick>,
  score: float,
  lives: float,
}

let worldWidth = 32.0
let worldHeight = 24.0
let halfHeight = worldHeight / 2.0
let paddleY = -9.25
let paddleWidth = 5.2
let paddleHeight = 0.65
let paddleSpeed = 18.0
let ballRadius = 0.42
let courtEdge = 14.85
let playfieldTop = 9.9
let playfieldBottom = -10.2
let sideWallHeight = playfieldTop - playfieldBottom
let sideWallY = (playfieldTop + playfieldBottom) / 2.0
let paddleLimit = courtEdge - (paddleWidth + 0.35) / 2.0
let brickWidth = 3.35
let brickHeight = 1.05
let brickGap = 0.38
let columns = 8.0
let rows = 5.0
let camera2d = Camera2D.create(worldWidth, worldHeight)

let clamp = (lo: float, hi: float, value: float): float =>
  value |> Math.clamp(lo, hi)

let makeBricks = (): List<Brick> =>
  List.range(columns * rows)
    |> List.map((id) =>
      let row = Math.floor(id / columns) in
      let col = Math.mod(id, columns) in
      {
        id: id,
        x: -13.0 + col * (brickWidth + brickGap),
        y: 8.7 - row * (brickHeight + brickGap),
        row: row,
      })

let serveBall = (paddleX: float): Ball =>
  { x: paddleX, y: paddleY + 1.05, vx: 7.2, vy: 10.8 }

let freshGame = (): Model =>
  {
    phase: Ready,
    paddleX: 0.0,
    moveAxis: 0.0,
    pointerActive: false,
    ball: serveBall(0.0),
    bricks: makeBricks(),
    score: 0.0,
    lives: 3.0,
  }

let init: Model = freshGame()

let keyHeld = (key: Key.t, snapshot: Input.snapshot): bool =>
  snapshot.heldKeys |> List.any((held) => held == key)

let launchOrRestart = (model: Model): Model =>
  match model.phase with
  | Ready => { model with phase: Playing }
  | Playing => model
  | Won => { freshGame() with phase: Playing }
  | GameOver => { freshGame() with phase: Playing }

let input = (model: Model, key: Key.t, isDown: bool): Model =>
  if isDown && key == Key.Space then launchOrRestart(model) else model

let mouseMove = (model: Model, x: float, y: float): Model =>
  { model with pointerActive: true }

let mouseButton = (model: Model, button: Mouse.t, isDown: bool): Model =>
  if isDown && button == Mouse.Left then launchOrRestart(model) else model

let sampledInput = (model: Model, snapshot: Input.snapshot): Model =>
  let left =
    keyHeld(Key.A, snapshot) || keyHeld(Key.Left, snapshot) in
  let right =
    keyHeld(Key.D, snapshot) || keyHeld(Key.Right, snapshot) in
  let axis =
    (if right then 1.0 else 0.0) - (if left then 1.0 else 0.0) in
  let pointerX =
    Camera2D.toWorld(snapshot.mouse, camera2d)
      |> Option.map((p) => p.x)
      |> Option.defaultValue(model.paddleX) in
  let usePointer = model.pointerActive && axis == 0.0 in
  {
    model with
      moveAxis: axis,
      pointerActive: usePointer,
      paddleX:
        if usePointer
        then clamp(0.0 - paddleLimit, paddleLimit, pointerX)
        else model.paddleX
  }

let overlaps = (
  ball: Ball,
  x: float,
  y: float,
  width: float,
  height: float
): bool =>
  ball.x + ballRadius >= x - width / 2.0
    && ball.x - ballRadius <= x + width / 2.0
    && ball.y + ballRadius >= y - height / 2.0
    && ball.y - ballRadius <= y + height / 2.0

let stepWalls = (ball: Ball, dt: float): Ball =>
  let moved = {
    ball with
      x: ball.x + ball.vx * dt,
      y: ball.y + ball.vy * dt
  } in
  let hitLeft = moved.x - ballRadius < 0.0 - courtEdge in
  let hitRight = moved.x + ballRadius > courtEdge in
  let wallBall =
    if hitLeft then
      { moved with x: 0.0 - courtEdge + ballRadius, vx: Math.abs(moved.vx) }
    else if hitRight then
      { moved with x: courtEdge - ballRadius, vx: 0.0 - Math.abs(moved.vx) }
    else moved in
  if wallBall.y + ballRadius > playfieldTop then
    {
      wallBall with
        y: playfieldTop - ballRadius,
        vy: 0.0 - Math.abs(wallBall.vy)
    }
  else wallBall

let stepPaddle = (paddleX: float, ball: Ball): Ball =>
  if ball.vy < 0.0
    && overlaps(ball, paddleX, paddleY, paddleWidth, paddleHeight)
  then
    let impact = (ball.x - paddleX) / (paddleWidth / 2.0) in
    {
      ball with
        y: paddleY + paddleHeight / 2.0 + ballRadius,
        vx: clamp(-11.5, 11.5, ball.vx * 0.35 + impact * 10.0),
        vy: Math.abs(ball.vy)
    }
  else ball

let stepBricks = (
  ball: Ball,
  bricks: List<Brick>
): (Ball, List<Brick>, float) =>
  match bricks
    |> List.find((brick) =>
      overlaps(ball, brick.x, brick.y, brickWidth, brickHeight))
  with
  | Option.None => (ball, bricks, 0.0)
  | Option.Some(brick) =>
    let remaining =
      bricks |> List.filter((candidate) => candidate.id != brick.id) in
    let overlapX =
      brickWidth / 2.0 + ballRadius - Math.abs(ball.x - brick.x) in
    let overlapY =
      brickHeight / 2.0 + ballRadius - Math.abs(ball.y - brick.y) in
    let bounced =
      if overlapX < overlapY
      then { ball with vx: 0.0 - ball.vx }
      else { ball with vy: 0.0 - ball.vy } in
    (bounced, remaining, 1.0)

let tick = (model: Model, dt: float, tts: float): Model =>
  let nextPaddle =
    if model.pointerActive then model.paddleX
    else
      model.paddleX + model.moveAxis * paddleSpeed * dt
        |> clamp(0.0 - paddleLimit, paddleLimit) in
  match model.phase with
  | Ready =>
    {
      model with
        paddleX: nextPaddle,
        ball: serveBall(nextPaddle)
    }
  | Won => { model with paddleX: nextPaddle }
  | GameOver => { model with paddleX: nextPaddle }
  | Playing =>
    let moved = stepWalls(model.ball, dt) in
    let paddled = stepPaddle(nextPaddle, moved) in
    let (nextBall, nextBricks, hitCount) =
      stepBricks(paddled, model.bricks) in
    let nextScore = model.score + hitCount * 100.0 in
    if List.isEmpty(nextBricks) then
      {
        model with
          phase: Won,
          paddleX: nextPaddle,
          ball: nextBall,
          bricks: nextBricks,
          score: nextScore
      }
    else if nextBall.y + ballRadius < -halfHeight then
      let nextLives = model.lives - 1.0 in
      if nextLives <= 0.0 then
        {
          model with
            phase: GameOver,
            paddleX: nextPaddle,
            ball: serveBall(nextPaddle),
            lives: 0.0
        }
      else
        {
          model with
            phase: Ready,
            paddleX: nextPaddle,
            ball: serveBall(nextPaddle),
            lives: nextLives
        }
    else
      {
        model with
          paddleX: nextPaddle,
          ball: nextBall,
          bricks: nextBricks,
          score: nextScore
      }

let navy = Color.rgb(0.018, 0.025, 0.075)
let panel = Color.rgb(0.035, 0.055, 0.13)
let cyan = Color.rgb(0.18, 0.93, 1.0)
let white = Color.rgb(0.9, 0.98, 1.0)
let muted = Color.rgb(0.36, 0.52, 0.66)
let pink = Color.rgb(1.0, 0.2, 0.62)
let orange = Color.rgb(1.0, 0.48, 0.16)
let yellow = Color.rgb(1.0, 0.83, 0.2)
let lime = Color.rgb(0.35, 1.0, 0.52)
let violet = Color.rgb(0.58, 0.35, 1.0)

let brickColor = (row: float): Color.t =>
  if row == 0.0 then pink
  else if row == 1.0 then orange
  else if row == 2.0 then yellow
  else if row == 3.0 then lime
  else violet

let drawBrick = (brick: Brick): Sprite.t =>
  Sprite.group([
    Sprite.rectangle(Color.rgb(0.008, 0.014, 0.04), brickWidth + 0.16, brickHeight + 0.16),
    Sprite.rectangle(brickColor(brick.row), brickWidth, brickHeight),
    Sprite.rectangle(white, brickWidth - 0.25, 0.12)
      |> Sprite.fade(0.28)
      |> Sprite.moveY(0.3),
  ])
    |> Sprite.move(brick.x, brick.y)

let drawStars = (): List<Sprite.t> =>
  List.range(22.0)
    |> List.map((i) =>
      let x = -14.8 + Math.mod(i * 7.3, 29.6) in
      let y = -8.0 + Math.mod(i * 4.7, 18.0) in
      Sprite.circle(cyan, if Math.mod(i, 3.0) == 0.0 then 0.06 else 0.035)
        |> Sprite.fade(0.28)
        |> Sprite.move(x, y))

let drawLives = (lives: float): List<Sprite.t> =>
  List.range(lives)
    |> List.map((i) =>
      Sprite.circle(pink, 0.22)
        |> Sprite.move(12.0 + i * 0.62, 10.7))

let phaseLabel = (phase: Phase): string =>
  match phase with
  | Ready => "SPACE OR CLICK TO LAUNCH"
  | Playing => ""
  | Won => "BOARD CLEARED!  SPACE TO RESTART"
  | GameOver => "GAME OVER  //  SPACE TO RETRY"

let draw = (model: Model, tts: float): Frame.t =>
  let bricks = model.bricks |> List.map(drawBrick) in
  let stars = drawStars() in
  let lives = drawLives(model.lives) in
  let scoreText = Text.fixed(model.score, 0.0) in
  let scoreSize = 0.78 in
  let scoreWidth = Sprite.measure(scoreSize, scoreText).width in
  let hud = Sprite.group([
    Sprite.text(cyan, 0.55, "SCORE")
      |> Sprite.move(-13.4, 10.85),
    Sprite.text(white, scoreSize, scoreText)
      |> Sprite.move(-11.7 + scoreWidth / 2.0, 10.77),
    Sprite.text(muted, 0.5, $"BRICKS {List.length(model.bricks)}")
      |> Sprite.move(0.0, 10.82),
    Sprite.text(cyan, 0.55, "LIVES")
      |> Sprite.move(9.7, 10.85),
  ]) in
  let paddle = Sprite.group([
    Sprite.rectangle(Color.rgb(0.0, 0.0, 0.02), paddleWidth + 0.35, paddleHeight + 0.3)
      |> Sprite.fade(0.65)
      |> Sprite.moveY(-0.16),
    Sprite.rectangle(cyan, paddleWidth, paddleHeight),
    Sprite.rectangle(white, paddleWidth - 0.45, 0.12)
      |> Sprite.fade(0.55)
      |> Sprite.moveY(0.18),
  ])
    |> Sprite.move(model.paddleX, paddleY) in
  let ball = Sprite.group([
    Sprite.circle(cyan, ballRadius * 1.9) |> Sprite.fade(0.12),
    Sprite.circle(cyan, ballRadius),
    Sprite.circle(white, ballRadius * 0.42) |> Sprite.move(-0.1, 0.12),
  ])
    |> Sprite.move(model.ball.x, model.ball.y) in
  let prompt =
    if model.phase == Playing then Sprite.blank()
    else
      Sprite.group([
        Sprite.rectangle(panel, 23.0, 1.8) |> Sprite.fade(0.96),
        Sprite.rectangle(cyan, 22.6, 0.06) |> Sprite.moveY(0.87),
        Sprite.text(white, 0.62, phaseLabel(model.phase)),
      ])
        |> Sprite.moveY(-2.0) in
  Frame.create2D(
    camera2d,
    Sprite.group([
      Sprite.rectangle(navy, worldWidth, worldHeight),
      Sprite.rectangle(panel, 30.5, 21.0),
      Sprite.rectangle(navy, 29.8, 20.3),
      Sprite.group(stars),
      Sprite.rectangle(cyan, 0.09, sideWallHeight)
        |> Sprite.fade(0.55)
        |> Sprite.move(-14.85, sideWallY),
      Sprite.rectangle(pink, 0.09, sideWallHeight)
        |> Sprite.fade(0.55)
        |> Sprite.move(14.85, sideWallY),
      Sprite.rectangle(cyan, 29.8, 0.09)
        |> Sprite.fade(0.38)
        |> Sprite.moveY(playfieldTop),
      Sprite.group(bricks),
      paddle,
      ball,
      hud,
      Sprite.group(lives),
      prompt,
    ]))

expect List.length(makeBricks()) == 40.0
expect (launchOrRestart(init)).phase == Playing
expect (
  let won = { freshGame() with phase: Won, score: 900.0 } in
  let restarted = launchOrRestart(won) in
  restarted.phase == Playing
    && restarted.score == 0.0
    && restarted.lives == 3.0
    && List.length(restarted.bricks) == 40.0
    && launchOrRestart(restarted) == restarted
)
expect overlaps({ x: 0.0, y: 0.0, vx: 0.0, vy: 1.0 }, 0.0, 0.0, 2.0, 1.0)
expect (
  let bounced =
    stepPaddle(
      0.0,
      { x: 0.0, y: paddleY, vx: 0.0, vy: -4.0 }) in
  bounced.vy == 4.0
)
expect (
  let bounced =
    stepWalls(
      { x: courtEdge, y: 0.0, vx: 3.0, vy: 0.0 },
      0.0) in
  bounced.vx == -3.0
)
expect (
  let missed =
    {
      freshGame() with
        phase: Playing,
        ball: { x: 0.0, y: -13.0, vx: 0.0, vy: -1.0 },
        lives: 2.0
    } in
  let next = tick(missed, 0.0, 0.0) in
  next.phase == Ready && next.lives == 1.0
)
expect (
  let missed =
    {
      freshGame() with
        phase: Playing,
        ball: { x: 0.0, y: -13.0, vx: 0.0, vy: -1.0 },
        lives: 1.0
    } in
  let next = tick(missed, 0.0, 0.0) in
  next.phase == GameOver && next.lives == 0.0
)
expect (
  let lastBrick = { id: 0.0, x: 0.0, y: 0.0, row: 0.0 } in
  let almostWon =
    {
      freshGame() with
        phase: Playing,
        ball: { x: 0.0, y: 0.0, vx: 0.0, vy: 1.0 },
        bricks: [lastBrick]
    } in
  let won = tick(almostWon, 0.0, 0.0) in
  won.phase == Won
    && won.score == 100.0
    && List.isEmpty(won.bricks)
)
expect (
  let (ball, remaining, hitCount) =
    stepBricks(
      { x: 11.245, y: 2.98, vx: 2.0, vy: 10.0 },
      makeBricks()) in
  hitCount == 1.0
    && List.length(remaining) == 39.0
    && ball.vx == -2.0
    && ball.vy == 10.0
)
