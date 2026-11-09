// Ammo: rounds, magazines and a weapon's well and chamber, as plain data.
// Every round is somewhere: in a magazine (in a well, a hand, a slot or the
// world), in a chamber, or loose; or it is spent. Nothing here makes or
// loses one, so unspent + spent is the same after every operation. A
// shotgun's tube is its magazine: fixed in the gun, loaded a shell at a time.

/// What a magazine fits: the pistol's well or the rifle's. `Shell`: a
/// shotgun shell, and the tube it loads into.
type Kind = | PistolMag | RifleMag | Shell

type Mag = { kind: Kind, rounds: float, capacity: float }

/// A weapon's ammunition: the magazine kind its well takes, the magazine in
/// it (a tube's is always in), whether a live round is chambered, whether
/// the slide (or bolt) is locked back (it closed on an empty magazine), and
/// whether a fired hull is still in the chamber (a pump gun's, until pumped).
type State = { well: Kind, mag: Option.t<Mag>, chambered: bool, locked: bool, hull: bool }

let capacity = (k: Kind): float =>
  match k with
  | PistolMag => 12.0
  | RifleMag => 30.0
  | Shell => 6.0

let full = (k: Kind): Mag => { kind: k, rounds: capacity(k), capacity: capacity(k) }

/// Loaded and chambered: a full magazine plus one (12+1, 30+1).
let loaded = (k: Kind): State => { well: k, mag: Option.Some(full(k)), chambered: true, locked: false, hull: false }

/// Unloaded: an empty magazine (a tube, empty), nothing chambered.
let unloaded = (k: Kind): State => { well: k, mag: Option.Some({ full(k) with rounds: 0.0 }), chambered: false, locked: false, hull: false }

/// Whether a shot cycles the action itself (a magazine-fed gun's). A tube's
/// gun is a pump: the hand cycles it.
let selfLoading = (k: Kind): bool => k != Shell

let roundsIn = (m: Option.t<Mag>): float => m |> Option.map((g) => g.rounds) |> Option.defaultValue(0.0)

/// A weapon's unspent rounds: its magazine's and the chambered one.
let count = (g: State): float => roundsIn(g.mag) + (if g.chambered then 1.0 else 0.0)

/// The next round from the magazine into an empty chamber, if it has one.
let feed = (g: State): State =>
  match g.mag with
  | Option.Some(m) =>
    if g.chambered || m.rounds <= 0.0 then g
    else { g with mag: Option.Some({ m with rounds: m.rounds - 1.0 }), chambered: true }
  | Option.None => g

/// The slide (or bolt, or pump) going home: it feeds the next round, and
/// (a self-loading gun's) locks back instead if the magazine in the well is
/// empty. With no magazine, or a pump on an empty tube, it closes on an
/// empty chamber.
let close = (g: State): State =>
  let fed = feed({ g with locked: false }) in
  { fed with locked: selfLoading(g.well) && not fed.chambered && (fed.mag |> Option.map((m) => m.rounds <= 0.0) |> Option.defaultValue(false)) }

type Pull = | Fired | Dry

/// The trigger pulled: the chambered round is spent and the slide cycles
/// (`close`): the next round fed, or, the magazine empty, locked back. A
/// pump gun doesn't cycle: its hull stays in the chamber until pumped out.
/// Nothing chambered (locked back, or a hull, included): dry, and nothing
/// changes.
let fire = (g: State): (State, Pull) =>
  if not g.chambered then (g, Dry)
  else if selfLoading(g.well) then (close({ g with chambered: false }), Fired)
  else ({ g with chambered: false, hull: true }, Fired)

/// The magazine dropped out of the well: the weapon, and the magazine (now
/// in the world), if there was one. The chambered round stays. A tube stays
/// in its gun.
let eject = (g: State): (State, Option.t<Mag>) =>
  if g.well == Shell then (g, Option.None) else ({ g with mag: Option.None }, g.mag)

/// Whether a magazine goes in: its kind fits the well, and the well is empty.
let fits = (m: Mag, g: State): bool => m.kind == g.well && Option.isNone(g.mag)

