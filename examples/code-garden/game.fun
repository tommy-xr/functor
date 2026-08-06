// gallery: A living procedural garden whose laws ARE this file — edit a constant, save, and the plants you already grew re-shape without restarting.
// gallery-controls: Space plant a seed - 1/2/3 pick species (Lantern/Ember/Sunthread) - R replant the garden - the file itself is the main controller
//
// code-garden — the source file is the game.
//
// Everything you can SEE is derived, every frame, from the definitions in the
// "THE LAWS" block below. Everything that PERSISTS — which plots were sown,
// what species each holds, and how many seconds each has been alive — lives in
// the model. Hot-reload keeps the model, so editing a law bends a garden that
// is already growing instead of rebooting it:
//
//   functor -d examples/code-garden run native
//   ... then edit `branchAngle` to 70deg and save. The same plants, the same
//   ages, a different botany.
//
// The precise mental model (worth internalizing — it is the whole point):
//
//   model  = { plants: [{ i, species, age }], pick }
//   scene  = draw(model, tts) = f(THE LAWS, model, tts)
//
//   * `age` is stored in raw SECONDS, never in "how grown is it". `growthRate`
//     converts seconds to branch levels inside `draw`, so lowering it makes the
//     WHOLE existing garden younger-looking on the very next frame, and raising
//     it flowers the garden you already have. Had `tick` folded growthRate into
//     the model, an edit would only affect future growth — which is exactly the
//     screensaver-reboot feeling this example is built to avoid.
//   * No closures and no engine values live in the model, so nothing in it goes
//     stale on reload; every material, angle and color is rebuilt from the laws.
//   * `tts` (day/night, wind) is the shell's clock and keeps its phase across a
//     reload too, so the sky does not jump when you save.
//   * A plant stores only its PLOT INDEX, not a position: `plotRadius` and the
//     spiral are laws too, so widening the bed re-arranges the garden you have.
//
// It demos well untouched: `init` sows five plants at staggered ages and a
// `Sub.every` timer keeps self-seeding up to `maxPlants`, so an idle window
// grows a full garden through a full day. The keyboard is a garnish; the file
// is the controller.

// ---------------------------------------------------------------------------
// Small pure helpers the laws are written in terms of.
// ---------------------------------------------------------------------------

// A plain-data color. `Color.t` is opaque and has no accessors or blend, so a
// palette that has to be interpolated (day -> night) is kept as floats and
// converted at the material boundary.
type Rgb = { r: float, g: float, b: float }

let rgb = (r: float, g: float, b: float): Rgb => { r: r, g: g, b: b }
let toColor = (c: Rgb) => Color.rgb(c.r, c.g, c.b)
let mix = (u: Rgb, v: Rgb, t: float): Rgb =>
  { r: Math.lerp(v.r, t, u.r), g: Math.lerp(v.g, t, u.g), b: Math.lerp(v.b, t, u.b) }
let gain = (k: float, c: Rgb): Rgb => { r: c.r * k, g: c.g * k, b: c.b * k }

// Per-branch variation, as a pure function of the branch's ADDRESS rather than
// a drawn random number, so the same plant is the same plant on every frame and
// after every reload. `Random.seed` hashes the number's BITS, so one `step` off
// a freshly seeded stream is exactly an address -> 0..1 lookup — no seed to
// thread, and nothing to keep in the model.
let hash01 = (n: float): float =>
  let (v, _) = Random.step(Random.seed(n)) in
  v

// ---------------------------------------------------------------------------
// THE LAWS — the garden is these numbers. Edit, save, watch.
// ---------------------------------------------------------------------------

// Botany.
let maxDepth = 4.0          // branch generations; 5 is lush and ~2x the cost
let branchAngle = 27deg     // how far a child leans off its parent
let branchTwist = 137deg    // how far each generation spins around the stem
let stemLength = 0.95       // length of a first-generation segment
let lengthFalloff = 0.74    // each generation is this fraction of its parent
let stemWidth = 0.07        // DIAMETER of a first-generation segment (the
                            // unit cylinder has radius 0.5, so this scales it)
let widthFalloff = 0.62
let jitter = 0.34           // 0 = topiary, 1 = wild

// Time. `age` is stored in seconds; this is the only thing that turns seconds
// into growth, and it is read in `draw` — so editing it re-ages the garden.
let growthRate = 0.42       // branch generations gained per second of age
let sproutEvery = 7.0       // seconds between self-seedings
let maxPlants = 9.0

