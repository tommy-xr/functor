// The cinema's front (cinema.fun dresses its street side with it): a deep
// marquee over the entrance, its letter boards and soffit lit, bulbs
// chasing round its edges and a neon strip under its lip; a blade sign at
// the facade's west corner; backlit poster cases along the front wall; a
// ticket booth; the doors set back between two cheeks, their double glass
// doors propped open; and a terrazzo forecourt with a roped queue, an
// A-frame and a bin.
//
// Boxes ride the street side's draws, the signs and posters are quads of
// one atlas (cinemaFront.mjs) in one static model, the bulbs three
// instanced draws (one per chase phase, recoloured by the clock), the
// glass the street side's panes. By night the boards, soffit and posters
// glow and the bulbs run; by day they're lit like paint and the bulbs are
// out. No shadows; nothing is baked.

open Layout

let box = Exterior.box
let rgb = Building.rgb
let v = Vec3.make
let l = CinemaLayout.layout

// The facade's outer face, and its frame: u is x, d out toward the street.
let front = l.footprint.z0 - l.facade / 2.0
let south = Streetscape.sideFrame(l, South)
let walk = Streetscape.walk
// The terrazzo's top: over the sidewalk, and in the forecourt the sidewalk
// is cut back from (the road's level).
let pave = walk + 0.025
let court = Exterior.street + 0.02

let fascia = rgb(178.0, 152.0, 160.0)
let brass = rgb(150.0, 118.0, 64.0)
let boothDark = rgb(12.0, 10.0, 14.0)
let bladeRed = rgb(56.0, 18.0, 28.0)
let velvet = rgb(120.0, 20.0, 34.0)
let deadBulb = rgb(70.0, 64.0, 58.0)
let terrazzoTint = rgb(236.0, 232.0, 226.0)

// The cinema's finishes: painted stucco over a tiled base, and a paler coat.
let stuccoWall = Finishes.on(Finishes.stucco, rgb(122.0, 104.0, 132.0))
let stuccoPale = Finishes.on(Finishes.stucco, rgb(146.0, 132.0, 154.0))
let baseTiles = Finishes.on(Finishes.tiles, rgb(64.0, 118.0, 124.0))

// ---- The marquee -----------------------------------------------------------------

// 22 m wide, 4.2 m deep, its fascia 1.7 m tall; the soffit's lit panel
// just under it.
let marquee = Streetscape.boxIn(south, 19.0, 41.0, 3.4, 5.1, 0.0, 4.2)
let soffitY = marquee.y0 - 0.01
let boardY0 = 3.62
let boardY1 = 4.88
// A light box's face: `night` glowing, `day` its faded cell, lit.
let lightBox = (night: StreetAtlas.Cell, day: StreetAtlas.Cell, q: Building.Quad): Streetscape.Decal =>
  { kind: Streetscape.Glow, day: day, night: night, q: q }
let boards: List<Streetscape.Decal> = [
  lightBox(CinemaAtlas.boardFront, CinemaAtlas.boardFrontDay, Streetscape.faceIn(south, true, 19.4, 40.6, boardY0, boardY1, 4.215)),
  lightBox(CinemaAtlas.boardSide, CinemaAtlas.boardSideDay, Streetscape.faceIn({ side: West, x: marquee.x0, z: 0.0 }, true, front - 3.95, front - 0.25, boardY0, boardY1, 0.015)),
  lightBox(CinemaAtlas.boardSide, CinemaAtlas.boardSideDay, Streetscape.faceIn({ side: East, x: marquee.x1, z: 0.0 }, true, front - 3.95, front - 0.25, boardY0, boardY1, 0.015)),
  Streetscape.glow(CinemaAtlas.soffit, (Building.ceilingFace(0.0, marquee.x0 + 0.05, marquee.z0 + 0.05, marquee.x1 - 0.05, front, soffitY)).q),
]
// Cyan under the lip, along the front and down both sides.
let neon = [
  box(19.1, 3.33, front - 4.14, 40.9, 3.37, front - 4.08),
  box(19.08, 3.33, front - 4.08, 19.14, 3.37, front - 0.2),
  box(40.86, 3.33, front - 4.08, 40.92, 3.37, front - 0.2),
]