/// Whether a weapon takes one of `k` now: a magazine its well fits, the well
/// empty; a shell, its tube not full.
let takes = (k: Kind, g: State): bool =>
  k == g.well && (if k == Shell then g.mag |> Option.map((m) => m.rounds < m.capacity) |> Option.defaultValue(false) else Option.isNone(g.mag))

/// A shell pushed into the tube: the weapon, and whether it went in (the
/// tube full, or not a tube: refused).
let load = (g: State): (State, bool) =>
  match g.mag with
  | Option.Some(m) => if takes(Shell, g) then ({ g with mag: Option.Some({ m with rounds: m.rounds + 1.0 }) }, true) else (g, false)
  | Option.None => (g, false)

/// A magazine into the well: the weapon, and the magazine if refused
/// (another kind, or the well occupied), still where it was. It doesn't
/// chamber (`rack` does).
let insert = (m: Mag, g: State): (State, Option.t<Mag>) =>
  if fits(m, g) then ({ g with mag: Option.Some(m) }, Option.None) else (g, Option.Some(m))

/// The slide (or bolt, or pump) pulled back: a chambered round is thrown
/// out loose, a hull thrown out spent. The weapon, and the loose rounds (0
/// or 1).
let extract = (g: State): (State, float) => ({ g with chambered: false, hull: false }, if g.chambered then 1.0 else 0.0)

/// Racked, back then home: a chambered round thrown out loose, the next fed
/// (and the lock released, or set again on an empty magazine). The weapon,
/// and the loose rounds (0 or 1).
let rack = (g: State): (State, float) =>
  let (back, loose) = extract(g) in (close(back), loose)

// --- the truth tables ---

let mag = (k: Kind, n: float): Mag => { full(k) with rounds: n }
let gunWith = (k: Kind, m: Option.t<Mag>, chambered: bool): State => { well: k, mag: m, chambered: chambered, locked: false, hull: false }
let lockedWith = (k: Kind, m: Option.t<Mag>): State => { gunWith(k, m, false) with locked: true }
let some = (k: Kind, n: float) => Option.Some(mag(k, n))

expect count(loaded(PistolMag)) == 13.0 && count(loaded(RifleMag)) == 31.0

// fire: chambered → spent, the next fed; the magazine empty → the chamber
// empties and the slide locks back; no magazine → it empties, unlocked;
// nothing chambered → dry, unchanged.
expect fire(gunWith(PistolMag, some(PistolMag, 5.0), true)) == (gunWith(PistolMag, some(PistolMag, 4.0), true), Fired)
expect fire(gunWith(PistolMag, some(PistolMag, 0.0), true)) == (lockedWith(PistolMag, some(PistolMag, 0.0)), Fired)
expect fire(gunWith(PistolMag, Option.None, true)) == (gunWith(PistolMag, Option.None, false), Fired)
expect fire(gunWith(PistolMag, some(PistolMag, 5.0), false)) == (gunWith(PistolMag, some(PistolMag, 5.0), false), Dry)
expect fire(gunWith(PistolMag, Option.None, false)) == (gunWith(PistolMag, Option.None, false), Dry)
expect fire(lockedWith(PistolMag, some(PistolMag, 0.0))) == (lockedWith(PistolMag, some(PistolMag, 0.0)), Dry)

// eject: the magazine out, the chamber and the lock as they were; none, nothing.
expect eject(gunWith(RifleMag, some(RifleMag, 7.0), true)) == (gunWith(RifleMag, Option.None, true), some(RifleMag, 7.0))
expect eject(lockedWith(RifleMag, some(RifleMag, 0.0))) == (lockedWith(RifleMag, Option.None), some(RifleMag, 0.0))
expect eject(gunWith(RifleMag, Option.None, false)) == (gunWith(RifleMag, Option.None, false), Option.None)

