// Digits: a number as glowing seven-segment digits. The headset does not draw
// the `ui` hook, so in-world feedback is geometry. Built in the XY plane,
// readable from +Z, one digit 1 wide and 2 tall.

// Segments: top, top right, bottom right, bottom, bottom left, top left, middle.
let segments: List<Stations.Box> =
  let bar = (x: float, y: float, w: float, h: float): Stations.Box =>
    { center: Quat.v3(x, y, 0.0), size: Quat.v3(w, h, 0.12) } in
  [
    bar(0.0, 1.0, 1.0, 0.18), bar(0.5, 0.5, 0.18, 1.0), bar(0.5, -0.5, 0.18, 1.0),
    bar(0.0, -1.0, 1.0, 0.18), bar(-0.5, -0.5, 0.18, 1.0), bar(-0.5, 0.5, 0.18, 1.0),
    bar(0.0, 0.0, 1.0, 0.18),
  ]

let patterns = [
  [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0], [0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
  [1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0],
  [0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0],
  [1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
  [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0],
]

/// Which segments light for `n` (0-9); anything else lights none.
let litFor = (n: float): List<bool> =>
  List.nth(n, patterns) |> Option.defaultValue([]) |> List.map((on) => on > 0.0)

expect (litFor(8.0) |> List.filter((on) => on) |> List.length) == 7.0
expect (litFor(1.0) |> List.filter((on) => on) |> List.length) == 2.0
expect litFor(12.0) == []

let digit = (n: float, on: Color.t, off: Color.t): Scene.t =>
  let lit = litFor(n) in
  Scene.group(segments |> List.indexedMap((i, s) =>
    Scene.cube()
      |> Scene.scaleXYZ(s.size.x, s.size.y, s.size.z)
      |> Scene.translate(Quat.toVec3(s.center))
      |> Scene.emissive(if List.nth(i, lit) |> Option.defaultValue(false) then on else off)))

/// Two digits, 0-99 (clamped); a leading zero stays dark.
let number = (value: float, on: Color.t, off: Color.t): Scene.t =>
  let v = Math.clamp(0.0, 99.0, Math.floor(value)) in
  let tens = Math.floor(v / 10.0) in
  Scene.group([
    digit(if tens > 0.0 then tens else 10.0, on, off) |> Scene.translate(Vec3.make(-0.8, 0.0, 0.0)),
    digit(Math.mod(v, 10.0), on, off) |> Scene.translate(Vec3.make(0.8, 0.0, 0.0)),
  ])