// ---- The blade sign --------------------------------------------------------------

// A maroon blade 30 cm thick standing 2.1 m out from the facade by its west
// corner, on two brackets under the cornice and a stay over it to a post on
// the roof, CINEMA down both faces; pink tubes round its edges.
let blade = box(1.25, 3.7, front - 2.45, 1.55, 12.1, front - 0.35)
let bladeBoxes = [
  (blade, bladeRed),
  (box(1.3, 4.5, front - 0.35, 1.5, 4.62, front), Streetscape.darkSteel),
  (box(1.3, 9.0, front - 0.35, 1.5, 9.12, front), Streetscape.darkSteel),
  (box(1.3, 11.2, front - 0.35, 1.5, 11.32, 1.2), Streetscape.darkSteel),
  (box(1.3, l.storey, 1.0, 1.5, 11.2, 1.2), Streetscape.darkSteel),
]
let bladeFaces: List<Streetscape.Decal> = [
  Streetscape.glow(CinemaAtlas.blade, Streetscape.faceIn({ side: West, x: blade.x0, z: 0.0 }, true, front - 2.35, front - 0.45, 3.82, 12.05, 0.012)),
  Streetscape.glow(CinemaAtlas.blade, Streetscape.faceIn({ side: East, x: blade.x1, z: 0.0 }, true, front - 2.35, front - 0.45, 3.82, 12.05, 0.012)),
]
let pink = [
  box(1.22, blade.y0, blade.z0 - 0.06, 1.58, blade.y1, blade.z0),
  box(1.22, blade.y0, blade.z1 - 0.06, 1.58, blade.y1, blade.z1),
  box(1.22, blade.y1, blade.z0 - 0.06, 1.58, blade.y1 + 0.06, blade.z1),
  box(1.22, blade.y0 - 0.06, blade.z0 - 0.06, 1.58, blade.y0, blade.z1),
]

// ---- Bulbs -----------------------------------------------------------------------

// A bulb and its place along its run: the chase lights every third.
type Bulb = { at: Vec3.t, k: float }
// `n` values from `a` to `b`.
let spaced = (a: float, b: float, n: float): List<float> => List.range(n) |> List.map((k) => a + (b - a) * k / (n - 1.0))
// Round the marquee (down its west side, along the front, up its east
// side) in two rows, 3.5 cm proud; and up both faces of the blade.
let marqueePath: List<(float, float)> = List.flatten([
  spaced(front - 0.3, front - 4.05, 16.0) |> List.map((z) => (marquee.x0 - 0.035, z)),
  spaced(19.15, 40.85, 87.0) |> List.map((x) => (x, marquee.z0 - 0.035)),
  spaced(front - 4.05, front - 0.3, 16.0) |> List.map((z) => (marquee.x1 + 0.035, z)),
])
let bulbs: List<Bulb> = List.flatten([
  [3.51, 4.99] |> List.concatMap((y) => marqueePath |> List.indexedMap((k, p) => let (x, z) = p in { at: v(x, y, z), k: k })),
  [blade.x0 - 0.035, blade.x1 + 0.035] |> List.concatMap((x) =>
    spaced(4.0, 11.8, 33.0) |> List.indexedMap((k, y) => { at: v(x, y, front - 2.25), k: k })),
])
// One in twenty is dead.
let isDead = (b: Bulb): bool => Building.noise(Vec3.x(b.at) * 13.1 + Vec3.y(b.at) * 7.7 + Vec3.z(b.at) * 3.3) < 0.05
let bulbSize = 0.08
let phases: List<List<Instance.t>> = List.range(3.0) |> List.map((p) =>
  bulbs |> List.filter((b) => not isDead(b) && Math.mod(b.k, 3.0) == p)
    |> List.map((b) => Instance.trs(b.at, 0deg, bulbSize, bulbSize, bulbSize)))