// insert: into an empty well of its kind; refused into another kind's or an
// occupied one, the magazine unchanged.
expect insert(mag(PistolMag, 3.0), gunWith(PistolMag, Option.None, false)) == (gunWith(PistolMag, some(PistolMag, 3.0), false), Option.None)
expect insert(mag(RifleMag, 30.0), gunWith(PistolMag, Option.None, true)) == (gunWith(PistolMag, Option.None, true), some(RifleMag, 30.0))
expect insert(mag(PistolMag, 9.0), gunWith(PistolMag, some(PistolMag, 2.0), true)) == (gunWith(PistolMag, some(PistolMag, 2.0), true), some(PistolMag, 9.0))
// Into a locked gun: in, still locked, nothing chambered.
expect insert(mag(PistolMag, 12.0), lockedWith(PistolMag, Option.None)) == (lockedWith(PistolMag, some(PistolMag, 12.0)), Option.None)

// rack: chambers from the magazine; a round already chambered comes out
// loose. Locked, it releases the lock and chambers; on an empty magazine it
// locks again.
expect rack(gunWith(PistolMag, some(PistolMag, 4.0), false)) == (gunWith(PistolMag, some(PistolMag, 3.0), true), 0.0)
expect rack(gunWith(PistolMag, some(PistolMag, 4.0), true)) == (gunWith(PistolMag, some(PistolMag, 3.0), true), 1.0)
expect rack(gunWith(PistolMag, some(PistolMag, 0.0), true)) == (lockedWith(PistolMag, some(PistolMag, 0.0)), 1.0)
expect rack(gunWith(PistolMag, Option.None, false)) == (gunWith(PistolMag, Option.None, false), 0.0)
expect rack(lockedWith(PistolMag, some(PistolMag, 12.0))) == (gunWith(PistolMag, some(PistolMag, 11.0), true), 0.0)
expect rack(lockedWith(PistolMag, Option.None)) == (gunWith(PistolMag, Option.None, false), 0.0)
// The slide lock's states: a full magazine emptied by shots locks it on the
// last; a dry pull leaves it; a magazine out and a fresh one in leave it;
// the rack releases it and chambers.
expect (
  let shots = List.range(13.0) |> List.fold((g, _) => let (next, _) = fire(g) in next, loaded(PistolMag)) in
  let (dry, pull) = fire(shots) in
  let (out, _) = eject(dry) in
  let (fresh, _) = insert(full(PistolMag), out) in
  let (racked, loose) = rack(fresh) in
  shots == lockedWith(PistolMag, some(PistolMag, 0.0)) && pull == Dry && dry == shots
    && out.locked && fresh.locked && not fresh.chambered
    && racked == gunWith(PistolMag, some(PistolMag, 11.0), true) && loose == 0.0
)

// The pump gun: a shot leaves its hull in the chamber, so the next pull is
// dry until it is pumped: back, the hull out (no round loose); home, the
// next shell fed from the tube. It never locks back, and its tube stays in.
let tube = (n: float): Option.t<Mag> => some(Shell, n)
let pumpWith = (n: float, chambered: bool, hull: bool): State => { gunWith(Shell, tube(n), chambered) with hull: hull }
expect fire(pumpWith(3.0, true, false)) == (pumpWith(3.0, false, true), Fired)
expect fire(pumpWith(3.0, false, true)) == (pumpWith(3.0, false, true), Dry)
expect extract(pumpWith(3.0, false, true)) == (pumpWith(3.0, false, false), 0.0)
expect extract(pumpWith(3.0, true, false)) == (pumpWith(3.0, false, false), 1.0)
expect close(pumpWith(3.0, false, false)) == pumpWith(2.0, true, false)
expect close(pumpWith(0.0, false, false)) == pumpWith(0.0, false, false)
expect rack(pumpWith(0.0, false, true)) == (pumpWith(0.0, false, false), 0.0)
expect eject(pumpWith(4.0, true, false)) == (pumpWith(4.0, true, false), Option.None)
// One shot a stroke: two shells, pulled between pumps, fire twice; each pull
// without a pump is dry.
expect (
  let pull = (g: State) => let (next, p) = fire(g) in (next, p == Fired) in
  let (racked, _) = rack(pumpWith(2.0, false, false)) in
  let (a, first) = pull(racked) in
  let (b, again) = pull(a) in
  let (pumped, loose) = rack(b) in
  let (c, second) = pull(pumped) in
  let (emptied, _) = rack(c) in
  let (_, dry) = pull(emptied) in
  first && not again && loose == 0.0 && second && not dry && emptied == pumpWith(0.0, false, false)
)
// A shell loads into a tube with room; a full tube, or a magazine well, refuses it.
expect load(pumpWith(5.0, false, false)) == (pumpWith(6.0, false, false), true)
expect load(pumpWith(6.0, true, false)) == (pumpWith(6.0, true, false), false)
expect load(loaded(PistolMag)) == (loaded(PistolMag), false)
expect takes(Shell, pumpWith(0.0, false, false)) && not takes(Shell, pumpWith(6.0, false, false)) && not takes(PistolMag, pumpWith(0.0, false, false))
expect takes(PistolMag, gunWith(PistolMag, Option.None, false)) && not takes(PistolMag, loaded(PistolMag)) && not takes(Shell, gunWith(PistolMag, Option.None, false))
expect count(unloaded(Shell)) == 0.0 && count(loaded(Shell)) == 7.0

