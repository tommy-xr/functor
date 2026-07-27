// A complete pointer-led Sprite game.
//
// Resize the window freely: Input.mouse carries its logical surface extent,
// and Camera2D.toWorld shares the renderer's aspect fit. The pointer therefore
// keeps landing on the same world-space card across letterboxing and Retina.

let camera =
  Camera2D.create(24.0, 13.5)
  |> Camera2D.at(2.0, 0.0)
  |> Camera2D.zoom(1.08)

let init = {
  pointer: Option.None,
  hover: 0.0 - 1.0,
  selected: 0.0 - 1.0
}

let hitCard = (point) =>
  let inCard = (cx, cy) =>
    point.x >= cx - 2.15 && point.x <= cx + 2.15 &&
    point.y >= cy - 1.65 && point.y <= cy + 1.65
  in
  if inCard(-4.5, 2.0) then 0.0
  else if inCard(1.0, 2.0) then 1.0
  else if inCard(6.5, 2.0) then 2.0
  else if inCard(-1.75, -2.0) then 3.0
  else if inCard(3.75, -2.0) then 4.0
  else 0.0 - 1.0

let sampledInput = (model, snapshot: Input.snapshot) =>
  let pointer = Camera2D.toWorld(snapshot.mouse, camera) in
  match pointer with
  | Option.Some(point) =>
      let hover = hitCard(point) in
      { model with
          pointer: pointer,
          hover: hover }
  | Option.None =>
      { model with
          pointer: Option.None,
          hover: 0.0 - 1.0 }

let mouseButton = (model, button, isDown) =>
  if button == Mouse.Left && isDown && model.hover >= 0.0
  then { model with selected: model.hover }
  else model

let tick = (model, dt, tts) => model

let bg = Color.rgb(0.025, 0.035, 0.09)
let panel = Color.rgb(0.075, 0.09, 0.18)
let panelHot = Color.rgb(0.11, 0.15, 0.28)
let ink = Color.rgb(0.91, 0.95, 1.0)
let dim = Color.rgb(0.48, 0.57, 0.74)
let cyan = Color.rgb(0.18, 0.88, 0.98)
let pink = Color.rgb(1.0, 0.28, 0.67)
let amber = Color.rgb(1.0, 0.72, 0.22)
let violet = Color.rgb(0.53, 0.38, 1.0)
let mint = Color.rgb(0.28, 0.96, 0.66)

let accent = (index) =>
  if index == 0.0 then cyan
  else if index == 1.0 then pink
  else if index == 2.0 then amber
  else if index == 3.0 then violet
  else mint

let title = (index) =>
  if index == 0.0 then "TIDAL"
  else if index == 1.0 then "BLOOM"
  else if index == 2.0 then "SOLAR"
  else if index == 3.0 then "DUSK"
  else "ORBIT"

let glyph = (index, color) =>
  if index == 0.0 then
    Sprite.group([
      Sprite.circle(color, 0.58),
      Sprite.circle(panel, 0.36)
    ])
  else if index == 1.0 then
    Sprite.group([
      Sprite.circle(color, 0.34) |> Sprite.move(-0.32, 0.0),
      Sprite.circle(color, 0.34) |> Sprite.move(0.32, 0.0),
      Sprite.circle(color, 0.34) |> Sprite.move(0.0, 0.32),
      Sprite.circle(color, 0.34) |> Sprite.move(0.0, -0.32)
    ])
  else if index == 2.0 then
    Sprite.group([
      Sprite.circle(color, 0.48),
      Sprite.line(color, 0.12, { x: -0.82, y: 0.0 }, { x: 0.82, y: 0.0 }),
      Sprite.line(color, 0.12, { x: 0.0, y: -0.82 }, { x: 0.0, y: 0.82 })
    ])
  else if index == 3.0 then
    Sprite.polygon(color, [
      { x: 0.0, y: 0.72 },
      { x: 0.66, y: -0.5 },
      { x: -0.66, y: -0.5 }
    ])
  else
    Sprite.group([
      Sprite.circle(color, 0.58),
      Sprite.circle(panel, 0.42),
      Sprite.circle(color, 0.17)
    ])

let card = (model, tts, index, x, y) =>
  let hovered = model.hover == index in
  let selected = model.selected == index in
  let color = accent(index) in
  let lift = if hovered then 0.18 else 0.0 in
  let pulse = 1.0 + (if selected then Math.sin(tts * 5.0) * 0.035 else 0.0) in
  Sprite.group([
    Sprite.rectangle(bg, 4.5, 3.5) |> Sprite.move(0.12, -0.14),
    Sprite.rectangle(if hovered then color else dim, 4.42, 3.42),
    Sprite.rectangle(if hovered then panelHot else panel, 4.24, 3.24),
    glyph(index, color) |> Sprite.moveY(0.48),
    Sprite.text(ink, 0.54, title(index)) |> Sprite.moveY(-0.86),
    if selected
    then Sprite.text(color, 0.28, "SELECTED") |> Sprite.moveY(-1.3)
    else Sprite.text(dim, 0.28, "CLICK TO CHOOSE") |> Sprite.moveY(-1.3)
  ])
  |> Sprite.scale(pulse)
  |> Sprite.move(x, y + lift)

let pointerSprite = (model) =>
  match model.pointer with
  | Option.Some(point) =>
      Sprite.group([
        Sprite.circle(ink, 0.18),
        Sprite.circle(bg, 0.1),
        Sprite.line(ink, 0.045, { x: -0.32, y: 0.0 }, { x: 0.32, y: 0.0 }),
        Sprite.line(ink, 0.045, { x: 0.0, y: -0.32 }, { x: 0.0, y: 0.32 })
      ])
      |> Sprite.move(point.x, point.y)
  | Option.None => Sprite.blank()

let draw = (model, tts) =>
  Sprite.group([
    Sprite.rectangle(bg, 30.0, 18.0) |> Sprite.move(2.0, 0.0),
    Sprite.text(ink, 0.92, "POINTER CONSTELLATION") |> Sprite.move(2.0, 5.15),
    Sprite.text(dim, 0.34, "RESIZE THE WINDOW — PICKS STAY IN WORLD SPACE")
      |> Sprite.move(2.0, 4.48),
    card(model, tts, 0.0, -4.5, 2.0),
    card(model, tts, 1.0, 1.0, 2.0),
    card(model, tts, 2.0, 6.5, 2.0),
    card(model, tts, 3.0, -1.75, -2.0),
    card(model, tts, 4.0, 3.75, -2.0),
    pointerSprite(model)
  ])
  |> Frame.create2D(camera)

expect hitCard({ x: -4.5, y: 2.0 }) == 0.0
expect hitCard({ x: 3.75, y: -2.0 }) == 4.0
expect hitCard({ x: 20.0, y: 20.0 }) == 0.0 - 1.0
