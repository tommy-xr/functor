// Sprite.text — the built-in 2D font.
//
// Everything here draws with no asset at all: the font is compiled into the
// runtime. The example is also a layout demo, because `Sprite.measure` is what
// makes alignment and panel-sizing possible in pure game logic.

let cyan = Color.rgb(0.35, 0.95, 1.0)
let pink = Color.rgb(1.0, 0.4, 0.8)
let amber = Color.rgb(1.0, 0.8, 0.3)
let dim = Color.rgb(0.5, 0.55, 0.75)
let white = Color.rgb(1.0, 1.0, 1.0)
let panelFill = Color.rgb(0.1, 0.07, 0.19)

// Text is centered on its own box, so alignment is a half-width shift. These
// two helpers are the whole recipe — worth copying into any 2D game. They align
// the BLOCK: for a multi-line string only the widest line reaches the edge,
// because interior lines stay centered.
let leftAligned = (x, y, size, color, s) =>
  Sprite.text(color, size, s)
    |> Sprite.move(x + Sprite.measure(size, s).width * 0.5, y)

let rightAligned = (x, y, size, color, s) =>
  Sprite.text(color, size, s)
    |> Sprite.move(x - Sprite.measure(size, s).width * 0.5, y)

// A panel sized to its own caption: `measure` reports the block's box, so the
// chrome fits the text instead of being hand-tuned to it.
let captioned = (size, color, s) =>
  let box = Sprite.measure(size, s) in
  Sprite.group([
    Sprite.rectangle(panelFill, box.width + size, box.height + size),
    Sprite.text(color, size, s),
  ])

let init = { started: true }

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  // A score that actually counts, so the HUD row is doing real work.
  let score = Text.fixed(Math.floor(tts * 137.0), 0.0) in
  let lives = Text.fixed(3.0, 0.0) in

  // A three-line block. `\n` breaks lines at exactly one `size` of line
  // height, which is why stacking never overlaps.
  let dialog = "THE CHEST IS LOCKED.\nA RUSTED KEY LIES\nHALF-BURIED NEARBY." in

  Sprite.group([
    // --- HUD row: left-aligned labels, right-aligned values -------------
    // The values are right-aligned so digits grow LEFTWARD into their own
    // column and can never push the next label along. That is the whole
    // reason `measure` has to exist.
    leftAligned(-15.0, 7.8, 0.62, dim, "SCORE"),
    rightAligned(-8.5, 7.8, 0.62, cyan, score),
    leftAligned(-7.0, 7.8, 0.62, dim, "LIVES"),
    rightAligned(-2.0, 7.8, 0.62, cyan, lives),
    rightAligned(15.0, 7.8, 0.62, dim, "NO ASSET REQUIRED"),

    // --- the headline, two sizes ----------------------------------------
    Sprite.text(white, 1.9, "SPRITE.TEXT") |> Sprite.moveY(5.2),
    Sprite.text(pink, 0.72, "AN EMBEDDED PUBLIC-DOMAIN BITMAP FONT")
      |> Sprite.moveY(3.5),

    // --- a measured panel around a multi-line block ---------------------
    captioned(0.78, amber, dialog) |> Sprite.moveY(0.4),

    // --- the same string at four sizes ----------------------------------
    Sprite.text(dim, 0.45, "0.45  THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG")
      |> Sprite.moveY(-3.4),
    Sprite.text(dim, 0.7, "0.70  PACK MY BOX WITH FIVE DOZEN JUGS")
      |> Sprite.moveY(-4.5),
    Sprite.text(cyan, 1.0, "1.00  0123456789 !?@#$%&*")
      |> Sprite.moveY(-5.8),

    // --- filtering: the default vs crisp pixels -------------------------
    leftAligned(-14.0, -6.9, 0.5, dim, "LINEAR (DEFAULT)"),
    leftAligned(-14.0, -8.1, 1.5, white, "Aa8g"),
    leftAligned(2.0, -6.9, 0.5, dim, "NEAREST"),
    leftAligned(2.0, -8.1, 1.5, white, "Aa8g") |> Sprite.nearest(),
  ])
    |> Frame.create2D(Camera2D.create(32.0, 18.0))
    |> Frame.withClearColor(Color.rgb(0.04, 0.03, 0.09))

expect Sprite.measure(2.0, "SCORE").width == 10.0
expect Sprite.measure(2.0, "SCORE").height == 2.0
// A newline is a real line break, and the widest line sets the width.
expect Sprite.measure(1.0, "HI\nSCORE").width == 5.0
expect Sprite.measure(1.0, "HI\nSCORE").height == 2.0
// Height is the line height even for the empty string, so stacking by a
// measured height can never overlap two blocks.
expect Sprite.measure(1.5, "").height == 1.5
// Characters that draw nothing still occupy their cell.
expect Sprite.measure(1.0, "A B").width == 3.0
