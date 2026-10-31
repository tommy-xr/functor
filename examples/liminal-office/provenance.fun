// Bake provenance: bake.mjs writes baked/provenance.txt beside the lightmaps
// (the layout hash below, the bakes, the baker's version and settings); the
// game checks it at load, so a bake older than the layout reads as stale, not
// as current.

// The files the bakes' geometry and lights derive from. bake.mjs hashes the
// same list, the same way.
let sources = ["layout.fun", "building.fun"]

// A polynomial hash over little-endian u32 words, then any tail bytes, mod a
// prime below 2^32: every step stays exact in a double, so JavaScript
// computes the same number.
let prime = 4294967291.0
let factor = 1000003.0
let step = (h: float, x: float): float => Math.mod(h * factor + x, prime)
let hashBytes = (h0: float, b: Bytes.t): float =>
  let n = Bytes.length(b) in
  let words = Math.floor(n / 4.0) in
  let h1 = List.range(words) |> List.fold((h, i) => step(h, Bytes.u32le(4.0 * i, b) |> Binary.require), h0) in
  List.range(n - 4.0 * words) |> List.fold((h, i) => step(h, Bytes.u8(4.0 * words + i, b) |> Binary.require), h1)

// Unreadable sources (the web shell can't read project files yet) hash as
// nothing; the baked files are unreadable there too, so status is Missing.
let layoutHash = sources |> List.fold((h, name) =>
  if File.exists(Path.of(name)) then hashBytes(h, File.bytes(Path.of(name))) else h, 0.0)

// Night and day, made by one `functor bake` run.
let bakeNames = Building.bakes |> List.map((b) => b.name)

type Status =
  | Current
  | Missing
  | Stale(reason: string)

// `key=value` lines.
let field = (key: string, text: string): Option.t<string> =>
  text |> Text.split("\n") |> List.map(Text.trim)
    |> List.find((line) => List.head(Text.split("=", line)) == Option.Some(key))
    |> Option.map((line) => Text.replace($"{key}=", "", line))

// Whether a provenance record matches this layout and these bakes.
let check = (text: string, hash: float): Status =>
  let recorded = (key: string) => field(key, text) |> Option.defaultValue("") in
  let expected = Text.join(" ", sources) in
  let bakes = Text.join(" ", bakeNames) in
  if recorded("layout") == "" then Stale("no layout hash recorded")
  else if recorded("sources") != expected then Stale($"baked from {recorded("sources")}, expected {expected}")
  else if recorded("bakes") != bakes then Stale($"bakes {recorded("bakes")}, expected {bakes}")
  else if Text.parseFloat(recorded("layout")) != hash then Stale($"layout hash {recorded("layout")}, now {hash}")
  else Current

let bakeDir = Path.of("baked")
let lightPathOf = (name: string) => bakeDir |> Path.join($"{name}.png")
let lightPath = lightPathOf("night")
let dayLightPath = lightPathOf("day")
// Both bakes unwrap the same mesh the same way, so bake.mjs keeps one copy.
let meshPath = bakeDir |> Path.join("night.mesh.bin")
let recordPath = bakeDir |> Path.join("provenance.txt")

let status =
  if not (File.exists(meshPath) && List.all((name) => File.exists(lightPathOf(name)), bakeNames)) then Missing
  else if not File.exists(recordPath) then Stale("no provenance.txt")
  else
    let bytes = File.bytes(recordPath) in
    check(Bytes.utf8(0.0, Bytes.length(bytes), bytes) |> Binary.require, layoutHash)

let record = (hash: float, bakes: string): string =>
  $"layout={hash}\nsources=layout.fun building.fun\nbakes={bakes}\n"
expect bakeNames == ["night", "day"]
expect check(record(layoutHash, "night day"), layoutHash) == Current
expect check(record(layoutHash + 1.0, "night day"), layoutHash) != Current
// A record from before the day bake is stale.
expect check(record(layoutHash, "night"), layoutHash) != Current
expect check($"layout={layoutHash}\nsources=layout.fun\nbakes=night day\n", layoutHash) != Current
expect check("bakeVersion=2\n", layoutHash) != Current
expect step(prime - 1.0, 255.0) < prime
// The committed bakes match this layout: rebake after editing (README).
expect status == Current