// Blooms.
let bloomSize = 0.115
let petals = 5.0
let motes = 16.0            // fireflies, out after dusk
let nightGlow = 1.15         // emissive multiplier at midnight
let dayGlow = 0.8            // ... and at noon

// Weather and sky.
let windSway = 2.4          // degrees of sway contributed by ONE joint
let windSpeed = 0.85
let dayLength = 60.0        // seconds per full day/night cycle
let dayStart = 0.62         // phase at tts = 0 (0.62 = just past dusk)

// Ground.
let plotRadius = 4.2
let groundSize = 90.0
let groundRes = 22.0

// Camera.
let camRadius = 9.8
let camHeight = 2.75
let camTarget = 1.6
let camSpeed = 0.055        // radians per second
let camStart = 0.5

// Palette.
let moteColor = rgb(1.0, 0.86, 0.45)
let dayFog = rgb(0.55, 0.66, 0.64)
let nightFog = rgb(0.030, 0.038, 0.085)
let daySun = rgb(1.0, 0.93, 0.78)
let nightSun = rgb(0.42, 0.55, 0.95)
let dayAmbient = rgb(0.34, 0.39, 0.40)
let nightAmbient = rgb(0.065, 0.080, 0.135)
let daySky = rgb(0.63, 0.75, 0.78)     // clear color, a shade off the fog, so
let nightSky = rgb(0.045, 0.055, 0.125) // ... the horizon reads as a horizon
let groundDay = rgb(0.27, 0.33, 0.22)
let groundNight = rgb(0.10, 0.12, 0.13)

// ---------------------------------------------------------------------------
// Species — three botanies sharing one `grow`.
// ---------------------------------------------------------------------------

type Species = {
  stem: Rgb,      // lit stem albedo
  bloom: Rgb,     // emissive petal color
  core: Rgb,      // emissive bloom center
  lean: float,    // multiplier on branchAngle
  reach: float,   // multiplier on stemLength
}

let lantern = {
  stem: rgb(0.17, 0.30, 0.24),
  bloom: rgb(0.20, 0.95, 0.86),
  core: rgb(0.66, 0.92, 0.88),
  lean: 0.85,
  reach: 1.15,
}

let ember = {
  stem: rgb(0.31, 0.19, 0.24),
  bloom: rgb(1.0, 0.22, 0.58),
  core: rgb(0.95, 0.68, 0.80),
  lean: 1.35,
  reach: 0.88,
}

let sunthread = {
  stem: rgb(0.27, 0.28, 0.17),
  bloom: rgb(1.0, 0.72, 0.20),
  core: rgb(0.96, 0.90, 0.62),
  lean: 1.05,
  reach: 1.0,
}

let speciesOf = (i: float): Species =>
  if i < 0.5 then lantern
  else if i < 1.5 then ember
  else sunthread

// ---------------------------------------------------------------------------
// The ground. One height field, sampled twice: once as a mesh, once to seat
// each plant on the surface it is standing on.
// ---------------------------------------------------------------------------

let groundHeight = (x: float, z: float): float =>
  Math.sin(x * 0.31) * 0.22
    + Math.sin(z * 0.27 + 1.3) * 0.26
    + Math.sin((x + z) * 0.11) * 0.34
    - 0.4

// The heightmap spans a unit square centered on the origin, so cell (r, c)
// lands at these world coordinates once scaled to `groundSize`.
let cellX = (c: float): float => (c / (groundRes - 1.0) - 0.5) * groundSize
let cellZ = (r: float): float => (r / (groundRes - 1.0) - 0.5) * groundSize

// ---------------------------------------------------------------------------
// grow — the recursive L-system. A branch is a tapered stem plus either two
// children or a bloom, all wrapped in this joint's share of the wind.
// ---------------------------------------------------------------------------

let bloom = (sp: Species, openAmt: float, glow: float, salt: float): Scene.t =>
  let petal = (k: float): Scene.t =>
    Scene.sphere()
      |> Scene.scaleXYZ(bloomSize * 0.8, bloomSize * 0.26, bloomSize * 1.45)
      |> Scene.translate(Vec3.make(0.0, 0.0, bloomSize * 1.05))
      |> Scene.rotateX(58deg * (0.0 - openAmt))
      |> Scene.rotateY(Angle.degrees(k * 360.0 / petals + salt * 11.0))
      |> Scene.emissive(toColor(gain(glow, sp.bloom))) in
  Scene.group([
    Scene.sphere()
      |> Scene.scale(bloomSize * 0.5)
      |> Scene.emissive(toColor(gain(glow * 0.85, sp.core))),
    Scene.group(List.map(petal, List.range(petals))),
  ])
    |> Scene.scale(0.25 + 0.75 * openAmt)

