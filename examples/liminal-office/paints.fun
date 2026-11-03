// building.fun's mesh pipeline over an archetype's own palette (the
// cinema's). A paint is what a face shows: a look's texture repeating every
// `tile` metres, stretched once over the face (0), or one cell of an atlas
// stretched over it. A look is a texture and a draw: paints sharing a look
// (the cinema's signs, posters, neon and lenses, all one atlas) share a
// group, so a cell draws one group per look in it. Unlit looks take no
// light, so their faces stay out of the bake (no lightmap texels): each cell
// draws a baked model of its lit faces and a plain one of its unlit ones.
// Faces are building.fun's, their `mat` a paint. Editing this file makes the
// cinema's committed bake stale: rebake (README).

// `albedo`: the share of light it reflects in the bake.
type Look = { texture: Asset.Texture, unlit: bool, albedo: Color.t }
type Paint = { look: float, tile: float, cell: Option.t<Texture.rect> }
type Palette = { looks: List<Look>, paints: List<Paint> }

let paintAt = (p: Palette, i: float): Paint =>
  match List.nth(i, p.paints) with
  | Option.Some(x) => x
  | Option.None => Binary.invalid($"no paint {i}")
let lookAt = (p: Palette, i: float): Look =>
  match List.nth(i, p.looks) with
  | Option.Some(x) => x
  | Option.None => Binary.invalid($"no look {i}")
let lookOf = (p: Palette, f: Building.Face): float => paintAt(p, f.mat).look
let isUnlit = (p: Palette, f: Building.Face): bool => lookAt(p, lookOf(p, f)).unlit

// Texture UVs: a cell's rect, stretched; else building.fun's.
let uvsOf = (p: Palette, f: Building.Face): List<float> =>
  let paint = paintAt(p, f.mat) in
  match paint.cell with
  | Option.Some(r) => [r.u0, r.v1, r.u1, r.v1, r.u1, r.v0, r.u0, r.v0]
  | Option.None =>
    if paint.tile == 0.0 then [0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]
    else
      let au = Vec3.normalize(f.q.a) in
      let bu = Vec3.normalize(f.q.b) in
      Building.corners(f.q) |> List.concatMap((c) => [Vec3.dot(au, c) / paint.tile, 0.0 - Vec3.dot(bu, c) / paint.tile])

// A cell's faces in draw order: by look.
let sorted = (p: Palette, faces: List<Building.Face>): List<Building.Face> => faces |> List.sortBy((f) => lookOf(p, f))

// One span per look present in a cell's (sorted) faces, `first` counted
// from `base`; its `mat` is the look.
let spansOf = (p: Palette, base: float, faces: List<Building.Face>): List<Building.Span> =>
  let (spans, next) = List.range(List.length(p.looks)) |> List.fold((acc, m) =>
    let (out, first) = acc in
    let n = 2.0 * List.length(faces |> List.filter((f) => lookOf(p, f) == m)) in
    if n == 0.0 then acc
    else (out |> List.append([{ mat: m, first: first, count: n }]), first + n), ([], base)) in
  spans
let groupOf = (p: Palette, s: Building.Span): Model.group =>
  { first: s.first, count: s.count, texture: Option.Some(lookAt(p, s.mat).texture), unlit: lookAt(p, s.mat).unlit, emissive: 0.0 }

let meshOf = (p: Palette, faces: List<Building.Face>, spans: List<Building.Span>): Model.staticData => {
  positions: Building.positionsOf(faces),
  normals: Bytes.fromF32le(faces |> List.concatMap((f) =>
    let n = Building.xyz(Building.normalOf(f.q)) in List.flatten([n, n, n, n]))),
  uvs: Bytes.fromF32le(faces |> List.concatMap((f) => uvsOf(p, f))),
  uvs2: Bytes.fromF32le([]),
  triangles: Building.quadTriangles(List.length(faces)),
  groups: spans |> List.map((s) => groupOf(p, s)),
  lightmap: Option.None
}

// building.fun's `interiorOf` over `cellParts`' lit faces (each cell's
// faces, in cell order), with the caller's bake lights; no colliders (the
// archetype builds its own).
let interiorOf = (p: Palette, name: string, cellParts: List<List<Building.Face>>, nightLights: List<Light.t>, dayLights: List<Light.t>): Building.Interior =>
  let parts = cellParts |> List.map((faces) => sorted(p, faces |> List.filter((f) => not isUnlit(p, f)))) in
  let allFaces = List.flatten(parts) in
  let cellRanges = Building.rangesOf(parts) in
  let meshSpans = parts |> List.zip(cellRanges) |> List.concatMap((pair) =>
    let (faces, range) = pair in spansOf(p, range.first, faces)) in
  let interiorData = meshOf(p, allFaces, meshSpans) in
  let night: Lightmap.bakeJob = {
    name: $"{name}-night",
    mesh: interiorData,
    lights: nightLights,
    albedo: meshSpans |> List.map((s) => lookAt(p, s.mat).albedo),
    scale: Building.bakeScale,
  } in
  { cellParts: parts, allFaces: allFaces, cellRanges: cellRanges, meshSpans: meshSpans,
    interiorData: interiorData, blocks: [], colliders: [],
    night: night, day: { night with name: $"{name}-day", lights: dayLights } }

// building.fun's cut into one static model per cell (its `cellData`), with
// this palette's groups.
let cellModels = (p: Palette, k: Building.Interior, data: Model.staticData): List<Building.CellModel> =>
  Building.runsOf(k, data) |> List.map((run) => { cell: run.cell, model: Model.static({ Building.cellData(k, data, run) with
    groups: spansOf(p, 0.0, List.nth(run.cell, k.cellParts) |> Option.defaultValue([])) |> List.map((s) => groupOf(p, s)) }) })

// `cellParts`' unlit faces: a plain static model per cell that has any.
let unlitModels = (p: Palette, cellParts: List<List<Building.Face>>): List<Building.CellModel> =>
  cellParts |> List.indexedMap((i, faces) => (i, sorted(p, faces |> List.filter((f) => isUnlit(p, f)))))
    |> List.filter((e) => let (i, faces) = e in List.length(faces) > 0.0)
    |> List.map((e) => let (i, faces) = e in { cell: i, model: Model.static(meshOf(p, faces, spansOf(p, 0.0, faces))) })

// provenance.fun's `bakedOf` with this palette's groups.
let bakedOf = (p: Palette, k: Building.Interior, s: Provenance.Spec, status: Provenance.Status): Provenance.Baked =>
  match status with
  | Provenance.Current =>
    let mesh = Provenance.meshPathOf(s) |> File.bytes |> Lightmap.readMesh in
    { models: cellModels(p, k, { mesh with lightmap: Option.Some({
        texture: Texture.fromPng(File.bytes(Provenance.lightPathOf(s, "night")), Texture.Srgb),
        scale: Building.bakeScale, floor: Color.rgb(0.0, 0.0, 0.0) }) }),
      day: Option.Some(Texture.fromPng(File.bytes(Provenance.lightPathOf(s, "day")), Texture.Srgb)) }
  | _ => { models: cellModels(p, k, k.interiorData), day: Option.None }
