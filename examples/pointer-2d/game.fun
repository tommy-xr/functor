// Resize freely: Camera2D.toWorld uses the renderer's fit and returns None in
// letterbox bars, so the pointer stays on the same world-space target.
let camera =
  Camera2D.create(20.0, 12.0)
  |> Camera2D.at(2.0, -1.0)
  |> Camera2D.zoom(1.1)

let init = {
  pointer: Option.None,
  hover: 0.0 - 1.0,
  selected: 0.0 - 1.0,
  clickPending: false
}

let hitTarget = (point) =>
  let inside = (cx) =>
    point.x >= cx - 2.2 && point.x <= cx + 2.2 &&
    point.y >= -2.8 && point.y <= 0.8
  in
  if inside(-1.0) then 0.0
  else if inside(5.0) then 1.0
  else 0.0 - 1.0

let sampledInput = (model, snapshot: Input.snapshot) =>
  let pointer = Camera2D.toWorld(snapshot.mouse, camera) in
  match pointer with
  | Option.Some(point) =>
      let hover = hitTarget(point) in
      { model with
          pointer: pointer,
          hover: hover,
          selected:
            if model.clickPending && hover >= 0.0 then hover else model.selected,
          clickPending: false }
  | Option.None =>
      { model with
          pointer: Option.None,
          hover: 0.0 - 1.0,
          clickPending: false }

let mouseButton = (model, button, isDown) =>
  if button == Mouse.Left && isDown
  then { model with clickPending: true }
  else model

let tick = (model, dt, tts) => model

let bg = Color.rgb(0.025, 0.035, 0.09)
let panel = Color.rgb(0.075, 0.09, 0.18)
let ink = Color.rgb(0.91, 0.95, 1.0)
let dim = Color.rgb(0.48, 0.57, 0.74)
let cyan = Color.rgb(0.18, 0.88, 0.98)
let pink = Color.rgb(1.0, 0.28, 0.67)

let target = (model, index, x, label, color) =>
  let hovered = model.hover == index in
  let selected = model.selected == index in
  Sprite.group([
    Sprite.rectangle(if hovered then color else dim, 4.5, 3.7),
    Sprite.rectangle(panel, 4.25, 3.45),
    Sprite.text(color, 0.7, label) |> Sprite.moveY(0.35),
    Sprite.text(if selected then color else dim, 0.3,
      if selected then "SELECTED" else "CLICK TO CHOOSE")
      |> Sprite.moveY(-0.75)
  ])
  |> Sprite.move(x, -1.0)

let pointerSprite = (model) =>
  match model.pointer with
  | Option.Some(point) =>
      Sprite.group([
        Sprite.circle(ink, 0.16),
        Sprite.line(ink, 0.04, { x: -0.3, y: 0.0 }, { x: 0.3, y: 0.0 }),
        Sprite.line(ink, 0.04, { x: 0.0, y: -0.3 }, { x: 0.0, y: 0.3 })
      ])
      |> Sprite.move(point.x, point.y)
  | Option.None => Sprite.blank()

let draw = (model, tts) =>
  Sprite.group([
    Sprite.rectangle(bg, 24.0, 15.0) |> Sprite.move(2.0, -1.0),
    Sprite.text(ink, 0.85, "RESIZE-CORRECT POINTER") |> Sprite.move(2.0, 3.4),
    Sprite.text(dim, 0.32, "BARS ARE NOT PICKABLE") |> Sprite.move(2.0, 2.75),
    target(model, 0.0, -1.0, "CYAN", cyan),
    target(model, 1.0, 5.0, "PINK", pink),
    pointerSprite(model)
  ])
  |> Frame.create2D(camera)

expect hitTarget({ x: -1.0, y: -1.0 }) == 0.0
expect hitTarget({ x: 5.0, y: -1.0 }) == 1.0
expect hitTarget({ x: 10.0, y: 5.0 }) == 0.0 - 1.0
