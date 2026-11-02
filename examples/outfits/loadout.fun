// Loadout DATA: a body, its hair, beard and brows, and what each slot wears.
// A character draws its body's glb with only the loadout's parts (`parts` →
// `Scene.modelParts`; ASSETS.md):
// - each worn slot draws its item's pieces, and pieces mix across outfits;
// - a body region draws unless the slots over it are worn (`bodyParts`);
// - hair draws unless the hood is up.
// Only options a glb lacks are invalid (`isValid`): the Peasant has no hood,
// each body has its own hairstyles, and only the male has a beard.

type Body =
  | Male
  | Female

type Slot =
  | Head
  | Torso
  | Arms
  | Legs
  | Feet

type Outfit =
  | Bare
  | Ranger
  | Peasant

type Hair =
  | Bald
  | Buzzed
  | Parted
  | Buns
  | Long

type Worn = { slot: Slot, item: Outfit }

type Kit = { body: Body, hair: Hair, beard: bool, brows: bool, wear: List<Worn> }

let slots = [Head, Torso, Arms, Legs, Feet]

let outfits = [Bare, Ranger, Peasant]

let bodies = [Male, Female]

let contains = (xs, x): bool => xs |> List.any((y) => y == x)

// The items a slot offers: the Ranger's hood is its Head piece; the Peasant
// has none.
let itemsFor = (slot: Slot): List<Outfit> =>
  match slot with
  | Head => [Bare, Ranger]
  | _ => outfits

// The hairstyles a body offers: each is fitted to one head.
let hairsFor = (body: Body): List<Hair> =>
  match body with
  | Male => [Bald, Buzzed, Parted]
  | Female => [Bald, Buzzed, Buns, Long]

let defaultHair = (body: Body): Hair =>
  match body with
  | Male => Parted
  | Female => Buns

// A whole outfit: `outfit` in every slot that offers it, Bare elsewhere.
let dress = (body: Body, outfit: Outfit): Kit =>
  {
    body: body,
    hair: defaultHair(body),
    beard: false,
    brows: true,
    wear: slots |> List.map((s) => { slot: s, item: if contains(itemsFor(s), outfit) then outfit else Bare }),
  }

// `kit` on `body`, keeping its slots, and its hair and beard where that body
// offers them. Every slot item fits both bodies.
let rebody = (kit: Kit, body: Body): Kit =>
  {
    kit with
    body: body,
    hair: if contains(hairsFor(body), kit.hair) then kit.hair else defaultHair(body),
    beard: kit.beard && body == Male,
  }

// `kit` dressed whole in `outfit`, keeping its look.
let redress = (kit: Kit, outfit: Outfit): Kit => { kit with wear: dress(kit.body, outfit).wear }

// Every whole outfit on every body: 2 × 3.
let presets = bodies |> List.concatMap((b) => outfits |> List.map((o) => dress(b, o)))

// The whole outfit a loadout wears, if any (a mixed loadout wears none).
let outfitOf = (kit: Kit): Option.t<Outfit> =>
  outfits |> List.find((o) => dress(kit.body, o).wear == kit.wear)

let outfitOrBare = (kit: Kit): Outfit => outfitOf(kit) |> Option.defaultValue(Bare)

let itemOf = (kit: Kit, slot: Slot): Outfit =>
  kit.wear |> List.find((w) => w.slot == slot) |> Option.map((w) => w.item) |> Option.defaultValue(Bare)

let wears = (kit: Kit, slot: Slot): bool => itemOf(kit, slot) != Bare

// `kit` with `slot` wearing `item`.
let wear = (kit: Kit, slot: Slot, item: Outfit): Kit =>
  { kit with wear: kit.wear |> List.map((w) => if w.slot == slot then { slot: slot, item: item } else w) }

// One item per slot, each one the slot offers, and a look the body has.
let isValid = (kit: Kit): bool =>
  (slots |> List.all((s) => (kit.wear |> List.filter((w) => w.slot == s) |> List.length) == 1.0))
    && (kit.wear |> List.all((w) => contains(itemsFor(w.slot), w.item)))
    && contains(hairsFor(kit.body), kit.hair)
    && (not kit.beard || kit.body == Male)

let bodyName = (body: Body): string =>
  match body with
  | Male => "male"
  | Female => "female"

let outfitName = (outfit: Outfit): string =>
  match outfit with
  | Bare => "base"
  | Ranger => "ranger"
  | Peasant => "peasant"

let slotName = (slot: Slot): string =>
  match slot with
  | Head => "head"
  | Torso => "torso"
  | Arms => "arms"
  | Legs => "legs"
  | Feet => "feet"

let hairName = (hair: Hair): string =>
  match hair with
  | Bald => "bald"
  | Buzzed => "buzzed"
  | Parted => "parted"
  | Buns => "buns"
  | Long => "long"

