// Ammo: rounds, magazines and a weapon's well and chamber, as plain data.
// Every round is somewhere: in a magazine (in a well, a hand, a slot or the
// world), in a chamber, or loose; or it is spent. Nothing here makes or
// loses one, so unspent + spent is the same after every operation.

/// What a magazine fits: the pistol's well or the rifle's.
type Kind = | PistolMag | RifleMag

type Mag = { kind: Kind, rounds: float, capacity: float }

/// A weapon's ammunition: the magazine kind its well takes, the magazine in
/// it, and whether a round is chambered.
type Gun = { well: Kind, mag: Option.t<Mag>, chambered: bool }

let capacity = (k: Kind): float =>
  match k with
  | PistolMag => 12.0
  | RifleMag => 30.0

let full = (k: Kind): Mag => { kind: k, rounds: capacity(k), capacity: capacity(k) }

/// Loaded and chambered: a full magazine plus one (12+1, 30+1).
let loaded = (k: Kind): Gun => { well: k, mag: Option.Some(full(k)), chambered: true }

let roundsIn = (m: Option.t<Mag>): float => m |> Option.map((g) => g.rounds) |> Option.defaultValue(0.0)

/// A weapon's unspent rounds: its magazine's and the chambered one.
let count = (g: Gun): float => roundsIn(g.mag) + (if g.chambered then 1.0 else 0.0)

/// The next round from the magazine into an empty chamber, if it has one.
let feed = (g: Gun): Gun =>
  match g.mag with
  | Option.Some(m) =>
    if g.chambered || m.rounds <= 0.0 then g
    else { g with mag: Option.Some({ m with rounds: m.rounds - 1.0 }), chambered: true }
  | Option.None => g

type Pull = | Fired | Dry

/// The trigger pulled: the chambered round is spent and the next fed from
/// the magazine, if any. Nothing chambered: dry, and nothing changes. The
/// last round leaves the chamber empty (no slide lock: racking is `rack`).
let fire = (g: Gun): (Gun, Pull) =>
  if g.chambered then (feed({ g with chambered: false }), Fired) else (g, Dry)

/// The magazine dropped out of the well: the weapon, and the magazine (now
/// in the world), if there was one. The chambered round stays.
let eject = (g: Gun): (Gun, Option.t<Mag>) => ({ g with mag: Option.None }, g.mag)

/// Whether a magazine goes in: its kind fits the well, and the well is empty.
let fits = (m: Mag, g: Gun): bool => m.kind == g.well && Option.isNone(g.mag)

/// A magazine into the well: the weapon, and the magazine if refused
/// (another kind, or the well occupied), still where it was. It doesn't
/// chamber (`rack` does).
let insert = (m: Mag, g: Gun): (Gun, Option.t<Mag>) =>
  if fits(m, g) then ({ g with mag: Option.Some(m) }, Option.None) else (g, Option.Some(m))

/// Racked: a chambered round is thrown out loose, then the next fed. The
/// weapon, and the loose rounds (0 or 1).
let rack = (g: Gun): (Gun, float) =>
  (feed({ g with chambered: false }), if g.chambered then 1.0 else 0.0)

// --- the truth tables ---

let mag = (k: Kind, n: float): Mag => { full(k) with rounds: n }
let gunWith = (k: Kind, m: Option.t<Mag>, chambered: bool): Gun => { well: k, mag: m, chambered: chambered }
let some = (k: Kind, n: float) => Option.Some(mag(k, n))

expect count(loaded(PistolMag)) == 13.0 && count(loaded(RifleMag)) == 31.0

// fire: chambered → spent, the next fed; empty magazine or none → the
// chamber empties; nothing chambered → dry, unchanged.
expect fire(gunWith(PistolMag, some(PistolMag, 5.0), true)) == (gunWith(PistolMag, some(PistolMag, 4.0), true), Fired)
expect fire(gunWith(PistolMag, some(PistolMag, 0.0), true)) == (gunWith(PistolMag, some(PistolMag, 0.0), false), Fired)
expect fire(gunWith(PistolMag, Option.None, true)) == (gunWith(PistolMag, Option.None, false), Fired)
expect fire(gunWith(PistolMag, some(PistolMag, 5.0), false)) == (gunWith(PistolMag, some(PistolMag, 5.0), false), Dry)
expect fire(gunWith(PistolMag, Option.None, false)) == (gunWith(PistolMag, Option.None, false), Dry)

// eject: the magazine out, the chamber as it was; none, nothing.
expect eject(gunWith(RifleMag, some(RifleMag, 7.0), true)) == (gunWith(RifleMag, Option.None, true), some(RifleMag, 7.0))
expect eject(gunWith(RifleMag, Option.None, false)) == (gunWith(RifleMag, Option.None, false), Option.None)

// insert: into an empty well of its kind; refused into another kind's or an
// occupied one, the magazine unchanged.
expect insert(mag(PistolMag, 3.0), gunWith(PistolMag, Option.None, false)) == (gunWith(PistolMag, some(PistolMag, 3.0), false), Option.None)
expect insert(mag(RifleMag, 30.0), gunWith(PistolMag, Option.None, true)) == (gunWith(PistolMag, Option.None, true), some(RifleMag, 30.0))
expect insert(mag(PistolMag, 9.0), gunWith(PistolMag, some(PistolMag, 2.0), true)) == (gunWith(PistolMag, some(PistolMag, 2.0), true), some(PistolMag, 9.0))

