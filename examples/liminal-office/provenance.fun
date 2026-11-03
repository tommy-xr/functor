// Bake provenance, per archetype: bake.mjs writes baked/<archetype>/
// provenance.txt beside its lightmaps (the layout hash below, the bakes, the
// baker's version and settings); the game checks it at load, so a bake
// older than the layout reads as stale, not as current.

// An archetype's bake: the files its geometry and lights derive from
// (bake.mjs hashes the same list, the same way), its bakes' names (the
// hours), and the directory holding them.
type Spec = { dir: string, sources: List<string>, bakes: List<string> }

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
let layoutHash = (s: Spec): float => s.sources |> List.fold((h, name) =>
  if File.exists(Path.of(name)) then hashBytes(h, File.bytes(Path.of(name))) else h, 0.0)

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
let check = (s: Spec, text: string, hash: float): Status =>
  let recorded = (key: string) => field(key, text) |> Option.defaultValue("") in
  let expected = Text.join(" ", s.sources) in
  let bakes = Text.join(" ", s.bakes) in
  if recorded("layout") == "" then Stale("no layout hash recorded")
  else if recorded("sources") != expected then Stale($"baked from {recorded("sources")}, expected {expected}")
  else if recorded("bakes") != bakes then Stale($"bakes {recorded("bakes")}, expected {bakes}")
  else if Text.parseFloat(recorded("layout")) != hash then Stale($"layout hash {recorded("layout")}, now {hash}")
  else Current

let lightPathOf = (s: Spec, name: string) => Path.of(s.dir) |> Path.join($"{name}.png")
// Both bakes unwrap the same mesh the same way, so bake.mjs keeps one copy.
let meshPathOf = (s: Spec) => Path.of(s.dir) |> Path.join("night.mesh.bin")
let recordPathOf = (s: Spec) => Path.of(s.dir) |> Path.join("provenance.txt")

let statusOf = (s: Spec): Status =>
  if not (File.exists(meshPathOf(s)) && List.all((name) => File.exists(lightPathOf(s, name)), s.bakes)) then Missing
  else if not File.exists(recordPathOf(s)) then Stale("no provenance.txt")
  else
    let bytes = File.bytes(recordPathOf(s)) in
    check(s, Bytes.utf8(0.0, Bytes.length(bytes), bytes) |> Binary.require, layoutHash(s))

// A bake's cell models (building.fun's cut of its mesh) under its night
// lightmap, and its day lightmap; flat-lit without a current bake.
type Baked = { models: List<Building.CellModel>, day: Option.t<Asset.Texture> }
let bakedOf = (k: Building.Interior, s: Spec, status: Status): Baked =>
  match status with
  | Current =>
    let mesh = meshPathOf(s) |> File.bytes |> Lightmap.readMesh in
    { models: Building.cellModels(k, { mesh with lightmap: Option.Some({
        texture: Texture.fromPng(File.bytes(lightPathOf(s, "night")), Texture.Srgb),
        scale: Building.bakeScale, floor: Color.rgb(0.0, 0.0, 0.0) }) }),
      day: Option.Some(Texture.fromPng(File.bytes(lightPathOf(s, "day")), Texture.Srgb)) }
  | _ => { models: Building.cellModels(k, k.interiorData), day: Option.None }
// `scene` under a day lightmap, if there is one.
let underDay = (day: Option.t<Asset.Texture>, scene: Scene.t): Scene.t =>
  match day with
  | Option.Some(t) => scene |> Scene.withLightmap(t)
  | Option.None => scene

// A record checked against a made-up spec and hash.
let testSpec = { dir: "baked/test", sources: ["layout.fun", "testLayout.fun"], bakes: ["night", "day"] }
let testHash = 7.0
let record = (h: float, bakes: string): string =>
  $"layout={h}\nsources=layout.fun testLayout.fun\nbakes={bakes}\n"
expect check(testSpec, record(testHash, "night day"), testHash) == Current
expect check(testSpec, record(testHash + 1.0, "night day"), testHash) != Current
// A record from before the day bake is stale.
expect check(testSpec, record(testHash, "night"), testHash) != Current
expect check(testSpec, $"layout={testHash}\nsources=layout.fun\nbakes=night day\n", testHash) != Current
expect check(testSpec, "bakeVersion=2\n", testHash) != Current
expect step(prime - 1.0, 255.0) < prime
