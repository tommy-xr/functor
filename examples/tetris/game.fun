// gallery: NEON STACK — a deterministic falling-block puzzle with ghost pieces, wall kicks, line-clear flashes, scoring, and increasing gravity.
// gallery-controls: A/D or arrows move · W/Up rotate clockwise · Z rotate counter-clockwise · S/Down soft drop · Space hard drop · R restart

// The board is plain data: a list of occupied { x, y, kind } cells.
// No physics world or hidden mutable grid is involved.
let boardWidth = 10.0
let boardHeight = 20.0
let boardLeft = -8.0
let boardBottom = -10.0

let dark = Color.rgb(0.025, 0.03, 0.09)
let well = Color.rgb(0.055, 0.065, 0.15)
let gridColor = Color.rgb(0.12, 0.14, 0.26)
let ink = Color.rgb(0.88, 0.94, 1.0)
let cyan = Color.rgb(0.15, 0.9, 1.0)
let pink = Color.rgb(1.0, 0.2, 0.62)
let white = Color.rgb(1.0, 1.0, 1.0)

let kindColor = (kind) =>
  if kind == 0.0 then Color.rgb(0.1, 0.9, 1.0)
  else if kind == 1.0 then Color.rgb(1.0, 0.82, 0.18)
  else if kind == 2.0 then Color.rgb(0.75, 0.3, 1.0)
  else if kind == 3.0 then Color.rgb(0.2, 0.95, 0.48)
  else if kind == 4.0 then Color.rgb(1.0, 0.22, 0.36)
  else if kind == 5.0 then Color.rgb(0.25, 0.42, 1.0)
  else Color.rgb(1.0, 0.52, 0.16)

// Seven deterministic pieces. Rotation is reduced modulo four, and every
// offset remains integral so collision checks are exact structural comparisons.
let pieceOffsets = (kind, rotation) =>
  let r = Math.mod(rotation, 4.0) in
  if kind == 0.0 then
    if r == 0.0 || r == 2.0 then
      [{ x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 2.0, y: 0.0 }]
    else
      [{ x: 0.0, y: -1.0 }, { x: 0.0, y: 0.0 }, { x: 0.0, y: 1.0 }, { x: 0.0, y: 2.0 }]
  else if kind == 1.0 then
    [{ x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 0.0, y: 1.0 }, { x: 1.0, y: 1.0 }]
  else if kind == 2.0 then
    if r == 0.0 then
      [{ x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 0.0, y: 1.0 }]
    else if r == 1.0 then
      [{ x: 0.0, y: -1.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 0.0, y: 1.0 }]
    else if r == 2.0 then
      [{ x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 0.0, y: -1.0 }]
    else
      [{ x: 0.0, y: -1.0 }, { x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 0.0, y: 1.0 }]
  else if kind == 3.0 then
    if r == 0.0 || r == 2.0 then
      [{ x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 0.0, y: 1.0 }, { x: 1.0, y: 1.0 }]
    else
      [{ x: 0.0, y: 1.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 1.0, y: -1.0 }]
  else if kind == 4.0 then
    if r == 0.0 || r == 2.0 then
      [{ x: -1.0, y: 1.0 }, { x: 0.0, y: 1.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }]
    else
      [{ x: 1.0, y: 1.0 }, { x: 1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 0.0, y: -1.0 }]
  else if kind == 5.0 then
    if r == 0.0 then
      [{ x: -1.0, y: 1.0 }, { x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }]
    else if r == 1.0 then
      [{ x: 0.0, y: 1.0 }, { x: 1.0, y: 1.0 }, { x: 0.0, y: 0.0 }, { x: 0.0, y: -1.0 }]
    else if r == 2.0 then
      [{ x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 1.0, y: -1.0 }]
    else
      [{ x: 0.0, y: 1.0 }, { x: 0.0, y: 0.0 }, { x: -1.0, y: -1.0 }, { x: 0.0, y: -1.0 }]
  else
    if r == 0.0 then
      [{ x: 1.0, y: 1.0 }, { x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }]
    else if r == 1.0 then
      [{ x: 0.0, y: 1.0 }, { x: 0.0, y: 0.0 }, { x: 0.0, y: -1.0 }, { x: 1.0, y: -1.0 }]
    else if r == 2.0 then
      [{ x: -1.0, y: 0.0 }, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: -1.0, y: -1.0 }]
    else
      [{ x: -1.0, y: 1.0 }, { x: 0.0, y: 1.0 }, { x: 0.0, y: 0.0 }, { x: 0.0, y: -1.0 }]