let deadBoxes: List<(Exterior.Box, Color.t)> = bulbs |> List.filter(isDead) |> List.map((b) =>
  let h = bulbSize / 2.0 in
  (box(Vec3.x(b.at) - h, Vec3.y(b.at) - h, Vec3.z(b.at) - h, Vec3.x(b.at) + h, Vec3.y(b.at) + h, Vec3.z(b.at) + h), deadBulb))

// The chase: 6 steps a second, so a phase lights for 1/6 s and comes round
// every half second. Lit bulbs glow warm, the rest low; by day all are out.
let chaseRate = 6.0
let phaseAt = (tts: float): float => Math.mod(Math.floor(tts * chaseRate), 3.0)
let bulbOn: Hours.Rgb = { r: 1.0, g: 0.9, b: 0.68 }
let bulbLow: Hours.Rgb = { r: 0.42, g: 0.3, b: 0.18 }
let bulbOff: Hours.Rgb = { r: 0.46, g: 0.44, b: 0.42 }
type Chase = { phase: float, lamps: float }
let chaseOf = (c: Chase): Scene.t =>
  Scene.group(phases |> List.indexedMap((p, is) =>
    Scene.cube() |> Scene.color(Hours.color(Hours.mix(bulbOff, if p == c.phase then bulbOn else bulbLow, c.lamps))) |> Scene.instanced(is)))
// The bulbs at time `tts`, the sodium lamps' level `lamps` (1 at night),
// in the cinema's space: rebuilt only when the phase (held by day) or the
// hour changes, and its instance lists are the same values every time, so
// they upload once.
let chaseAt = (tts: float, lamps: float): Scene.t =>
  Scene.memo("cinema-chase", { phase: if lamps > 0.0 then phaseAt(tts) else 0.0, lamps: lamps }, chaseOf)

// ---- Poster cases ------------------------------------------------------------------

// Six backlit cases on the front wall under the marquee, 1.2 × 1.9 m and
// 14 cm proud: a brass frame, the poster 5 cm out, glass in front of it.
let posterAt = [(20.1, (CinemaAtlas.poster0, CinemaAtlas.poster0Day)), (21.9, (CinemaAtlas.poster1, CinemaAtlas.poster1Day)),
                (34.5, (CinemaAtlas.poster2, CinemaAtlas.poster2Day)), (36.2, (CinemaAtlas.poster3, CinemaAtlas.poster3Day)),
                (37.9, (CinemaAtlas.poster4, CinemaAtlas.poster4Day)), (39.6, (CinemaAtlas.poster5, CinemaAtlas.poster5Day))]
let caseBoxes: List<(Exterior.Box, Color.t)> = posterAt |> List.concatMap((e) =>
  let (c, _) = e in
  let p = (u0: float, u1: float, y0: float, y1: float, d1: float) => Streetscape.boxIn(south, c + u0, c + u1, y0, y1, 0.0, d1) in
  [(p(-0.6, -0.52, 0.6, 2.5, 0.14), brass), (p(0.52, 0.6, 0.6, 2.5, 0.14), brass),
   (p(-0.52, 0.52, 2.42, 2.5, 0.14), brass), (p(-0.52, 0.52, 0.6, 0.68, 0.14), brass),
   (p(-0.52, 0.52, 0.68, 2.42, 0.02), boothDark)])
let posters: List<Streetscape.Decal> = posterAt |> List.map((e) =>
  let (c, cells) = e in
  let (night, day) = cells in lightBox(night, day, Streetscape.faceIn(south, true, c - 0.52, c + 0.52, 0.68, 2.42, 0.05)))

// ---- The entrance ----------------------------------------------------------------

