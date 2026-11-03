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
let officeStreet = Streetscape.office(OfficeLayout.layout, Facade.office(OfficeLayout.layout))
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
    level: Portals.levelOf(l, Tower.collarTop), street: Streetscape.tower(l, Tower.streetOf(l, n)),
    night: placed |> List.map(Tower.nightOf), day: placed |> List.map(Tower.dayOf) }
let tower8 = towerOf(8.0)
let tower12 = towerOf(12.0)
let tower16 = towerOf(16.0)

// The cinema: its own palette's cell models (paints.fun), and its seats.
let cinemaBake = bakeOf("cinema", ["layout.fun", "building.fun", "paints.fun", "cinemaLayout.fun", "cinemaBuilding.fun"])
let cinemaStatus = Provenance.statusOf(cinemaBake)
let cinemaBaked = Paints.bakedOf(CinemaBuilding.palette, CinemaBuilding.interior, cinemaBake, cinemaStatus)
let cinema: Archetype =
  { name: "cinema", layout: CinemaLayout.layout, colliders: Cinema.colliders, status: cinemaStatus,
    level: Cinema.level, street: Cinema.exterior, night: Cinema.nightOf(cinemaBaked), day: Cinema.dayOf(cinemaBaked) }

// The mall: one bake at 6 texels/m (mallInterior.fun), its own cells,
// colliders and street side (mall.fun).
let mallBake = bakeOf("mall", ["layout.fun", "building.fun", "mallLayout.fun", "mallInterior.fun"])
let mallStatus = Provenance.statusOf(mallBake)
let mallBaked = MallInterior.bakedOf(mallBake, mallStatus)
let mall: Archetype =
  { name: "mall", layout: MallLayout.layout, colliders: Mall.colliders, status: mallStatus,
    level: Mall.level, street: Mall.street, night: Mall.nightOf(mallBaked), day: Mall.dayOf(mallBaked) }


// The house in two palettes (housePalettes.fun): one layout, cells and
// shape; two interiors, bakes and street sides.
let houseSources = ["layout.fun", "houseLayout.fun", "building.fun", "housePalettes.fun"]
let houseLevel = Portals.levelOf(HouseLayout.layout, Portals.outsideTop)
let houseColliders = HouseExterior.ownCollidersOf(HouseLayout.layout)
let houseOf = (name: string, k: Building.Interior, status: Provenance.Status, street: Exterior.Street, rooms: (List<Scene.t>, List<Scene.t>)): Archetype =>
  let (night, day) = rooms in
  { name: name, layout: HouseLayout.layout, colliders: List.append(k.colliders, houseColliders), status: status, level: houseLevel, street: street, night: night, day: day }
let ochreInterior = Building.interiorIn("house-ochre", HouseLayout.layout, HousePalettes.ochre)
let ochreBake = bakeOf("house-ochre", houseSources)
let ochreStatus = Provenance.statusOf(ochreBake)
let ochreRooms = roomsOf(ochreInterior, ochreBake, ochreStatus)
let ochreStreet = HouseExterior.streetOf(HouseLayout.layout, HouseExterior.ochre)
let houseOchre = houseOf("house-ochre", ochreInterior, ochreStatus, ochreStreet, ochreRooms)
let magentaInterior = Building.interiorIn("house-magenta", HouseLayout.layout, HousePalettes.magenta)
let magentaBake = bakeOf("house-magenta", houseSources)
let magentaStatus = Provenance.statusOf(magentaBake)
let magentaRooms = roomsOf(magentaInterior, magentaBake, magentaStatus)
let magentaStreet = HouseExterior.streetOf(HouseLayout.layout, HouseExterior.magenta)
let houseMagenta = houseOf("house-magenta", magentaInterior, magentaStatus, magentaStreet, magentaRooms)

let all = [office, tower8, tower12, tower16, cinema, mall, houseOchre, houseMagenta]

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
             Tower.endsInterior.night, Tower.endsInterior.day, CinemaBuilding.interior.night, CinemaBuilding.interior.day,
             MallInterior.interior.night, MallInterior.interior.day,
             ochreInterior.night, ochreInterior.day, magentaInterior.night, magentaInterior.day]
// Whether every archetype's bake is current.
let baked = all |> List.all((a) => a.status == Provenance.Current)

// Every bake's cell models under both lightmaps, kept on the GPU but never
// drawn: with no interior active, or at the other hour, the meshes and
// lightmaps (and each street side's dressing) stay loaded, so entering a building or switching the hour
// uploads nothing. (The towers draw the same models, moved.)
let resident = Scene.keepResident(Scene.group(List.flatten([office.night, office.day, Tower.residentOf(typicalBaked), Tower.residentOf(endsBaked),
                                                         Tower.residentOf(cinemaBaked), Cinema.fixed, Tower.residentOf(mallBaked),
                                                         houseOchre.night, houseOchre.day, houseMagenta.night, houseMagenta.day,
                                                         [Cinema.lotDressing.night, Cinema.lotDressing.day],
                                                         all |> List.concatMap((a) => [a.street.dressing.night, a.street.dressing.day])])))

expect (bakes |> List.map((b) => b.name)) == ["office-night", "office-day", "tower-typical-night", "tower-typical-day", "tower-ends-night", "tower-ends-day", "cinema-night", "cinema-day", "mall-night", "mall-day",
  "house-ochre-night", "house-ochre-day", "house-magenta-night", "house-magenta-day"]
// The committed bakes match their layouts: rebake after editing (README).
expect all |> List.all((a) => a.status == Provenance.Current)
