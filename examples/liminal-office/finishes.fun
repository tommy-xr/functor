// The street side's textured finishes: facades.mjs's tileable images, laid
// on boxes by world position (`Scene.litWorldTexture`), so a box's texels
// stay square at any scale and copies of one finish draw as one instanced
// cube. The images are near-white: a surface's tint gives its colour.
// They're painted offline (facades.mjs): 256² of noise would cost the
// interpreter seconds at load, where the interiors' 32² ones are cheap.

// A finish: its image, repeating every `tile` metres.
type Finish = { name: string, texture: Asset.Texture, tile: float }
// A finish in a colour.
type Surface = { finish: Finish, tint: Color.t }

let finishOf = (name: string, tile: float): Finish =>
  { name: name, tile: tile, texture: Asset.texture($"facade-{name}.png") }

// Storey-high precast panels (office): a repeat is a storey, two panels wide.
let precast = finishOf("precast", 3.3)
// Curtain wall (towers): metal ribs, a spandrel band over each storey line.
let curtain = finishOf("curtain", 3.3)
// Painted stucco (cinema).
let stucco = finishOf("stucco", 2.0)
// 15 cm glazed tiles (the cinema's base).
let tiles = finishOf("tiles", 0.6)
// Flat roofs' membrane.
let roofing = finishOf("roofing", 4.0)
// Split-face block (mall): 40 × 20 cm in running bond.
let splitface = finishOf("splitface", 1.6)
// Ribbed metal (the mall's fascia): a standing seam every 0.5 m.
let fascia = finishOf("fascia", 1.0)
// Clapboard (houses): 15 cm boards, each lapped over the one below.
let clapboard = finishOf("clapboard", 1.2)
// Asphalt shingles (houses' gables): 14 cm courses of staggered tabs.
let shingles = finishOf("shingles", 1.12)

// Sidewalks: 1.5 m concrete slabs in sawn joints (streetscape.fun).
let paving = finishOf("paving", 3.0)

let all = [precast, curtain, stucco, tiles, roofing, splitface, fascia, clapboard, shingles, paving]

let on = (f: Finish, tint: Color.t): Surface => { finish: f, tint: tint }

// Placed boxes, by surface: each finish's instances, tinted, in `all`'s
// order.
let groupedBy = (placed: List<(Instance.t, Surface)>): List<List<Instance.t>> =>
  all |> List.map((f) => placed |> List.filter((e) => let (i, s) = e in s.finish.name == f.name)
    |> List.map((e) => let (i, s) = e in i |> Instance.tint(s.tint)))
// Each finish's instances (as `groupedBy` gives) of `template` (the unit
// cube, or a house's gable): a draw per finish present.
let drawnAs = (template: Scene.t, groups: List<List<Instance.t>>): Scene.t =>
  Scene.group(all |> List.indexedMap((k, f) => (f, List.nth(k, groups) |> Option.defaultValue([])))
    |> List.filter((e) => let (f, is) = e in List.length(is) > 0.0)
    |> List.map((e) => let (f, is) = e in template |> Scene.litWorldTexture(f.texture, f.tile) |> Scene.instanced(is)))
let drawn = (groups: List<List<Instance.t>>): Scene.t => drawnAs(Scene.cube(), groups)
let instancedBy = (placed: List<(Instance.t, Surface)>): Scene.t => drawn(groupedBy(placed))