let cellsFor = (piece) =>
  pieceOffsets(piece.kind, piece.rotation)
  |> List.map((offset) => {
      x: piece.x + offset.x,
      y: piece.y + offset.y,
      kind: piece.kind
    })

let occupies = (board, x, y) =>
  board |> List.any((cell) => cell.x == x && cell.y == y)

let validPiece = (board, piece) =>
  cellsFor(piece)
  |> List.all((cell) =>
      cell.x >= 0.0 &&
      cell.x < boardWidth &&
      cell.y >= 0.0 &&
      cell.y < boardHeight &&
      not occupies(board, cell.x, cell.y))

// Multiplication by five permutes all seven kinds before repeating. Starting
// with O gives the opening well an immediately readable two-line solution.
let nextKind = (index) => Math.mod(index * 5.0 + 1.0, 7.0)

let spawned = (kind) => {
  kind: kind,
  rotation: 0.0,
  x: 4.0,
  y: 18.0
}

// A small deterministic opening stack makes the game readable immediately and
// creates useful decisions without requiring an asset or a prerecorded replay.
let openingBoard =
  List.grid(
    (row, col) => { x: col, y: row, kind: Math.mod(col + row * 3.0, 7.0) },
    4.0,
    boardWidth)
  |> List.flatten
  |> List.filter((cell) =>
      (cell.y == 0.0 && cell.x != 4.0 && cell.x != 5.0) ||
      (cell.y == 1.0 && cell.x != 4.0 && cell.x != 5.0) ||
      (cell.y == 2.0 && (cell.x == 1.0 || cell.x == 2.0 || cell.x == 6.0 || cell.x == 7.0)) ||
      (cell.y == 3.0 && (cell.x == 2.0 || cell.x == 7.0)))

let fresh = {
  board: openingBoard,
  active: spawned(nextKind(0.0)),
  pieceIndex: 0.0,
  score: 0.0,
  lines: 0.0,
  level: 0.0,
  fallTimer: 0.0,
  phase: "playing",
  clearTimer: 0.0,
  flashRows: [],
  heldKeys: []
}

let init = fresh

let tryMove = (model, dx, dy) =>
  let moved = { model.active with x: model.active.x + dx, y: model.active.y + dy } in
  if validPiece(model.board, moved) then { model with active: moved } else model

let tryRotate = (model, direction) =>
  let turned = { model.active with rotation: model.active.rotation + direction } in
  let kicks = [
    turned,
    { turned with x: turned.x - 1.0 },
    { turned with x: turned.x + 1.0 },
    { turned with x: turned.x - 2.0 },
    { turned with x: turned.x + 2.0 },
    { turned with y: turned.y + 1.0 }
  ] in
  match kicks |> List.find((piece) => validPiece(model.board, piece)) with
  | Option.Some(piece) => { model with active: piece }
  | Option.None => model

let completeRows = (board) =>
  List.range(boardHeight)
  |> List.filter((row) =>
      List.length(board |> List.filter((cell) => cell.y == row)) == boardWidth)

let beginLock = (model, extraScore) =>
  let joined = model.board |> List.append(cellsFor(model.active)) in
  let rows = completeRows(joined) in
  if List.isEmpty(rows) then
    let nextIndex = model.pieceIndex + 1.0 in
    let nextPiece = spawned(nextKind(nextIndex)) in
    if validPiece(joined, nextPiece) then
      { model with
          board: joined,
          active: nextPiece,
          pieceIndex: nextIndex,
          score: model.score + extraScore,
          fallTimer: 0.0 }
    else
      { model with
          board: joined,
          active: nextPiece,
          score: model.score + extraScore,
          phase: "gameover",
          fallTimer: 0.0 }
  else
    { model with
        board: joined,
        score: model.score + extraScore,
        phase: "clearing",
        clearTimer: 0.32,
        flashRows: rows,
        fallTimer: 0.0 }

let clearAward = (count) =>
  if count == 1.0 then 100.0
  else if count == 2.0 then 300.0
  else if count == 3.0 then 500.0
  else 800.0

let finishClear = (model) =>
  let count = List.length(model.flashRows) in
  let collapsed =
    model.board
    |> List.filter((cell) => not (model.flashRows |> List.any((row) => row == cell.y)))
    |> List.map((cell) =>
        let below = model.flashRows |> List.filter((row) => row < cell.y) |> List.length in
        { cell with y: cell.y - below }) in
  let nextIndex = model.pieceIndex + 1.0 in
  let nextPiece = spawned(nextKind(nextIndex)) in
  let newLines = model.lines + count in
  let newLevel = Math.floor(newLines / 10.0) in
  let scored = model.score + clearAward(count) * (model.level + 1.0) in
  if validPiece(collapsed, nextPiece) then
    { model with
        board: collapsed,
        active: nextPiece,
        pieceIndex: nextIndex,
        score: scored,
        lines: newLines,
        level: newLevel,
        phase: "playing",
        clearTimer: 0.0,
        flashRows: [] }
  else
    { model with
        board: collapsed,
        active: nextPiece,
        pieceIndex: nextIndex,
        score: scored,
        lines: newLines,
        level: newLevel,
        phase: "gameover",
        clearTimer: 0.0,
        flashRows: [] }

