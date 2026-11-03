// The building archetypes: each is a `Layout` (its `<name>Layout.fun`)
// derived ONCE, at load, into its interior (building.fun), culling cells
// (portals.fun) and street side (exterior.fun), and its committed bakes
// (baked/<bake>/) loaded into per-cell models. Every copy of an archetype
// (block.fun) draws these same values, so the GPU holds one of each. The
// towers (tower.fun) stack one typical floor's models, at three heights.

// An archetype's derived parts: its layout, colliders, cells and street
// side, and whether its bakes are current. `night` and `day`: its cell
// models in their cells under each bake (flat-lit without a current one).
type Archetype = {
  name: string,
  layout: Layout.Layout,
  colliders: List<Building.Collider>,
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
  let b = Provenance.bakedOf(k, s, status) in
  let night = b.models |> List.map((p) => roomOf(k, p)) in
  (night, night |> List.map((room) => Provenance.underDay(b.day, room)))

// Bake `name`, from `sources` (bake.mjs lists the same files), in
// baked/<name>/.
let bakeOf = (name: string, sources: List<string>): Provenance.Spec =>
  { dir: $"baked/{name}", sources: sources, bakes: ["night", "day"] }

// Each part is its own top-level value, so a hot reload re-derives only what
// an edit touched: a street-side edit keeps the interior and its bake.
let officeInterior = Building.interiorOf("office", OfficeLayout.layout)
let officeBake = bakeOf("office", ["layout.fun", "officeLayout.fun", "building.fun"])
let officeStatus = Provenance.statusOf(officeBake)
let officeRooms = roomsOf(officeInterior, officeBake, officeStatus)
let officeLevel = Portals.levelOf(OfficeLayout.layout, Portals.outsideTop)
let officeStreet = Exterior.streetOf(OfficeLayout.layout)
let office: Archetype =
  let (night, day) = officeRooms in
  { name: "office", layout: OfficeLayout.layout, colliders: officeInterior.colliders, status: officeStatus,
    level: officeLevel, street: officeStreet, night: night, day: day }

// The towers' two bakes, the typical floor's and the ground and roof's.
let towerSources = ["layout.fun", "towerLayout.fun", "building.fun"]
let typicalBake = bakeOf("tower-typical", towerSources)
let typicalStatus = Provenance.statusOf(typicalBake)
let typicalBaked = Provenance.bakedOf(Tower.typicalInterior, typicalBake, typicalStatus)
let endsBake = bakeOf("tower-ends", towerSources)
let endsStatus = Provenance.statusOf(endsBake)
let endsBaked = Provenance.bakedOf(Tower.endsInterior, endsBake, endsStatus)
// A tower of `n` typical floors.
let towerOf = (n: float): Archetype =>
  let l = TowerLayout.towerOf(n) in
  let placed = Tower.placedOf(n, typicalBaked, endsBaked) in
  { name: $"tower-{n}", layout: l, colliders: Tower.collidersOf(n),
    status: if typicalStatus != Provenance.Current then typicalStatus else endsStatus,
    level: Portals.levelOf(l, Tower.collarTop), street: Tower.streetOf(l, n),
    night: placed |> List.map(Tower.nightOf), day: placed |> List.map(Tower.dayOf) }
let tower8 = towerOf(8.0)
let tower12 = towerOf(12.0)
let tower16 = towerOf(16.0)

let all = [office, tower8, tower12, tower16]

// The archetype a copy names.
let named = (name: string): Archetype =>
  match all |> List.find((a) => a.name == name) with
  | Option.Some(a) => a
  | Option.None => Binary.invalid($"no archetype {name}")
// The first archetype whose bake isn't current, if any.
let stale = all |> List.find((a) => a.status != Provenance.Current)

// Every bake's jobs, for `functor bake` (through game.fun): `<bake>-night`
// and `<bake>-day`.
let bakes = [officeInterior.night, officeInterior.day, Tower.typicalInterior.night, Tower.typicalInterior.day,
             Tower.endsInterior.night, Tower.endsInterior.day]
// Whether every archetype's bake is current.
let baked = all |> List.all((a) => a.status == Provenance.Current)

// Every bake's cell models under both lightmaps, kept on the GPU but never
// drawn: with no interior active, or at the other hour, the meshes and
// lightmaps stay loaded, so entering a building or switching the hour
// uploads nothing. (The towers draw the same models, moved.)
let resident = Scene.keepResident(Scene.group(List.flatten([office.night, office.day, Tower.residentOf(typicalBaked), Tower.residentOf(endsBaked)])))

expect (bakes |> List.map((b) => b.name)) == ["office-night", "office-day", "tower-typical-night", "tower-typical-day", "tower-ends-night", "tower-ends-day"]
// The committed bakes match their layouts: rebake after editing (README).
expect all |> List.all((a) => a.status == Provenance.Current)
