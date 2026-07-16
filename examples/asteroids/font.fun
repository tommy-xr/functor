// font.fun — module Font: a tiny 5x3 "pixel" font rendered as emissive
// cubes in the XZ plane, for centered arcade-style title screens (the Ui
// prelude has corner anchors only — see docs/todo.md). Glyphs are 5 rows
// of 3 cells (1.0 = cube). Only the letters the game needs exist.
//
// There is no way to iterate a string's characters in Functor Lang today
// (no Text.chars), so words are passed as explicit glyph lists:
//   Font.word(0.7, 0.0, -8.0, [Font.gA, Font.gS, ...])

let gA = [[0.0,1.0,0.0],[1.0,0.0,1.0],[1.0,1.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0]]
let gD = [[1.0,1.0,0.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,1.0,0.0]]
let gE = [[1.0,1.0,1.0],[1.0,0.0,0.0],[1.0,1.0,0.0],[1.0,0.0,0.0],[1.0,1.0,1.0]]
let gG = [[0.0,1.0,1.0],[1.0,0.0,0.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[0.0,1.0,1.0]]
let gI = [[1.0,1.0,1.0],[0.0,1.0,0.0],[0.0,1.0,0.0],[0.0,1.0,0.0],[1.0,1.0,1.0]]
let gM = [[1.0,0.0,1.0],[1.0,1.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0]]
let gN = [[1.0,0.0,1.0],[1.0,1.0,1.0],[1.0,0.0,1.0],[1.0,1.0,1.0],[1.0,0.0,1.0]]
let gO = [[1.0,1.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,1.0,1.0]]
let gP = [[1.0,1.0,1.0],[1.0,0.0,1.0],[1.0,1.0,1.0],[1.0,0.0,0.0],[1.0,0.0,0.0]]
let gR = [[1.0,1.0,1.0],[1.0,0.0,1.0],[1.0,1.0,0.0],[1.0,0.0,1.0],[1.0,0.0,1.0]]
let gS = [[0.0,1.0,1.0],[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0],[1.0,1.0,0.0]]
let gT = [[1.0,1.0,1.0],[0.0,1.0,0.0],[0.0,1.0,0.0],[0.0,1.0,0.0],[0.0,1.0,0.0]]
let gU = [[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,1.0,1.0]]
let gV = [[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[0.0,1.0,0.0]]
let gW = [[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,0.0,1.0],[1.0,1.0,1.0],[1.0,0.0,1.0]]
let gY = [[1.0,0.0,1.0],[1.0,0.0,1.0],[0.0,1.0,0.0],[0.0,1.0,0.0],[0.0,1.0,0.0]]
let gSpace = [[0.0,0.0,0.0],[0.0,0.0,0.0],[0.0,0.0,0.0],[0.0,0.0,0.0],[0.0,0.0,0.0]]

// Cubes for one glyph with its top-left cell at (gx, gz); s = cell size.
// Rows advance toward +z (screen-down under the overhead camera).
let glyphCubes = (gx, gz, s, rows) =>
  let folded = rows |> List.fold((acc, row) =>
    let (rz, out) = acc in
    let rowFolded = row |> List.fold((acc2, c) =>
      let (cx, out2) = acc2 in
      (cx + s,
       (match c > 0.5 with
        | true => [Scene.cube() |> Scene.scale(s * 0.9) |> Scene.translate(Vec3.make(cx, 0.0, rz)), ..out2]
        | false => out2)),
      (gx, out)) in
    let (ignored, out3) = rowFolded in
    (rz + s, out3),
    (gz, [])) in
  let (ignored2, cubes) = folded in
  cubes

// A word centered on (cx, cz): glyphs are 3 cells + 1 gap wide (4*s pitch).
let word = (s, cx, cz, glyphs) =>
  let count = Lib.length(glyphs) in
  let width = count * 4.0 * s - s in
  // startX/row origin are CELL CENTERS, hence the half-cell offsets.
  let startX = cx - width * 0.5 + s * 0.5 in
  let folded = glyphs |> List.fold((acc, g) =>
    let (gx, out) = acc in
    (gx + 4.0 * s, Lib.append(glyphCubes(gx, cz - 2.0 * s, s, g), out)),
    (startX, [])) in
  let (ignored, cubes) = folded in
  Scene.group(cubes)