let dropDistance = (model) =>
  List.range(boardHeight)
  |> List.filter((distance) =>
      validPiece(
        model.board,
        { model.active with y: model.active.y - distance }))
  |> List.maximum

let hardDrop = (model) =>
  let distance = dropDistance(model) in
  let landed = { model with active: { model.active with y: model.active.y - distance } } in
  beginLock(landed, distance * 2.0)

let softDrop = (model) =>
  let moved = tryMove(model, 0.0, -1.0) in
  if moved.active.y != model.active.y
  then { moved with score: moved.score + 1.0, fallTimer: 0.0 }
  else beginLock(model, 0.0)

let keyHeld = (model, key) => model.heldKeys |> List.any((held) => held == key)

let handlePress = (model, key) =>
  if key == Key.R then fresh
  else if model.phase != "playing" then model
  else if key == Key.A || key == Key.Left then tryMove(model, -1.0, 0.0)
  else if key == Key.D || key == Key.Right then tryMove(model, 1.0, 0.0)
  else if key == Key.W || key == Key.Up then tryRotate(model, 1.0)
  else if key == Key.Z then tryRotate(model, -1.0)
  else if key == Key.S || key == Key.Down then softDrop(model)
  else if key == Key.Space then hardDrop(model)
  else model

// The held-key latch turns OS key repeat into one deterministic action per edge.
let input = (model, key, isDown) =>
  if isDown then
    if keyHeld(model, key) then model
    else
      let pressed = handlePress(model, key) in
      { pressed with heldKeys: pressed.heldKeys |> List.append([key]) }
  else
    { model with heldKeys: model.heldKeys |> List.filter((held) => held != key) }

let tick = (model, dt, tts) =>
  if model.phase == "gameover" then model
  else if model.phase == "clearing" then
    if model.clearTimer - dt <= 0.0
    then finishClear(model)
    else { model with clearTimer: model.clearTimer - dt }
  else
    let interval = Math.max(0.1, 0.72 - model.level * 0.055) in
    let elapsed = model.fallTimer + dt in
    if elapsed < interval then { model with fallTimer: elapsed }
    else
      let moved = tryMove(model, 0.0, -1.0) in
      if moved.active.y != model.active.y
      then { moved with fallTimer: elapsed - interval }
      else beginLock(model, 0.0)

let cellCenterX = (x) => boardLeft + 0.5 + x
let cellCenterY = (y) => boardBottom + 0.5 + y

let blockPicture = (kind, clearing) =>
  let color = if clearing then white else kindColor(kind) in
  Sprite.group([
    Sprite.square(Color.rgb(0.015, 0.02, 0.07), 0.94),
    Sprite.square(color, 0.82),
    Sprite.rectangle(white, 0.62, 0.08)
      |> Sprite.fade(if clearing then 0.9 else 0.28)
      |> Sprite.moveY(0.28),
    Sprite.rectangle(dark, 0.62, 0.07)
      |> Sprite.fade(0.32)
      |> Sprite.moveY(-0.29)
  ])

let drawCell = (model, cell) =>
  let clearing = model.flashRows |> List.any((row) => row == cell.y) in
  blockPicture(cell.kind, clearing)
  |> Sprite.move(cellCenterX(cell.x), cellCenterY(cell.y))

let ghostCells = (model) =>
  let distance = dropDistance(model) in
  cellsFor({ model.active with y: model.active.y - distance })

let drawGhost = (model, cell) =>
  Sprite.group([
    Sprite.square(kindColor(cell.kind), 0.82) |> Sprite.fade(0.22),
    Sprite.square(well, 0.62)
  ])
  |> Sprite.move(cellCenterX(cell.x), cellCenterY(cell.y))

let verticalGrid =
  List.range(boardWidth + 1.0)
  |> List.map((column) =>
      Sprite.rectangle(gridColor, 0.025, boardHeight)
      |> Sprite.move(boardLeft + column, 0.0))

