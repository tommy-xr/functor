// Loadout DATA: a body plus what each slot wears. It is slot-shaped so
// per-slot toggles can fill it later; today a loadout draws only when it
// matches a preset (`outfitOf`), as that preset's parts.

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

type Worn = { slot: Slot, item: Outfit }

type Kit = { body: Body, wear: List<Worn> }

let slots = [Head, Torso, Arms, Legs, Feet]

let outfits = [Bare, Ranger, Peasant]

let bodies = [Male, Female]

// Which slots each free outfit fills: the Ranger's hood is its Head piece;
// pauldrons ride on Torso. Bare fills none.
let covers = (outfit: Outfit): List<Slot> =>
  match outfit with
  | Bare => []
  | Ranger => [Head, Torso, Arms, Legs, Feet]
  | Peasant => [Torso, Arms, Legs, Feet]

// The loadout a preset picker produces: `outfit` in every slot it covers.
let dress = (body: Body, outfit: Outfit): Kit =>
  {
    body: body,
    wear: slots |> List.map((s) => {
      slot: s,
      item: if covers(outfit) |> List.any((c) => c == s) then outfit else Bare,
    }),
  }

// Every preset the picker offers: 2 bodies × 3 outfits.
let presets = bodies |> List.concatMap((b) => outfits |> List.map((o) => dress(b, o)))

// The preset a loadout matches, if any (a mixed loadout needs part toggles).
let outfitOf = (kit: Kit): Option.t<Outfit> =>
  outfits |> List.find((o) => dress(kit.body, o) == kit)

// The preset outfit, or Bare for a loadout no preset matches.
let outfitOrBare = (kit: Kit): Outfit => outfitOf(kit) |> Option.defaultValue(Bare)

// A valid loadout wears exactly one item per slot.
let isValid = (kit: Kit): bool =>
  slots |> List.all((s) => (kit.wear |> List.filter((w) => w.slot == s) |> List.length) == 1.0)

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

// The glbs at LOD 0/1/2: one per body, holding every piece as a part (ASSETS.md).
let lods = (body: Body): List<Asset.Model> =>
  match body with
  | Male => [Assets.male_lod0, Assets.male_lod1, Assets.male_lod2]
  | Female => [Assets.female_lod0, Assets.female_lod1, Assets.female_lod2]

let asset = (kit: Kit, lod: float): Asset.Model =>
  lods(kit.body) |> List.nth(lod) |> Option.defaultValue(Assets.male_lod0)

// Part names both bodies share; only the hairstyles differ.
let shared = Assets.male_lod0Parts

// The face draws in every loadout.
let face = [shared.body_head, shared.eyes, shared.eyebrows]

// A preset's hair: none under the Ranger's hood, else the body's default style.
let hairOf = (body: Body, outfit: Outfit): List<string> =>
  match outfit with
  | Ranger => []
  | _ => (
    match body with
    | Male => [Assets.male_lod0Parts.hair_simple_parted]
    | Female => [Assets.female_lod0Parts.hair_buns]
  )

// An outfit's pieces; Bare shows every body region instead.
let outfitParts = (outfit: Outfit): List<string> =>
  match outfit with
  | Bare => [shared.body_torso, shared.body_arms, shared.body_legs, shared.body_shins, shared.body_feet]
  | Ranger => [shared.ranger_hood, shared.ranger_pauldrons, shared.ranger_body, shared.ranger_arms, shared.ranger_legs, shared.ranger_feet]
  | Peasant => [shared.peasant_body, shared.peasant_arms, shared.peasant_legs, shared.peasant_feet]

// The parts a loadout draws (`Scene.modelParts`); a non-preset loadout draws bare.
let parts = (kit: Kit): List<string> =>
  let outfit = outfitOrBare(kit) in
  face |> List.append(hairOf(kit.body, outfit)) |> List.append(outfitParts(outfit))

// A preset's label, e.g. "female_ranger_lod1".
let presetName = (kit: Kit, lod: float): string =>
  match outfitOf(kit) with
  | Option.Some(o) => $"{bodyName(kit.body)}_{outfitName(o)}_lod{lod}"
  | Option.None => $"{bodyName(kit.body)}_mixed_lod{lod}"

expect List.length(presets) == 6.0
expect presets |> List.all(isValid)
expect presets |> List.all((k) => outfitOf(k) != Option.None)
expect outfitOf(dress(Female, Peasant)) == Option.Some(Peasant)
expect outfitOf(dress(Male, Bare)) == Option.Some(Bare)
// Peasant has no hood: its Head slot stays bare.
expect (dress(Male, Peasant).wear |> List.find((w) => w.slot == Head)) == Option.Some({ slot: Head, item: Bare })
// A mixed loadout is valid data but no preset matches it.
expect (
  let mixed = { body: Male, wear: slots |> List.map((s) => { slot: s, item: if s == Feet then Peasant else Ranger }) } in
  isValid(mixed) && outfitOf(mixed) == Option.None
)
expect not isValid({ body: Male, wear: [{ slot: Head, item: Bare }] })
expect presetName(dress(Female, Ranger), 1.0) == "female_ranger_lod1"
expect presetName(dress(Male, Bare), 2.0) == "male_base_lod2"
expect parts(dress(Male, Ranger)) == [shared.body_head, shared.eyes, shared.eyebrows, shared.ranger_hood, shared.ranger_pauldrons, shared.ranger_body, shared.ranger_arms, shared.ranger_legs, shared.ranger_feet]
expect parts(dress(Female, Peasant)) |> List.any((p) => p == Assets.female_lod0Parts.hair_buns)
expect List.length(parts(dress(Male, Bare))) == 9.0
// A mixed loadout draws bare.
expect (
  let mixed = { body: Female, wear: slots |> List.map((s) => { slot: s, item: if s == Feet then Peasant else Ranger }) } in
  parts(mixed) == parts(dress(Female, Bare))
)
