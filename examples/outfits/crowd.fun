// Crowd layout: `count` characters on rows that arc around the camera's
// ground point. Distance from the camera's eye picks the tier:
//   near (< nearRadius): LOD0, each individually posed;
//   mid (< midRadius):   LOD1, each individually posed;
//   far:                 LOD2, one `Scene.instanced` batch per (preset, phase
//                        bucket). Copies in a batch share ONE pose, so far
//                        characters with the same preset and bucket move in
//                        lockstep: at most farBuckets distinct poses per preset.
// Everything here is plain data, computed once per `count` change.

type Tier =
  | Near
  | Mid
  | Far

type Seat = { x: float, z: float, yaw: float }

// A character's static look: the memo deps for its subtree.
type Look = { kit: Loadout.Kit, lod: float }

type Member = {
  key: string,
  seat: Seat,
  tier: Tier,
  look: Look,
  phase: float,
  run: float,
}

type Batch = {
  key: string,
  look: Look,
  bucket: float,
  seats: List<Seat>,
}

type Layout = { members: List<Member>, batches: List<Batch> }

let firstRadius = 5.0

let rowGap = 1.3

let spacing = 1.0

// Rows span this arc (radians) in front of the camera.
let arc = 1.05

// The crowd camera: above the arcs' center, so every seat in a row is the
// same distance away.
let eye = { x: 0.0, y: 3.0, z: 0.0 }

// Distance is measured to a character's chest.
let chest = 0.9

let nearRadius = 7.0

let midRadius = 12.0

let farBuckets = 4.0

// A deterministic per-index fraction in [0, 1): the golden-ratio sequence.
let fraction = (i: float): float => Math.mod(i * 0.618034, 1.0)

let tierOf = (distance: float): Tier =>
  if distance < nearRadius then Near else if distance < midRadius then Mid else Far

let lodOf = (tier: Tier): float =>
  match tier with
  | Near => 0.0
  | Mid => 1.0
  | Far => 2.0

// Row k: its radius and how many characters fit along the arc at `spacing`.
let rows = List.range(14.0) |> List.map((k) =>
  let radius = firstRadius + k * rowGap in
  { radius: radius, size: Math.floor(arc * radius / spacing) })

// Seats in row order (nearest first), each turned to face the camera.
let seats =
  rows |> List.concatMap((row) =>
    List.range(row.size) |> List.map((j) =>
      let angle = (j - (row.size - 1.0) / 2.0) * spacing / row.radius in
      { x: eye.x + row.radius * Math.sin(angle), z: eye.z + row.radius * Math.cos(angle), yaw: angle + Math.pi }))

let distanceOf = (seat: Seat): float =>
  Vec3.length(Vec3.make(seat.x - eye.x, chest - eye.y, seat.z - eye.z))

// Character i's preset: alternating bodies, outfits mostly clothed.
let kitOf = (i: float): Loadout.Kit =>
  let body = if Math.mod(i, 2.0) == 0.0 then Loadout.Male else Loadout.Female in
  let pick = fraction(i * 3.0 + 0.25) in
  Loadout.dress(body, if pick < 0.45 then Loadout.Ranger else if pick < 0.85 then Loadout.Peasant else Loadout.Bare)

// Walk/run blend inputs, as in examples/crowd: a playhead offset in [0, 2) s
// and a run weight in [0.25, 0.75], so neither blend entry drops.
let phaseOf = (i: float): float => fraction(i + 0.5) * 2.0

let runOf = (i: float): float => 0.25 + 0.5 * fraction(i)

// A far character's bucket, and the shared phase/run of each bucket.
let bucketOf = (i: float): float => Math.floor(fraction(i + 0.5) * farBuckets)

let bucketPhase = (b: float): float => (b + 0.5) / farBuckets * 2.0

let bucketRun = (b: float): float => 0.25 + 0.5 * (b + 0.5) / farBuckets

let memberAt = (i: float, seat: Seat): Member =>
  let tier = tierOf(distanceOf(seat)) in
  let look = { kit: kitOf(i), lod: lodOf(tier) } in
  match tier with
  | Far => { key: $"char/{i}", seat: seat, tier: tier, look: look, phase: bucketPhase(bucketOf(i)), run: bucketRun(bucketOf(i)) }
  | _ => { key: $"char/{i}", seat: seat, tier: tier, look: look, phase: phaseOf(i), run: runOf(i) }

let members = (count: float): List<Member> =>
  seats |> List.take(count) |> List.indexedMap(memberAt)

// Far members grouped by (preset, bucket); empty groups dropped.
let batchesOf = (all: List<Member>): List<Batch> =>
  let far = all |> List.filter((m) => m.tier == Far) in
  Loadout.presets |> List.concatMap((kit) =>
    List.range(farBuckets) |> List.concatMap((b) =>
      let group = far |> List.filter((m) => m.look.kit == kit && m.phase == bucketPhase(b)) in
      if List.isEmpty(group) then []
      else [{
        key: $"far/{Loadout.presetName(kit, 2.0)}/{b}",
        look: { kit: kit, lod: 2.0 },
        bucket: b,
        seats: group |> List.map((m) => m.seat),
      }]))

let layout = (count: float): Layout =>
  let all = members(count) in
  { members: all |> List.filter((m) => m.tier != Far), batches: batchesOf(all) }

let countTier = (tier: Tier, all: List<Member>): float =>
  all |> List.filter((m) => m.tier == tier) |> List.length

// Tier assignment by distance.
expect tierOf(5.0) == Near
expect tierOf(7.0) == Mid
expect tierOf(11.9) == Mid
expect tierOf(12.0) == Far
// The front row (5 m out, eye 2.1 m above chest) is near; row 3 (7.6 m) is mid.
expect (seats |> List.head |> Option.map(distanceOf) |> Option.map(tierOf)) == Option.Some(Near)
expect (seats |> List.nth(11.0) |> Option.map(distanceOf) |> Option.map(tierOf)) == Option.Some(Mid)
expect List.length(seats) >= 100.0
// 100 characters split ~10 near / ~40 mid / ~50 far.
expect countTier(Near, members(100.0)) == 11.0
expect countTier(Mid, members(100.0)) == 38.0
expect countTier(Far, members(100.0)) == 51.0
// Small crowds stay near.
expect countTier(Near, members(1.0)) == 1.0
expect countTier(Near, members(25.0)) == 11.0
// Every far member lands in exactly one batch, at LOD2.
expect (
  let l = layout(100.0) in
  (l.batches |> List.map((b) => List.length(b.seats)) |> List.sum) == 51.0
    && (l.batches |> List.all((b) => b.look.lod == 2.0))
    && List.length(l.batches) <= 6.0 * farBuckets
)
// Phases are deterministic, in range, and shared within a far bucket.
expect phaseOf(7.0) == phaseOf(7.0)
expect List.range(100.0) |> List.all((i) => phaseOf(i) >= 0.0 && phaseOf(i) < 2.0)
expect List.range(100.0) |> List.all((i) => runOf(i) >= 0.25 && runOf(i) <= 0.75)
expect layout(100.0) == layout(100.0)
expect (
  members(100.0) |> List.filter((m) => m.tier == Far)
    |> List.all((m) => List.range(farBuckets) |> List.any((b) => m.phase == bucketPhase(b)))
)
// Near/mid characters are individually phased: no two share a playhead.
expect (
  let ps = layout(100.0).members |> List.map((m) => m.phase) in
  ps |> List.all((p) => (ps |> List.filter((q) => q == p) |> List.length) == 1.0)
)