// --- conservation ---
// A bench of weapons and loose magazines, driven by ops: unspent (every
// magazine, every weapon's well and chamber, the loose rounds and shells)
// plus spent never changes.

/// `shells`: loose live shells (out of a box, or pumped out), ready to load.
type Bench = { guns: List<State>, mags: List<Mag>, loose: float, shells: float, spent: float }

// `Extract` / `Close`: the slide's (or pump's) two strokes, as the game makes
// them (a magazine or shell may go in between). `Load`: a loose shell into a
// gun's tube.
type Op = | Trigger(gun: float) | Eject(gun: float) | Insert(mag: float, gun: float) | Rack(gun: float) | Extract(gun: float) | Close(gun: float) | Load(gun: float)

let unspent = (b: Bench): float =>
  (b.mags |> List.map((m) => m.rounds) |> List.sum) + (b.guns |> List.map(count) |> List.sum) + b.loose + b.shells

let total = (b: Bench): float => unspent(b) + b.spent

let atIndex = (i: float, f: (State) => State, gs: List<State>): List<State> => gs |> List.indexedMap((j, g) => if j == i then f(g) else g)

let apply = (b: Bench, op: Op): Bench =>
  let gun = (i: float) => List.nth(i, b.guns) |> Option.defaultValue(gunWith(PistolMag, Option.None, false)) in
  match op with
  | Trigger(i) =>
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
  | Rack(i) => let (g, loose) = rack(gun(i)) in thrownOut(b, i, g, loose)
  | Extract(i) => let (g, loose) = extract(gun(i)) in thrownOut(b, i, g, loose)
  | Close(i) => { b with guns: atIndex(i, close, b.guns) }
  | Load(i) =>
    let (g, went) = load(gun(i)) in
    if b.shells < 1.0 || not went then b else { b with guns: atIndex(i, (_) => g, b.guns), shells: b.shells - 1.0 }

/// Gun `i` now `g`, having thrown `n` rounds out: loose shells from a tube's
/// gun, loose rounds from the others.
let thrownOut = (b: Bench, i: float, g: State, n: float): Bench =>
  let shells = g.well == Shell in
  { b with guns: atIndex(i, (_) => g, b.guns), loose: if shells then b.loose else b.loose + n, shells: if shells then b.shells + n else b.shells }

let bench: Bench = {
  guns: [loaded(PistolMag), loaded(RifleMag), unloaded(Shell)],
  mags: [full(PistolMag), mag(PistolMag, 3.0), full(RifleMag)],
  loose: 0.0,
  shells: 12.0,
  spent: 0.0
}

/// Every bench `ops` passes through, the start first, conserving.
let conserves = (b0: Bench, ops: List<Op>): bool =>
  let (_, ok) = ops |> List.fold((acc, op) =>
    let (b, good) = acc in
    let next = apply(b, op) in
    (next, good && total(next) == total(b0) && next.loose >= 0.0 && next.shells >= 0.0
      && (next.mags |> List.all((m) => m.rounds >= 0.0 && m.rounds <= m.capacity))), (b0, true)) in
  ok

