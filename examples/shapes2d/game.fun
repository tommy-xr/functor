// Sprite.circle / Sprite.polygon / Sprite.line — filled 2D shapes.
//
// Everything here is asset-free procedural art. Before these primitives a 2D
// game could only fill rectangles, so vector-style art had to be faked from
// rotated thin rectangles (with corner notches and angle-dependent stroke width)
// and a filled circle or triangle was impossible without authoring a PNG.

let cyan = Color.rgb(0.35, 0.95, 1.0)
let pink = Color.rgb(1.0, 0.4, 0.8)
let amber = Color.rgb(1.0, 0.8, 0.3)
let lime = Color.rgb(0.6, 1.0, 0.45)
let dim = Color.rgb(0.5, 0.55, 0.75)
let white = Color.rgb(1.0, 1.0, 1.0)

// A regular n-gon as a point list. Points are used verbatim, so the centre is
// wherever the caller puts it.
let ngon = (sides, radius, cx, cy) =>
  List.range(sides)
    |> List.map((i) =>
         let a = i / sides * 2.0 * Math.pi in
         { x: cx + Math.cos(a) * radius, y: cy + Math.sin(a) * radius })

let label = (x, y, s) =>
  Sprite.text(dim, 0.42, s)
    |> Sprite.move(x + Sprite.measure(0.42, s).width * 0.5, y)

// A fan of equal-thickness spokes. Every spoke is the same `Sprite.line` at a
// different angle, so if thickness varied with angle it would be obvious here.
let spokes = (count, radius, thickness) =>
  List.range(count)
    |> List.map((i) =>
         let a = i / count * 2.0 * Math.pi in
         Sprite.line(cyan, thickness,
           { x: 0.0, y: 0.0 },
           { x: Math.cos(a) * radius, y: Math.sin(a) * radius }))
    |> Sprite.group()

// The classic asteroids hull has a notched tail, which is CONCAVE — so it is not
// one polygon. Two convex halves, grouped, is the documented way to fill it.
let ship =
  Sprite.group([
    Sprite.polygon(lime, [
      { x: 0.0, y: 1.3 }, { x: 0.0, y: -0.45 }, { x: -0.85, y: -0.9 }]),
    Sprite.polygon(lime, [
      { x: 0.0, y: 1.3 }, { x: 0.85, y: -0.9 }, { x: 0.0, y: -0.45 }]),
  ])

let init = { started: true }

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  let spin = Angle.radians(tts * 0.6) in
  let pulse = 1.35 + Math.sin(tts * 2.0) * 0.35 in

  Sprite.group([
    Sprite.text(white, 1.5, "FILLED 2D SHAPES") |> Sprite.moveY(7.4),

    // --- circles: a fixed one, and one that breathes -------------------
    Sprite.circle(pink, 1.6) |> Sprite.move(-11.5, 3.0),
    Sprite.circle(amber, pulse) |> Sprite.move(-7.4, 3.0),
    Sprite.circle(cyan, 0.55) |> Sprite.move(-4.2, 3.0),
    label(-13.1, 0.7, "CIRCLE"),

    // --- polygons: a triangle, a pentagon, a many-sided disc ----------
    Sprite.polygon(lime, [{ x: -1.3, y: -1.1 }, { x: 1.3, y: -1.1 }, { x: 0.0, y: 1.3 }])
      |> Sprite.move(0.6, 3.0),
    Sprite.polygon(pink, ngon(5.0, 1.5, 0.0, 0.0))
      |> Sprite.rotate(spin)
      |> Sprite.move(4.6, 3.0),
    Sprite.polygon(amber, ngon(9.0, 1.5, 0.0, 0.0))
      |> Sprite.rotate(spin)
      |> Sprite.move(8.6, 3.0),
    label(-0.7, 0.7, "POLYGON"),

    // --- lines: equal thickness at every angle -------------------------
    spokes(14.0, 3.0, 0.14) |> Sprite.move(-11.0, -4.2),
    label(-13.1, -8.2, "LINE"),

    // --- thickness is geometry: these two differ only in thickness -----
    Sprite.line(pink, 0.1, { x: -1.0, y: -6.2 }, { x: 4.0, y: -2.2 }),
    Sprite.line(pink, 0.45, { x: -1.0, y: -7.0 }, { x: 4.0, y: -3.0 }),
    label(-1.2, -8.2, "THICKNESS"),

    // --- a concave hull, filled as two convex halves ------------------
    ship |> Sprite.scale(2.1) |> Sprite.rotate(spin) |> Sprite.move(10.2, -4.4),
    label(7.2, -8.2, "CONCAVE = 2 CONVEX"),
  ])
    |> Frame.create2D(Camera2D.create(32.0, 18.0))
    |> Frame.withClearColor(Color.rgb(0.04, 0.03, 0.09))

// `ngon` is pure geometry, so it is testable with no renderer.
expect List.length(ngon(5.0, 1.0, 0.0, 0.0)) == 5.0
expect List.length(ngon(9.0, 2.0, 1.0, 1.0)) == 9.0
// Its real invariant: every vertex sits exactly `radius` from the centre. The
// worst deviation over all vertices must be ~0, which is what actually pins the
// trigonometry (a bound like `< 3.0` would hold for any vertex of any n-gon).
expect (
  let cx = 1.0 in
  let cy = 0.5 in
  let worst =
    ngon(6.0, 2.0, cx, cy)
      |> List.fold((acc, p) =>
           let dx = p.x - cx in
           let dy = p.y - cy in
           Math.max(acc, Math.abs(Math.sqrt(dx * dx + dy * dy) - 2.0)), 0.0) in
  worst < 0.0001
)