// Cheeks either side of the forecourt, 2.4 m out to the marquee's
// underside, so the doors stand back in a recess: tiles to the base's
// height, stucco over.
let cheeks: List<(Exterior.Box, Finishes.Surface)> = [(26.5, 26.9), (33.1, 33.5)] |> List.concatMap((e) =>
  let (u0, u1) = e in
  [(Streetscape.boxIn(south, u0, u1, Exterior.street, 1.1, 0.0, 2.4), baseTiles),
   (Streetscape.boxIn(south, u0, u1, 1.1, soffitY - 0.01, 0.0, 2.4), stuccoWall)])

// The double doors, propped open against the 1.8 m gap's edges (glass.fun
// glazes the rest of the doorway): each leaf a brass frame round a pane,
// a push bar on its inner face, swung out 90° from the wall line, where
// glass.fun's entrance panes stand (`windowOut`). The street side only
// draws them (shells don't), but they're solid: the cinema's interior is
// active long before they're in reach.
let doorLine = l.footprint.z0 - (l.facade / 2.0 - 0.05)
let leafOut = doorLine - 0.9
let mid = (Exterior.entranceOf(l).lo + Exterior.entranceOf(l).hi) / 2.0
// The leaf on the gap's west edge (`side` 1) or east edge (-1).
let leafAt = (side: float): List<(Exterior.Box, Color.t)> =>
  let x = mid - 0.9 * side in
  let (x0, x1) = if side > 0.0 then (x, x + 0.04) else (x - 0.04, x) in
  let (b0, b1) = if side > 0.0 then (x1, x1 + 0.04) else (x0 - 0.04, x0) in
  [(box(x0, 0.0, leafOut, x1, 2.2, leafOut + 0.06), brass), (box(x0, 0.0, doorLine - 0.06, x1, 2.2, doorLine), brass),
   (box(x0, 2.12, leafOut, x1, 2.2, doorLine), brass), (box(x0, 0.0, leafOut, x1, 0.25, doorLine), brass),
   (box(b0, 0.98, leafOut + 0.12, b1, 1.03, doorLine - 0.12), brass)]
let doorBoxes = List.flatten([leafAt(1.0), leafAt(-1.0)])
// Their colliders, each leaf a slab.
let doorBlocks = [1.0, -1.0] |> List.map((side) =>
  let x = mid - 0.9 * side in box(x - 0.06, 0.0, leafOut, x + 0.06, 2.2, doorLine))

// ---- The ticket booth --------------------------------------------------------------

// Against the west cheek: a tiled base, a dark window in a frame with a
// chrome speaker grille on its glass, a counter, a stucco head with its
// sign, a cap.
let booth = Streetscape.boxIn(south, 23.4, 26.5, walk, 2.85, 0.0, 1.7)
let boothBoxes: List<(Exterior.Box, Color.t)> =
  let p = (u0: float, u1: float, y0: float, y1: float, d0: float, d1: float) => Streetscape.boxIn(south, u0, u1, y0, y1, d0, d1) in
  [(p(23.5, 26.5, 1.1, 2.35, 0.0, 1.55), boothDark),
   (p(23.4, 23.5, 1.1, 2.35, 0.0, 1.55), Facade.frameDark),
   (p(23.4, 23.56, 1.1, 2.35, 1.55, 1.72), Facade.frameDark), (p(26.34, 26.5, 1.1, 2.35, 1.55, 1.72), Facade.frameDark),
   (p(24.93, 24.97, 1.15, 2.35, 1.55, 1.62), Facade.frameDark),
   (p(23.4, 26.5, 1.1, 1.15, 1.55, 1.9), Streetscape.steel),
   (p(24.83, 25.07, 1.35, 1.59, 1.62, 1.66), Streetscape.steel),
   (p(23.3, 26.5, 2.75, 2.85, 0.0, 1.82), Facade.frameDark)]
let boothSkinned: List<(Exterior.Box, Finishes.Surface)> = [
  (Streetscape.boxIn(south, 23.4, 26.5, walk, 1.1, 0.0, 1.7), baseTiles),
  (Streetscape.boxIn(south, 23.4, 26.5, 2.35, 2.75, 0.0, 1.72), stuccoPale),
]
let boothSigns: List<Streetscape.Decal> = [
  Streetscape.glow(CinemaAtlas.tickets, Streetscape.faceIn(south, true, 24.35, 25.55, 2.41, 2.685, 1.725)),
  Streetscape.sign(CinemaAtlas.grille, Streetscape.faceIn(south, true, 24.84, 25.06, 1.36, 1.58, 1.665)),
]