// The glbs at LOD 0/1/2: one per body, holding every piece as a part (ASSETS.md).
let lods = (body: Body): List<Asset.Model> =>
  match body with
  | Male => [Assets.male_lod0, Assets.male_lod1, Assets.male_lod2]
  | Female => [Assets.female_lod0, Assets.female_lod1, Assets.female_lod2]

let asset = (kit: Kit, lod: float): Asset.Model =>
  lods(kit.body) |> List.nth(lod) |> Option.defaultValue(Assets.male_lod0)

// A loadout's label: its whole outfit, e.g. "female_ranger_lod1", or "mixed".
let presetName = (kit: Kit, lod: float): string =>
  match outfitOf(kit) with
  | Option.Some(o) => $"{bodyName(kit.body)}_{outfitName(o)}_lod{lod}"
  | Option.None => $"{bodyName(kit.body)}_mixed_lod{lod}"

// --- Parts ------------------------------------------------------------------

// Part names both bodies share; only the hairstyles differ. Every LOD has
// LOD0's names.
let shared = Assets.male_lod0Parts

let onlyIf = (cond: bool, xs: List<string>): List<string> => if cond then xs else []

// A slot's pieces in `item`. Pauldrons ride on the Ranger's torso.
let slotParts = (slot: Slot, item: Outfit): List<string> =>
  match item with
  | Bare => []
  | Ranger => (
    match slot with
    | Head => [shared.ranger_hood]
    | Torso => [shared.ranger_body, shared.ranger_pauldrons]
    | Arms => [shared.ranger_arms]
    | Legs => [shared.ranger_legs]
    | Feet => [shared.ranger_feet]
  )
  | Peasant => (
    match slot with
    | Head => []
    | Torso => [shared.peasant_body]
    | Arms => [shared.peasant_arms]
    | Legs => [shared.peasant_legs]
    | Feet => [shared.peasant_feet]
  )

// The body regions that show. Each hides under its slot's piece; the shins
// hide only under both trousers and boots, which leave them showing alone.
let bodyParts = (kit: Kit): List<string> =>
  let bare = (slot) => not wears(kit, slot) in
  [shared.body_head]
    |> List.append(onlyIf(bare(Torso), [shared.body_torso]))
    |> List.append(onlyIf(bare(Arms), [shared.body_arms]))
    |> List.append(onlyIf(bare(Legs), [shared.body_legs]))
    |> List.append(onlyIf(bare(Legs) || bare(Feet), [shared.body_shins]))
    |> List.append(onlyIf(bare(Feet), [shared.body_feet]))

let hairParts = (body: Body, hair: Hair): List<string> =>
  match body with
  | Male => (
    let p = Assets.male_lod0Parts in
    match hair with
    | Buzzed => [p.hair_buzzed]
    | Parted => [p.hair_simple_parted]
    | _ => []
  )
  | Female => (
    let p = Assets.female_lod0Parts in
    match hair with
    | Buzzed => [p.hair_buzzed]
    | Buns => [p.hair_buns]
    | Long => [p.hair_long]
    | _ => []
  )

let beardParts = (body: Body): List<string> =>
  match body with
  | Male => [Assets.male_lod0Parts.hair_beard]
  | Female => []

// The parts a loadout draws (`Scene.modelParts`).
let parts = (kit: Kit): List<string> =>
  bodyParts(kit)
    |> List.append([shared.eyes])
    |> List.append(onlyIf(kit.brows, [shared.eyebrows]))
    |> List.append(onlyIf(kit.beard, beardParts(kit.body)))
    |> List.append(onlyIf(not wears(kit, Head), hairParts(kit.body, kit.hair)))
    |> List.append(kit.wear |> List.concatMap((w) => slotParts(w.slot, w.item)))

// Every part of a body's glb, listed by hand from its typed record: the tests
// check each option selects only these, and some option selects each one. A
// part a rebuild adds must be added here.
let inventory = (body: Body): List<string> =>
  let s = shared in
  let common = [
    s.body_head, s.body_torso, s.body_arms, s.body_legs, s.body_shins, s.body_feet, s.eyes, s.eyebrows,
    s.hair_buzzed, s.ranger_hood, s.ranger_pauldrons, s.ranger_body, s.ranger_arms, s.ranger_legs,
    s.ranger_feet, s.peasant_body, s.peasant_arms, s.peasant_legs, s.peasant_feet,
  ] in
  match body with
  | Male => common |> List.append([Assets.male_lod0Parts.hair_simple_parted, Assets.male_lod0Parts.hair_beard])
  | Female => common |> List.append([Assets.female_lod0Parts.hair_buns, Assets.female_lod0Parts.hair_long])

// The parts every option can select, valid or not.
let reachable = (body: Body): List<string> =>
  bodyParts(dress(body, Bare))
    |> List.append([shared.eyes, shared.eyebrows])
    |> List.append(beardParts(body))
    |> List.append(hairsFor(body) |> List.concatMap((h) => hairParts(body, h)))
    |> List.append(slots |> List.concatMap((s) => itemsFor(s) |> List.concatMap((i) => slotParts(s, i))))