expect total(bench) == 13.0 + 31.0 + 0.0 + 12.0 + 3.0 + 30.0 + 12.0
// Hand-written: emptied, ejected, reloaded with a spare and racked (a loose
// round), a rifle magazine refused by the pistol, the rifle dry. The shotgun:
// seven shells offered (the seventh refused, full), pumped, fired, pulled
// dry, pumped with a live shell in (thrown out loose), then loaded again.
expect conserves(bench, List.flatten([
  List.range(15.0) |> List.map((_) => Trigger(0.0)),
  [Eject(0.0), Insert(1.0, 0.0), Rack(0.0), Rack(0.0), Insert(1.0, 0.0), Insert(0.0, 0.0), Trigger(0.0)],
  [Eject(1.0), Trigger(1.0), Trigger(1.0), Rack(1.0), Insert(2.0, 1.0)],
  List.range(7.0) |> List.map((_) => Load(2.0)),
  [Rack(2.0), Trigger(2.0), Trigger(2.0), Rack(2.0), Rack(2.0), Load(2.0), Load(2.0), Eject(2.0), Insert(0.0, 2.0)],
]))
let shotgunOf = (b: Bench): State => List.nth(2.0, b.guns) |> Option.defaultValue(unloaded(Shell))
// Six shells load (the seventh stays loose); pumped twice, one chambered,
// then thrown out live: a loose shell again.
expect (
  let b = List.range(7.0) |> List.fold((b, _) => apply(b, Load(2.0)), bench) in
  let pumped = apply(apply(b, Rack(2.0)), Rack(2.0)) in
  b.shells == 6.0 && count(shotgunOf(b)) == 6.0 && pumped.shells == 7.0 && count(shotgunOf(pumped)) == 5.0
)
expect (
  // The pistol emptied: 13 shots, then dry; reloaded and racked, fires again.
  let b = List.range(14.0) |> List.fold((b, _) => apply(b, Trigger(0.0)), bench) in
  let reloaded = List.fold((acc, op) => apply(acc, op), b, [Eject(0.0), Insert(0.0, 0.0), Rack(0.0)]) in
  b.spent == 13.0 && count(List.head(b.guns) |> Option.defaultValue(loaded(RifleMag))) == 0.0
    && (apply(reloaded, Trigger(0.0))).spent == 14.0 && reloaded.loose == 0.0
)

/// `n` ops drawn from `seed`: mostly pulls, then ejects, inserts, shells
/// loaded and racks on any weapon and any magazine, valid or not.
let randomOps = (n: float, seed: Random.Seed): List<Op> =>
  let (ops, _) = List.range(n) |> List.fold((acc, _) =>
    let (out, s) = acc in
    let (kind, s1) = Random.step(s) in
    let (which, s2) = Random.step(s1) in
    let (pick, s3) = Random.step(s2) in
    let gun = Math.floor(which * 3.0) in
    let op =
      if kind < 0.35 then Trigger(gun)
      else if kind < 0.5 then Eject(gun)
      else if kind < 0.7 then Insert(Math.floor(pick * 4.0), gun)
      else if kind < 0.8 then Load(gun)
      else if kind < 0.87 then Rack(gun)
      else if kind < 0.93 then Extract(gun)
      else Close(gun) in
    ([op, ..out], s3), ([], seed)) in
  List.reverse(ops)

// 1200 seeded ops (the slide's strokes apart too) on all three guns conserve
// every step, and reach every path: shots, loose rounds, shells loaded and
// pumped out, rounds left over.
expect (
  let ops = randomOps(1200.0, Random.seed(10.0)) in
  let end = ops |> List.fold(apply, bench) in
  // The most shells the shotgun held at once, and its shots.
  let (_, most, pumpShots) = ops |> List.fold((acc, op) =>
    let (b, hi, shots) = acc in
    let next = apply(b, op) in
    let fired = op == Trigger(2.0) && next.spent > b.spent in
    (next, Math.max(hi, count(shotgunOf(next))), if fired then shots + 1.0 else shots), (bench, 0.0, 0.0)) in
  List.length(ops) == 1200.0 && conserves(bench, ops)
    && end.spent > 30.0 && end.loose > 5.0 && most >= 3.0 && pumpShots > 3.0 && unspent(end) > 0.0 && total(end) == total(bench)
)