// ---- The forecourt -----------------------------------------------------------------

// Terrazzo under the marquee: over the sidewalk, and down in the forecourt
// to the doors.
let terrazzo = [
  box(26.9, Exterior.street - 0.02, front - Streetscape.depth, 33.1, court, front),
  Streetscape.boxIn(south, marquee.x0, 26.9, walk, pave, 0.0, Streetscape.depth - Streetscape.curb),
  Streetscape.boxIn(south, 33.1, marquee.x1, walk, pave, 0.0, Streetscape.depth - Streetscape.curb),
]
let terrazzoScene = Scene.cube() |> Scene.litWorldTexture(Asset.texture("cinema-terrazzo.png"), 1.2)
  |> Scene.instanced(terrazzo |> List.map((b) => Exterior.instanceOf(b) |> Instance.tint(terrazzoTint)))

// A queue roped off in front of the booth: brass stanchions, red rope
// sagging between them.
let queue = [(22.0, 1.9), (22.0, 2.6), (23.45, 2.6), (24.9, 2.6), (26.35, 2.6)]
let stanchion = (u: float, d: float): List<(Exterior.Box, Color.t)> =>
  let p = (h: float, y0: float, y1: float) => Streetscape.boxIn(south, u - h, u + h, pave + y0, pave + y1, d - h, d + h) in
  [(p(0.15, 0.0, 0.03), brass), (p(0.025, 0.03, 0.95), brass), (p(0.04, 0.95, 1.01), brass)]
// A rope from (u0, d0) to (u1, d1), along u or d: its middle third lower.
let rope = (a: (float, float), b: (float, float)): List<(Exterior.Box, Color.t)> =>
  let (u0, d0) = a in
  let (u1, d1) = b in
  [0.0, 1.0, 2.0] |> List.map((k) =>
    let y = if k == 1.0 then 0.78 else 0.84 in
    let s0 = k / 3.0 in
    let s1 = (k + 1.0) / 3.0 in
    let (ua, ub) = (u0 + (u1 - u0) * s0, u0 + (u1 - u0) * s1) in
    let (da, db) = (d0 + (d1 - d0) * s0, d0 + (d1 - d0) * s1) in
    (Streetscape.boxIn(south, Math.min(ua, ub) - 0.02, Math.max(ua, ub) + 0.02, pave + y, pave + y + 0.04, Math.min(da, db) - 0.02, Math.max(da, db) + 0.02), velvet))
let queueBoxes = List.flatten([
  queue |> List.concatMap((e) => let (u, d) = e in stanchion(u, d)),
  queue |> List.zip(List.drop(1.0, queue)) |> List.concatMap((e) => let (a, b) = e in rope(a, b)),
])

// An A-frame on the terrazzo east of the doors: NOW SHOWING out to the
// street, COMING SOON back toward the building, each board leaning in to a
// hinge.
let aframeU = 36.0
let aframeD = 2.6
let aframeFaces: List<Streetscape.Decal> =
  let (x0, x1) = (aframeU - 0.3, aframeU + 0.3) in
  let y = pave + 0.05 in
  [Streetscape.sign(CinemaAtlas.aframeNow, { o: v(x1, y, front - aframeD - 0.2), a: v(x0 - x1, 0.0, 0.0), b: v(0.0, 0.9, 0.17) }),
   Streetscape.sign(CinemaAtlas.aframeSoon, { o: v(x0, y, front - aframeD + 0.2), a: v(x1 - x0, 0.0, 0.0), b: v(0.0, 0.9, 0.0 - 0.17) })]
