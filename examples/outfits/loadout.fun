// Loadout DATA: a body plus what each slot wears. It is slot-shaped so part
// toggles can later replace the whole-glb presets without changing it; today
// a loadout renders only when it matches a preset (`outfitOf`).

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

// A loadout's one item per slot, at most one entry each.
let isValid = (kit: Kit): bool =>
  List.length(kit.wear) == List.length(slots)
    && (slots |> List.all((s) => (kit.wear |> List.filter((w) => w.slot == s) |> List.length) == 1.0))

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

// Preset glbs at LOD 0/1/2 (ASSETS.md).
let lods = (body: Body, outfit: Outfit): List<Asset.Model> =>
  match body with
  | Male => (
    match outfit with
    | Bare => [Assets.male_base_lod0, Assets.male_base_lod1, Assets.male_base_lod2]
    | Ranger => [Assets.male_ranger_lod0, Assets.male_ranger_lod1, Assets.male_ranger_lod2]
    | Peasant => [Assets.male_peasant_lod0, Assets.male_peasant_lod1, Assets.male_peasant_lod2]
  )
  | Female => (
    match outfit with
    | Bare => [Assets.female_base_lod0, Assets.female_base_lod1, Assets.female_base_lod2]
    | Ranger => [Assets.female_ranger_lod0, Assets.female_ranger_lod1, Assets.female_ranger_lod2]
    | Peasant => [Assets.female_peasant_lod0, Assets.female_peasant_lod1, Assets.female_peasant_lod2]
  )

// The glb for a preset loadout at `lod`; a non-preset loadout shows the bare body.
let asset = (kit: Kit, lod: float): Asset.Model =>
  lods(kit.body, outfitOf(kit) |> Option.defaultValue(Bare))
    |> List.nth(lod)
    |> Option.defaultValue(Assets.male_base_lod0)

// The glb file stem, e.g. "female_ranger_lod1".
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
