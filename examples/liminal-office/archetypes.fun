// The building archetypes: each is a `Layout` (its `<name>Layout.fun`)
// derived ONCE, at load, into its interior (building.fun), culling cells
// (portals.fun) and street side (exterior.fun), and its committed bake
// (baked/<name>/) loaded into per-cell models. Every copy of an archetype
// (block.fun) draws these same values, so the GPU holds one of each.

// An archetype's derived parts. `night` and `day`: its cell models in their
// cells under each bake (flat-lit without a current one).
type Archetype = {
  name: string,
  layout: Layout.Layout,
  interior: Building.Interior,
  bake: Provenance.Spec,
  status: Provenance.Status,
  level: Cells.t,
  street: Exterior.Street,
  night: List<Scene.t>,
  day: List<Scene.t>,
}

// A cell model drawn in its cell. (`k` is for the e2e's draw-count variants,
// which patch this body to draw `k`'s groups.)
let roomOf = (k: Building.Interior, p: Building.CellModel): Scene.t => Scene.modelOf(p.model) |> Scene.inCells([p.cell])

// The baked mesh cut into cell models, under the night bake, and the same
// models under the day bake: one mesh, two lightmaps.
let roomsOf = (k: Building.Interior, s: Provenance.Spec, status: Provenance.Status): (List<Scene.t>, List<Scene.t>) =>
  match status with
  | Provenance.Current =>
    let mesh = Provenance.meshPathOf(s) |> File.bytes |> Lightmap.readMesh in
    let night = Building.cellModels(k, { mesh with lightmap: Option.Some({
      texture: Texture.fromPng(File.bytes(Provenance.lightPathOf(s, "night")), Texture.Srgb),
      scale: Building.bakeScale, floor: Color.rgb(0.0, 0.0, 0.0) }) }) |> List.map((p) => roomOf(k, p)) in
    let dayLightmap = Texture.fromPng(File.bytes(Provenance.lightPathOf(s, "day")), Texture.Srgb) in
    (night, night |> List.map((room) => room |> Scene.withLightmap(dayLightmap)))
  | _ =>
    let night = Building.cellModels(k, k.interiorData) |> List.map((p) => roomOf(k, p)) in
    (night, night)

// Archetype `name`'s bake, from `sources` (bake.mjs lists the same files),
// in baked/<name>/.
let bakeOf = (name: string, sources: List<string>): Provenance.Spec =>
  { dir: $"baked/{name}", sources: sources, bakes: ["night", "day"] }

// Each part is its own top-level value, so a hot reload re-derives only what
// an edit touched: a street-side edit keeps the interior and its bake.
let officeInterior = Building.interiorOf("office", OfficeLayout.layout)
let officeBake = bakeOf("office", ["layout.fun", "officeLayout.fun", "building.fun"])
let officeStatus = Provenance.statusOf(officeBake)
let officeRooms = roomsOf(officeInterior, officeBake, officeStatus)
let officeLevel = Portals.levelOf(OfficeLayout.layout)
let officeStreet = Exterior.streetOf(OfficeLayout.layout)
let office: Archetype =
  let (night, day) = officeRooms in
  { name: "office", layout: OfficeLayout.layout, interior: officeInterior, bake: officeBake, status: officeStatus,
    level: officeLevel, street: officeStreet, night: night, day: day }

let all = [office]

// The archetype a copy names.
let named = (name: string): Archetype =>
  match all |> List.find((a) => a.name == name) with
  | Option.Some(a) => a
  | Option.None => Binary.invalid($"no archetype {name}")
// The first archetype whose bake isn't current, if any.
let stale = all |> List.find((a) => a.status != Provenance.Current)

// Every archetype's bake jobs, for `functor bake` (through game.fun):
// `<name>-night` and `<name>-day`.
let bakes = all |> List.concatMap((a) => [a.interior.night, a.interior.day])
// Whether every archetype's bake is current.
let baked = all |> List.all((a) => a.status == Provenance.Current)

// Every archetype's cell models under both bakes, kept on the GPU but never
// drawn: with no interior active, or at the other hour, the meshes and
// lightmaps stay loaded, so entering a building or switching the hour
// uploads nothing.
let resident = Scene.keepResident(Scene.group(all |> List.concatMap((a) => List.flatten([a.night, a.day]))))

expect (bakes |> List.map((b) => b.name)) == ["office-night", "office-day"]
// The committed bakes match their layouts: rebake after editing (README).
expect all |> List.all((a) => a.status == Provenance.Current)