let aframeBoxes: List<(Exterior.Box, Color.t)> =
  let p = (y0: float, y1: float, d0: float, d1: float) => Streetscape.boxIn(south, aframeU - 0.32, aframeU + 0.32, pave + y0, pave + y1, aframeD + d0, aframeD + d1) in
  [(p(0.93, 0.97, -0.04, 0.04), Streetscape.darkSteel), (p(0.0, 0.05, 0.16, 0.22), Streetscape.darkSteel), (p(0.0, 0.05, -0.22, -0.16), Streetscape.darkSteel)]

// ---- Assembled ---------------------------------------------------------------------

// What the street side gains as streetscape pieces: lit boxes kept on
// shells (the cases, the booth, the blade, dead bulbs, the A-frame under
// its boards, which the shells' signs keep), the queue on the street side
// only, the cheeks and the booth in finishes,
// and what you'd bump into. The doors aren't among them: they stand in the
// doorway the pieces keep clear of (cinema.fun trims them on instead).
let props: Streetscape.Props = Streetscape.joined([
  { Streetscape.nothing with
    plain: List.flatten([caseBoxes, boothBoxes, bladeBoxes, deadBoxes, aframeBoxes]),
    fine: queueBoxes,
    skinned: List.flatten([cheeks, boothSkinned]),
    solid: List.flatten([
      cheeks |> List.map((e) => let (b, _) = e in b),
      [booth,
       Streetscape.boxIn(south, 21.85, 26.5, pave, 1.0, 2.45, 2.75),
       Streetscape.boxIn(south, 21.85, 22.15, pave, 1.0, 1.75, 2.75),
       Streetscape.boxIn(south, aframeU - 0.35, aframeU + 0.35, pave, 1.0, aframeD - 0.24, aframeD + 0.24)],
    ]) },
  Streetscape.bin(south, 34.3, 3.1, pave),
])

// The signs, boards, soffit and posters as one static model of the atlas,
// by night (glowing faces unlit) or day (all lit, the boards and posters
// faded): a draw or two.
let atlas = Asset.texture("cinema-atlas.png")
let decals = List.flatten([boards, bladeFaces, posters, boothSigns, aframeFaces])
let glow: Exterior.Grid = { night: Streetscape.atlasModelOf(atlas, decals, true), day: Streetscape.atlasModelOf(atlas, decals, false) }

// The street side's extra glass (glass.fun draws it with the rest): each
// door leaf, the booth's window in two lights, each poster case; as
// (centre, yaw in degrees, width, height).
let panes: List<(Vec3.t, float, float, float)> = List.flatten([
  [(v(mid - 0.88, 1.185, (doorLine + leafOut) / 2.0), 90.0, 0.78, 1.87),
   (v(mid + 0.88, 1.185, (doorLine + leafOut) / 2.0), 270.0, 0.78, 1.87),
   (v(24.245, 1.75, front - 1.6), 180.0, 1.37, 1.2),
   (v(25.655, 1.75, front - 1.6), 180.0, 1.37, 1.2)],
  posterAt |> List.map((e) => let (c, _) = e in (v(c, 1.55, front - 0.11), 180.0, 1.04, 1.74)),
])

// ---- Checks ----------------------------------------------------------------------

// Every bulb but the dead is in exactly one phase.
expect List.length(phases |> List.concatMap((is) => is)) + List.length(deadBoxes) == List.length(bulbs)
expect List.length(deadBoxes) >= 3.0
// The phases come round in order, six a second.
expect ([0.0, 0.17, 0.34, 0.5] |> List.map(phaseAt)) == [0.0, 1.0, 2.0, 0.0]
// The doors leave the gap walkable: at least 1.6 m between the leaves'
// push bars.
let bars = doorBoxes |> List.filter((e) => let (b, _) = e in b.y1 - b.y0 < 0.06) |> List.map((e) => let (b, _) = e in b)
expect (bars |> List.map((b) => b.x0) |> List.maximum |> Option.defaultValue(0.0)) - (bars |> List.map((b) => b.x1) |> List.minimum |> Option.defaultValue(0.0)) >= 1.6