let has = (kit: Kit, part: string): bool => contains(parts(kit), part)

// Ranger hood and boots over Peasant clothes, no brows.
let hoodedPeasant =
  { wear(wear(dress(Male, Peasant), Head, Ranger), Feet, Ranger) with brows: false }

expect List.length(presets) == 6.0
expect presets |> List.all(isValid)
expect presets |> List.all((k) => outfitOf(k) != Option.None)
expect outfitOf(dress(Female, Peasant)) == Option.Some(Peasant)
expect outfitOf(dress(Male, Bare)) == Option.Some(Bare)
// The Peasant has no hood: its Head slot stays bare.
expect itemOf(dress(Male, Peasant), Head) == Bare
expect presetName(dress(Female, Ranger), 1.0) == "female_ranger_lod1"
expect presetName(dress(Male, Bare), 2.0) == "male_base_lod2"

// Mixing outfits is valid; it just wears no whole outfit.
expect isValid(hoodedPeasant) && outfitOf(hoodedPeasant) == Option.None
expect presetName(hoodedPeasant, 0.0) == "male_mixed_lod0"
expect parts(hoodedPeasant) == [
  shared.body_head, shared.eyes, shared.ranger_hood, shared.peasant_body, shared.peasant_arms, shared.peasant_legs,
  shared.ranger_feet,
]
// Only what a glb lacks is invalid.
expect not isValid(wear(dress(Male, Bare), Head, Peasant))
expect not isValid({ dress(Female, Bare) with hair: Parted })
expect not isValid({ dress(Female, Bare) with beard: true })
expect isValid({ dress(Male, Ranger) with hair: Bald, beard: true, brows: false })
expect not isValid({ dress(Male, Bare) with wear: [{ slot: Head, item: Bare }] })

// Every option resolves to real parts of its body's glb, and every part is
// some option's.
expect bodies |> List.all((b) => slots |> List.all((s) => itemsFor(s) |> List.all((i) =>
  (i == Bare) == List.isEmpty(slotParts(s, i)) && (slotParts(s, i) |> List.all((p) => contains(inventory(b), p))))))
expect bodies |> List.all((b) => hairsFor(b) |> List.all((h) =>
  (h == Bald) == List.isEmpty(hairParts(b, h)) && (hairParts(b, h) |> List.all((p) => contains(inventory(b), p)))))
expect bodies |> List.all((b) =>
  (reachable(b) |> List.all((p) => contains(inventory(b), p))) && (inventory(b) |> List.all((p) => contains(reachable(b), p))))
// A dressed body draws no part twice.
expect presets |> List.all((k) => parts(k) |> List.all((p) => (parts(k) |> List.filter((q) => q == p) |> List.length) == 1.0))

// Coverage: each region hides under its slot.
expect bodyParts(dress(Female, Bare)) == [
  shared.body_head, shared.body_torso, shared.body_arms, shared.body_legs, shared.body_shins, shared.body_feet,
]
expect bodyParts(dress(Male, Ranger)) == [shared.body_head]
expect not has(wear(dress(Male, Bare), Torso, Peasant), shared.body_torso)
expect not has(wear(dress(Male, Bare), Arms, Ranger), shared.body_arms)
// Shins hide only under both trousers and boots.
expect has(wear(dress(Male, Bare), Legs, Peasant), shared.body_shins)
expect has(wear(dress(Male, Bare), Feet, Ranger), shared.body_shins)
expect not has(wear(wear(dress(Male, Bare), Legs, Peasant), Feet, Ranger), shared.body_shins)
// Hair hides under the hood; the beard and brows don't.
expect has(dress(Female, Peasant), Assets.female_lod0Parts.hair_buns)
expect not has(dress(Female, Ranger), Assets.female_lod0Parts.hair_buns)
expect (let k = { dress(Male, Ranger) with beard: true } in has(k, Assets.male_lod0Parts.hair_beard) && has(k, shared.eyebrows))
expect not has({ dress(Male, Bare) with brows: false }, shared.eyebrows)
expect not has({ dress(Male, Bare) with hair: Bald }, Assets.male_lod0Parts.hair_simple_parted)

// Re-dressing keeps the look; a body change keeps the slots, and the look the
// body offers.
expect (let k = redress({ dress(Female, Bare) with hair: Long, brows: false }, Ranger) in k.hair == Long && not k.brows)
expect rebody(hoodedPeasant, Female).wear == hoodedPeasant.wear
expect rebody({ dress(Female, Bare) with hair: Long }, Male).hair == Parted
expect not rebody({ dress(Male, Bare) with beard: true }, Female).beard
expect rebody({ dress(Male, Bare) with hair: Buzzed }, Female).hair == Buzzed