let horizontalGrid =
  List.range(boardHeight + 1.0)
  |> List.map((row) =>
      Sprite.rectangle(gridColor, boardWidth, 0.025)
      |> Sprite.move(-3.0, boardBottom + row))

let label = (text, y) =>
  Sprite.text(Color.rgb(0.45, 0.55, 0.76), 0.44, text)
  |> Sprite.move(7.0, y)

let valueText = (text, y) =>
  Sprite.text(ink, 0.78, text)
  |> Sprite.move(7.0, y)

let controlText = (text, y) =>
  Sprite.text(Color.rgb(0.58, 0.65, 0.82), 0.34, text)
  |> Sprite.move(7.0, y)

let preview = (kind) =>
  pieceOffsets(kind, 0.0)
  |> List.map((cell) =>
      blockPicture(kind, false)
      |> Sprite.scale(0.78)
      |> Sprite.move(7.0 + cell.x * 0.78, 5.7 + cell.y * 0.78))
  |> Sprite.group

let statusOverlay = (model) =>
  if model.phase == "gameover" then
    Sprite.group([
      Sprite.rectangle(dark, 9.4, 4.4) |> Sprite.fade(0.92) |> Sprite.move(-3.0, 0.0),
      Sprite.text(pink, 0.92, "STACK OVER") |> Sprite.move(-3.0, 0.65),
      Sprite.text(ink, 0.4, "PRESS R TO RESTART") |> Sprite.move(-3.0, -0.65)
    ])
  else if model.phase == "clearing" then
    Sprite.text(white, 0.58, "LINE CLEAR")
    |> Sprite.fade(0.35 + 0.65 * (model.clearTimer / 0.32))
    |> Sprite.move(-3.0, 0.0)
  else Sprite.blank()

let draw = (model, tts) =>
  let playfield =
    Sprite.group([
      Sprite.rectangle(Color.rgb(0.11, 0.13, 0.3), 10.8, 20.8) |> Sprite.move(-3.0, 0.0),
      Sprite.rectangle(well, 10.2, 20.2) |> Sprite.move(-3.0, 0.0),
      Sprite.group(verticalGrid),
      Sprite.group(horizontalGrid),
      if model.phase == "playing"
      then model |> ghostCells |> List.map((cell) => drawGhost(model, cell)) |> Sprite.group
      else Sprite.blank(),
      model.board |> List.map((cell) => drawCell(model, cell)) |> Sprite.group,
      if model.phase == "playing"
      then cellsFor(model.active) |> List.map((cell) => drawCell(model, cell)) |> Sprite.group
      else Sprite.blank(),
      statusOverlay(model)
    ]) in
  let panel =
    Sprite.group([
      Sprite.rectangle(Color.rgb(0.045, 0.05, 0.13), 9.0, 20.8) |> Sprite.move(7.0, 0.0),
      Sprite.text(cyan, 0.92, "NEON") |> Sprite.move(7.0, 9.0),
      Sprite.text(pink, 0.92, "STACK") |> Sprite.move(7.0, 8.0),
      label("NEXT", 6.8),
      preview(nextKind(model.pieceIndex + 1.0)),
      label("SCORE", 3.5),
      valueText(Text.fixed(model.score, 0.0), 2.65),
      label("LINES", 1.35),
      valueText(Text.fixed(model.lines, 0.0), 0.5),
      label("LEVEL", -0.8),
      valueText(Text.fixed(model.level + 1.0, 0.0), -1.65),
      Sprite.rectangle(gridColor, 7.2, 0.04) |> Sprite.move(7.0, -2.8),
      label("CONTROLS", -3.55),
      controlText("A/D    LEFT / RIGHT", -4.25),
      controlText("W/UP   ROTATE CW", -4.85),
      controlText("Z      ROTATE CCW", -5.45),
      controlText("S/DN   SOFT DROP", -6.05),
      controlText("SPACE  HARD DROP", -6.65),
      controlText("R      RESTART", -7.25),
      Sprite.text(Color.rgb(0.25, 0.78, 0.92), 0.3, "PURE DATA  //  ZERO PHYSICS")
      |> Sprite.move(7.0, -9.25)
    ]) in
  Frame.create2D(
    Camera2D.create(32.0, 24.0),
    Sprite.group([
      Sprite.rectangle(dark, 32.0, 24.0),
      Sprite.circle(Color.rgb(0.08, 0.2, 0.42), 7.0) |> Sprite.fade(0.18) |> Sprite.move(-11.0, 8.0),
      Sprite.circle(Color.rgb(0.5, 0.05, 0.3), 6.0) |> Sprite.fade(0.14) |> Sprite.move(12.0, -8.0),
      playfield,
      panel
    ]))