// `t` is how many branch generations are still owed to this subtree. A whole
// number grows a full segment; the fractional remainder is the tip that is
// still extending, which is what makes growth continuous rather than steppy.
let grow = (sp: Species, salt: float, depth: float, t: float, glow: float, tts: float): Scene.t =>
  let ext = Math.clamp01(t) in
  let vary = 1.0 - jitter * 0.5 + jitter * hash01(salt) in
  let len = stemLength * sp.reach * Math.pow(lengthFalloff, depth) * vary * ext in
  let wid = stemWidth * Math.pow(widthFalloff, depth) in
  let stem =
    Scene.cylinder()
      |> Scene.scaleXYZ(wid, len, wid)
      |> Scene.translate(Vec3.make(0.0, len * 0.5, 0.0))
      |> Scene.lit(toColor(sp.stem)) in
  let child = (k: float): Scene.t =>
    grow(sp, salt * 2.0 + k * 0.5 + 1.0, depth + 1.0, t - 1.0, glow, tts)
      |> Scene.rotateZ(branchAngle * sp.lean * (k * 2.0 - 1.0) * vary)
      |> Scene.rotateY(branchTwist * (depth + hash01(salt + k) * jitter))
      |> Scene.translate(Vec3.make(0.0, len, 0.0)) in
  let crown =
    if t <= 1.0 || depth + 1.0 >= maxDepth then
      bloom(sp, ext, glow, salt) |> Scene.translate(Vec3.make(0.0, len, 0.0))
    else
      Scene.group([child(0.0), child(1.0)]) in
  let sway = windSway * (0.3 + depth * 0.35)
    * Math.sin(tts * windSpeed + salt * 1.7 + depth * 0.8) in
  Scene.group([stem, crown]) |> Scene.rotateZ(Angle.degrees(sway))

// Roughly how tall a plant of `levels` generations stands — the partial sum of
// the geometric stem lengths. Used to hang a point light in each crown, which
// is cheaper and steadier than trying to read a position back out of a Scene.
let crownHeight = (sp: Species, levels: float): float =>
  stemLength * sp.reach
    * (1.0 - Math.pow(lengthFalloff, levels))
    / (1.0 - lengthFalloff)

// ---------------------------------------------------------------------------
// The model — plots, species, ages. This is what survives a save.
// ---------------------------------------------------------------------------

type Plant = {
  i: float,         // plot index on the spiral — the position is DERIVED
  species: float,
  age: float,       // SECONDS alive, not growth: see the header
}

type Model = {
  plants: List<Plant>,
  pick: float,      // species the next Space plants
}

type Msg = | Sprout

// Plot `i` of a phyllotactic spiral, so plants never collide and the garden
// fills outward the way a real one seeded from its center would. The plot is
// DERIVED, never stored, so `plotRadius` and the spiral stay live laws: widen
// the bed and the garden you already grew spreads out on the next frame.
let plotArm = (i: float): float => plotRadius * Math.sqrt((i + 0.55) / maxPlants)
let plotX = (i: float): float => Math.cos(i * 2.39996) * plotArm(i)
let plotZ = (i: float): float => Math.sin(i * 2.39996) * plotArm(i)
// This plant's variation address, which every branch salts further.
let saltOf = (i: float): float => i * 1.618 + 0.37

let sow = (i: float, species: float, age: float): Plant =>
  { i: i, species: species, age: age }

let levelsOf = (p: Plant): float =>
  p.age * growthRate |> Math.clamp(0.0, maxDepth)

let plantOne = (m: Model, species: float): Model =>
  if List.length(m.plants) >= maxPlants then m
  else { m with plants: m.plants |> List.append([sow(List.length(m.plants), species, 0.0)]) }

let init = {
  plants: [
    sow(0.0, 0.0, 11.0),
    sow(1.0, 1.0, 8.0),
    sow(2.0, 2.0, 5.5),
    sow(3.0, 0.0, 3.0),
    sow(4.0, 1.0, 1.0),
  ],
  pick: 2.0,
}

let tick = (m: Model, dt: float, tts: float): Model =>
  { m with plants: m.plants |> List.map((p: Plant) => { p with age: p.age + dt }) }