// rack: chambers from the magazine; a round already chambered comes out loose.
expect rack(gunWith(PistolMag, some(PistolMag, 4.0), false)) == (gunWith(PistolMag, some(PistolMag, 3.0), true), 0.0)
expect rack(gunWith(PistolMag, some(PistolMag, 4.0), true)) == (gunWith(PistolMag, some(PistolMag, 3.0), true), 1.0)
expect rack(gunWith(PistolMag, some(PistolMag, 0.0), true)) == (gunWith(PistolMag, some(PistolMag, 0.0), false), 1.0)
expect rack(gunWith(PistolMag, Option.None, false)) == (gunWith(PistolMag, Option.None, false), 0.0)

// --- conservation ---
// A bench of weapons and loose magazines, driven by ops: unspent (every
// magazine, every weapon's well and chamber, the loose rounds) plus spent
// never changes.

type Bench = { guns: List<Gun>, mags: List<Mag>, loose: float, spent: float }

type Op = | Pull(gun: float) | Eject(gun: float) | Insert(mag: float, gun: float) | Rack(gun: float)

let unspent = (b: Bench): float =>
  (b.mags |> List.map((m) => m.rounds) |> List.sum) + (b.guns |> List.map(count) |> List.sum) + b.loose

let total = (b: Bench): float => unspent(b) + b.spent

let atIndex = (i: float, f: (Gun) => Gun, gs: List<Gun>): List<Gun> => gs |> List.indexedMap((j, g) => if j == i then f(g) else g)

let apply = (b: Bench, op: Op): Bench =>
  let gun = (i: float) => List.nth(i, b.guns) |> Option.defaultValue(gunWith(PistolMag, Option.None, false)) in
  match op with
  | Pull(i) =>
    let (g, pull) = fire(gun(i)) in
    { b with guns: atIndex(i, (_) => g, b.guns), spent: if pull == Fired then b.spent + 1.0 else b.spent }
  | Eject(i) =>
    let (g, out) = eject(gun(i)) in
    { b with guns: atIndex(i, (_) => g, b.guns), mags: match out with | Option.Some(m) => b.mags |> List.append([m]) | Option.None => b.mags }
  | Insert(k, i) =>
    (match List.nth(k, b.mags) with
     | Option.None => b
     | Option.Some(m) =>
       let (g, left) = insert(m, gun(i)) in
       if Option.isSome(left) then b
       else { b with guns: atIndex(i, (_) => g, b.guns), mags: List.append(List.drop(k + 1.0, b.mags), List.take(k, b.mags)) })
  | Rack(i) =>
    let (g, loose) = rack(gun(i)) in
    { b with guns: atIndex(i, (_) => g, b.guns), loose: b.loose + loose }

let bench: Bench = {
  guns: [loaded(PistolMag), loaded(RifleMag)],
  mags: [full(PistolMag), mag(PistolMag, 3.0), full(RifleMag)],
  loose: 0.0,
  spent: 0.0
}

/// Every bench `ops` passes through, the start first, conserving.
let conserves = (b0: Bench, ops: List<Op>): bool =>
  let (_, ok) = ops |> List.fold((acc, op) =>
    let (b, good) = acc in
    let next = apply(b, op) in
    (next, good && total(next) == total(b0) && next.loose >= 0.0
      && (next.mags |> List.all((m) => m.rounds >= 0.0 && m.rounds <= m.capacity))), (b0, true)) in
  ok

expect total(bench) == 13.0 + 31.0 + 12.0 + 3.0 + 30.0
// Hand-written: emptied, ejected, reloaded with a spare and racked (a loose
// round), a rifle magazine refused by the pistol, the rifle dry.
expect conserves(bench, List.flatten([
  List.range(15.0) |> List.map((_) => Pull(0.0)),
  [Eject(0.0), Insert(1.0, 0.0), Rack(0.0), Rack(0.0), Insert(1.0, 0.0), Insert(0.0, 0.0), Pull(0.0)],
  [Eject(1.0), Pull(1.0), Pull(1.0), Rack(1.0), Insert(2.0, 1.0)],
]))
expect (
  // The pistol emptied: 13 shots, then dry; reloaded and racked, fires again.
  let b = List.range(14.0) |> List.fold((b, _) => apply(b, Pull(0.0)), bench) in
  let reloaded = List.fold((acc, op) => apply(acc, op), b, [Eject(0.0), Insert(0.0, 0.0), Rack(0.0)]) in
  b.spent == 13.0 && count(List.head(b.guns) |> Option.defaultValue(loaded(RifleMag))) == 0.0
    && (apply(reloaded, Pull(0.0))).spent == 14.0 && reloaded.loose == 0.0
)

/// `n` ops drawn from `seed`: mostly pulls, then ejects, inserts and racks
/// on either weapon and any magazine, valid or not.
let randomOps = (n: float, seed: Random.Seed): List<Op> =>
  let (ops, _) = List.range(n) |> List.fold((acc, _) =>
    let (out, s) = acc in
    let (kind, s1) = Random.step(s) in
    let (which, s2) = Random.step(s1) in
    let (pick, s3) = Random.step(s2) in
    let gun = Math.floor(which * 2.0) in
    let op =
      if kind < 0.4 then Pull(gun)
      else if kind < 0.6 then Eject(gun)
      else if kind < 0.85 then Insert(Math.floor(pick * 4.0), gun)
      else Rack(gun) in
    (out |> List.append([op]), s3), ([], seed)) in
  ops

let seeded = randomOps(1200.0, Random.seed(10.0))
expect List.length(seeded) == 1200.0 && conserves(bench, seeded)
// …and it exercised every path: shots, dry pulls, refusals, loose rounds.
expect (
  let end = seeded |> List.fold(apply, bench) in
  end.spent > 40.0 && end.loose > 5.0 && unspent(end) > 0.0 && total(end) == total(bench)
)
