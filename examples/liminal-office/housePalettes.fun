// The house's two palettes: building.fun's materials, index for index,
// repainted. Each is its own archetype with its own bake (the albedo bounces
// differently): ochre, a sepia family house; magenta, a children's wing
// with a magenta carpet stair, pink walls, orange trim and ceiling, and a
// white niche.
// Editing this file makes both house bakes stale: rebake (README).
//
// `stair` paints the flight's risers and parapet (building.fun's
// `castConcrete`); its treads are the hall's carpet.

open Building

// Kit materials the house keeps: the kitchen's fluorescent panel, and the
// signs and tower fittings it never shows.
let office = (i: float): Material => materialIn(materials, i)

// Two-colour squares `n` texels a side: lino.
let checker = (n: float, a: Paint, b: Paint): Texture.pixels =>
  image(2.0 * n, 2.0 * n, (x, y) => if Math.mod(Math.floor(x / n) + Math.floor(y / n), 2.0) == 0.0 then a else b)
let average = (a: Paint, b: Paint): Paint => paint((a.r + b.r) / 2.0, (a.g + b.g) / 2.0, (a.b + b.b) / 2.0)
// Vertical stripes or boards: a darker texel column every `n`.
let striped = (n: float, k: float, p: Paint): Texture.pixels =>
  image(16.0, 16.0, (x, y) => if Math.mod(x, n) == 0.0 then shade(k, p) else shade(1.0 + 0.04 * (noise(x + 16.0 * y) - 0.5), p))
// A niche's back, darker toward its edges: the recess's shadow.
let recessed = (p: Paint): Texture.pixels =>
  image(16.0, 16.0, (x, y) => shade(0.8 + 0.05 * Math.min(4.0, Math.min(Math.min(x, y), Math.min(15.0 - x, 15.0 - y))), p))
let lamp = paint(250.0, 204.0, 140.0)

type Colours = { carpet: Paint, linoA: Paint, linoB: Paint, wall: Paint, ceiling: Paint, trim: Paint, stair: Paint, niche: Paint }

// A palette from its colours, in building.fun's material order.
let paletteOf = (c: Colours): List<Material> => [
  material(c.carpet, 1.0, false, speckled(0.3, c.carpet)),
  material(average(c.linoA, c.linoB), 0.6, false, checker(8.0, c.linoA, c.linoB)),
  material(c.wall, 0.8, false, striped(8.0, 0.9, c.wall)),
  material(c.ceiling, 1.0, false, speckled(0.04, c.ceiling)),
  office(panelOn),
  office(panelOff),
  material(c.trim, 1.0, false, speckled(0.08, c.trim)),
  office(exitGreen),
  material(lamp, 0.0, true, solid(lamp)),
  material(c.stair, 0.8, false, striped(8.0, 0.8, c.stair)),
  office(mustard),
  office(steel),
  office(doorPanel),
  material(c.niche, 0.0, false, recessed(c.niche)),
]

let ochre = paletteOf({
  carpet: paint(150.0, 112.0, 52.0),
  linoA: paint(214.0, 200.0, 160.0), linoB: paint(150.0, 110.0, 66.0),
  wall: paint(196.0, 170.0, 116.0),
  ceiling: paint(214.0, 204.0, 180.0),
  trim: paint(96.0, 66.0, 40.0),
  stair: paint(128.0, 92.0, 56.0),
  niche: paint(232.0, 220.0, 190.0),
})

let magenta = paletteOf({
  carpet: paint(176.0, 36.0, 120.0),
  linoA: paint(240.0, 206.0, 210.0), linoB: paint(250.0, 244.0, 236.0),
  wall: paint(232.0, 150.0, 168.0),
  ceiling: paint(240.0, 176.0, 120.0),
  trim: paint(228.0, 120.0, 56.0),
  stair: paint(150.0, 28.0, 104.0),
  niche: paint(246.0, 246.0, 240.0),
})

expect List.length(ochre) == List.length(materials) && List.length(magenta) == List.length(materials)