let update = (m: Model, msg: Msg): Model =>
  match msg with
  | Sprout => plantOne(m, Math.mod(List.length(m.plants), 3.0))

let subscriptions = (m: Model) => Sub.every(Time.seconds(sproutEvery), Sprout)

let input = (m: Model, key: Key.t, isDown: bool): Model =>
  if not isDown then m
  else
    match key with
    | Key.Space => plantOne(m, m.pick)
    | Key.Num1 => { m with pick: 0.0 }
    | Key.Num2 => { m with pick: 1.0 }
    | Key.Num3 => { m with pick: 2.0 }
    | Key.R => init
    | _ => m

// ---------------------------------------------------------------------------
// Inline tests over the pure core (`functor -d examples/code-garden test`).
// ---------------------------------------------------------------------------

// Growth is stored as seconds and converted by a LAW, so the conversion is the
// thing worth pinning: it is linear in age and saturates at maxDepth.
expect levelsOf(sow(0.0, 0.0, 0.0)) == 0.0
expect levelsOf(sow(0.0, 0.0, 1.0 / growthRate)) == 1.0
expect levelsOf(sow(0.0, 0.0, 10000.0)) == maxDepth

// tick only ages; it never touches shape.
expect (
  let totalAge = (m: Model) => m.plants |> List.map((p: Plant) => p.age) |> List.sum in
  totalAge(tick(init, 2.0, 0.0)) == totalAge(init) + 2.0 * List.length(init.plants)
)
expect (
  let plots = (m: Model) => m.plants |> List.map((p: Plant) => p.i) in
  plots(tick(init, 2.0, 0.0)) == plots(init)
)

// Self-seeding appends one plot — at the END, so plot order is planting order —
// and stops at the cap.
expect List.length(update(init, Sprout).plants) == List.length(init.plants) + 1.0
expect (
  let volunteer = update(init, Sprout).plants |> List.last in
  (volunteer |> Option.map((p: Plant) => p.age) |> Option.defaultValue(0.0 - 1.0)) == 0.0
)
expect (
  let full = List.range(maxPlants) |> List.fold((m, i) => update(m, Sprout), init) in
  List.length(full.plants) == maxPlants
)

// Plots never coincide: the spiral separates consecutive seeds.
expect (
  let dx = plotX(3.0) - plotX(4.0) in
  let dz = plotZ(3.0) - plotZ(4.0) in
  Math.sqrt(dx * dx + dz * dz) > 0.6
)

// A keypress that is not a control leaves the garden alone, and R replants it.
expect List.length(input(init, Key.Q, true).plants) == List.length(init.plants)
expect input(init, Key.Num1, true).pick == 0.0
expect (
  let totalAge = (m: Model) => m.plants |> List.map((p: Plant) => p.age) |> List.sum in
  let old = tick(init, 50.0, 0.0) in
  totalAge(old) > totalAge(init) && totalAge(input(old, Key.R, true)) == totalAge(init)
)

// The palette blend is a real lerp with exact endpoints.
expect mix(nightFog, dayFog, 0.0).r == nightFog.r
expect mix(nightFog, dayFog, 1.0).b == dayFog.b

// ---------------------------------------------------------------------------
// draw — one pure function of (laws, model, clock).
// ---------------------------------------------------------------------------

let dayPhase = (tts: float): float => Math.mod(tts / dayLength + dayStart, 1.0)
let sunAngle = (tts: float): float => dayPhase(tts) * 2.0 * Math.pi
let daylight = (tts: float): float => Math.clamp01(Math.sin(sunAngle(tts)) * 1.5 + 0.35)
let glowOf = (tts: float): float => Math.lerp(dayGlow, daylight(tts), nightGlow)

// A celestial body on the day arc: the sun at `phase`, the moon opposite it.
// Both sit inside the fog, so they read as a disc glowing through the mist, and
// the opaque ground hides whichever one has set. The camera orbits on its own
// period, so they drift in and out of frame rather than being always visible.
let celestial = (a: float, c: Rgb, size: float): Scene.t =>
  Scene.sphere()
    |> Scene.scale(size)
    |> Scene.emissive(toColor(c))
    |> Scene.translate(
         Vec3.make(Math.sin(a) * 15.0, Math.sin(a) * 10.0 + 1.5, Math.cos(a) * 15.0))

let plantNode = (glow: float, tts: float, p: Plant): Scene.t =>
  let x = plotX(p.i) in
  let z = plotZ(p.i) in
  grow(speciesOf(p.species), saltOf(p.i), 0.0, levelsOf(p), glow, tts)
    |> Scene.translate(Vec3.make(x, groundHeight(x, z), z))

// Fireflies: a handful of motes drifting over the bed after dusk. They fill the
// night sky, which is otherwise a flat wash, and cost one node each. Emissive
// black would still read as a dark speck against a bright sky, so at full
// daylight the whole group is dropped rather than faded.
let fireflies = (d: float, tts: float): Scene.t =>
  if d > 0.8 then Scene.group([])
  else
    let mote = (k: float): Scene.t =>
      let a = hash01(k * 3.7) * 6.2832 + tts * (0.10 + hash01(k * 9.1) * 0.22) in
      let r = 1.4 + hash01(k * 5.3) * 4.4 in
      let y = 1.0 + hash01(k * 2.9) * 2.4 + Math.sin(tts * 0.8 + k) * 0.3 in
      Scene.sphere()
        |> Scene.scale(0.05)
        |> Scene.emissive(toColor(gain(Math.clamp01(1.0 - d * 1.25), moteColor)))
        |> Scene.translate(Vec3.make(Math.cos(a) * r, y, Math.sin(a) * r)) in
    Scene.group(List.map(mote, List.range(motes)))

// A point light hung in a plant's crown, tinted by its own blooms — the reason
// the stems are visible at all at night. The engine evaluates at most 8 lights
// per draw and SILENTLY DROPS the rest, so the ambient + sun pair leaves room
// for exactly 6 of these: `lightsFor` gives them to the six oldest plants and
// the youngest three glow without lighting their surroundings.
let crownLight = (tts: float, p: Plant): Light.t =>
  let sp = speciesOf(p.species) in
  let levels = levelsOf(p) in
  let x = plotX(p.i) in
  let z = plotZ(p.i) in
  Light.point(
    Vec3.make(x, groundHeight(x, z) + crownHeight(sp, levels) * 0.62, z),
    toColor(sp.bloom),
    (0.55 + 1.15 * (levels / maxDepth)) * Math.clamp01(1.12 - daylight(tts) * 1.12),
    7.0)

let lightsFor = (m: Model, tts: float): List<Light.t> =>
  let d = daylight(tts) in
  let a = sunAngle(tts) in
  let sunUp = Math.sin(a) in
  let key =
    Light.directional(
      Vec3.make(0.0 - Math.cos(a) * 0.7, 0.0 - Math.abs(sunUp) - 0.25, 0.0 - 0.55),
      toColor(mix(nightSun, daySun, d)),
      0.12 + 0.95 * d)
      |> Light.castShadows in
  let crowns =
    m.plants
      |> List.sortBy((p: Plant) => 0.0 - p.age)
      |> List.take(6.0)
      |> List.map((p: Plant) => crownLight(tts, p)) in
  [Light.ambient(toColor(mix(nightAmbient, dayAmbient, d))), key] |> List.append(crowns)

let draw = (m: Model, tts: float) =>
  let d = daylight(tts) in
  let glow = glowOf(tts) in
  let a = sunAngle(tts) in
  let fogColor = mix(nightFog, dayFog, d) in
  let ground =
    Scene.heightmap(
      List.grid((r, c) => groundHeight(cellX(c), cellZ(r)), groundRes, groundRes))
      |> Scene.scaleXYZ(groundSize, 1.0, groundSize)
      |> Scene.lit(toColor(mix(groundNight, groundDay, d))) in
  let sky =
    Scene.group([
      celestial(a, gain(0.12 + 1.05 * d, daySun), 3.0),
      celestial(a + Math.pi, gain(1.0 - 0.75 * d, nightSun), 1.7),
    ]) in
  let garden = Scene.group(m.plants |> List.map((p: Plant) => plantNode(glow, tts, p))) in
  let motesNode = fireflies(d, tts) in
  let eye =
    Vec3.make(
      Math.sin(camStart + tts * camSpeed) * camRadius,
      camHeight,
      Math.cos(camStart + tts * camSpeed) * camRadius) in
  Frame.createLit(
    Camera3D.lookAt(eye, Vec3.make(0.0, camTarget, 0.0)) |> Camera3D.clip(0.1, 140.0),
    Scene.group([ground, sky, garden, motesNode]),
    lightsFor(m, tts))
    |> Frame.withFog(Fog.linear(11.0, 42.0, toColor(fogColor)))
    |> Frame.withClearColor(toColor(mix(nightSky, daySky, d)))
