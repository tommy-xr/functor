// neon-city — a procedural synthwave city you walk at night.
//
// A seeded grid of blocks and streets, a district profile (downtown spires,
// mid-rise, low sprawl), and a few thousand buildings — every one of them,
// and every neon strip, band and sign on them, drawn by a handful of
// `Scene.instanced` calls over a cube. The city is generated ONCE from the
// top-level parameters below, so editing any of them hot-reloads a new city.
//
// The wet street is faked: the whole lit city is mirrored below y = 0 and seen
// through a translucent dark floor. The sky, horizon glow and sun are a
// generated skybox (`node examples/neon-city/generate-sky.mjs`), which fog
// never touches, so the far grid dissolves into the horizon.
//
//   functor -d examples/neon-city run native
//   functor -d examples/neon-city test

// ---- Layout ------------------------------------------------------------------

let seed = 7.0
let blocksPerSide = 24.0
let blockSize = 60.0
let streetWidth = 16.0
let avenueWidth = 28.0
// Every Nth street (counting the outer edges) is an avenue.
let avenueEvery = 6.0
// Inset from the block edge to the first building, and the gap between lots.
let sidewalk = 3.0
let lotGap = 2.5
let emptyLotChance = 0.07
let sidewalkHeight = 0.2

// ---- Districts and buildings -------------------------------------------------

let downtownX = 120.0
let downtownZ = 420.0
let downtownRadius = 430.0
let midtownX = -560.0
let midtownZ = -300.0
let midtownRadius = 300.0
let midtownWeight = 0.75
let sprawlHeight = 8.0
let downtownHeight = 230.0
let heightCurve = 2.0
// Above this a tower gets setback tiers.
let spireHeight = 95.0
// Big lots downtown, small lots out in the sprawl.
let bigLotDistrict = 0.55

// ---- Neon --------------------------------------------------------------------

let stripChance = 0.45
let bandChance = 0.65
let bandSpacing = 9.0
let maxBands = 10.0
let bandGlow = 0.45
let roofSignChance = 0.35
let streetSignChance = 0.6
let storefrontChance = 0.8
let dashSpacing = 12.0
let carCount = 40.0
let pulseSpeed = 2.2
let flickerRate = 9.0

// ---- Atmosphere and camera ---------------------------------------------------

// Matches the skybox horizon (generate-sky.mjs) so fogged towers melt into it.
let fogColor = Color.rgb(0.42, 0.12, 0.46)
let fogNear = 40.0
let fogFar = 700.0
// Translucency of the wet street: lower shows more of the mirrored city.
let streetOpacity = 0.8
let walkEye = 1.7
let flyEye = 160.0
let walkSpeed = 9.0
let flySpeed = 60.0
let climbRate = 2.5
let sensitivity = 0.003
let pitchLimit = 1.45

// ---- Generator ---------------------------------------------------------------

// A stateless hash in [0, 1): every random choice is a pure function of
// (seed, cell, salt), so the city needs no generator threaded through it.
let hash = (s: float, a: float, b: float, salt: float): float =>
  let v = Math.sin(a * 127.1 + b * 311.7 + salt * 74.7 + s * 19.19) * 43758.5453 in
  v - Math.floor(v)

let isAvenue = (k: float): bool => Math.mod(k, avenueEvery) == 0.0
let streetWidthAt = (k: float): float => if isAvenue(k) then avenueWidth else streetWidth
// Avenues among streets 0..k.
let avenuesUpTo = (k: float): float => Math.floor(k / avenueEvery) + 1.0
let cityExtent =
  blocksPerSide * blockSize + (blocksPerSide + 1.0) * streetWidth
  + avenuesUpTo(blocksPerSide) * (avenueWidth - streetWidth)
let halfExtent = cityExtent / 2.0

// West (x) or south (z) edge of block i. Street i lies just before it, and
// street `blocksPerSide` closes the grid.
let blockStart = (i: float): float =>
  i * blockSize + (i + 1.0) * streetWidth
  + avenuesUpTo(i) * (avenueWidth - streetWidth) - halfExtent

let streets =
  List.range(blocksPerSide + 1.0)
  |> List.map((k) => { lo: blockStart(k) - streetWidthAt(k), hi: blockStart(k) })

let bump = (x: float, z: float, cx: float, cz: float, r: float): float =>
  Math.exp(-((x - cx) * (x - cx) + (z - cz) * (z - cz)) / (r * r))

// 0 = low sprawl .. 1 = downtown core: two seeded cores plus a slow wobble.
let district = (s: float, x: float, z: float): float =>
  let core = Math.max(bump(x, z, downtownX, downtownZ, downtownRadius),
                      midtownWeight * bump(x, z, midtownX, midtownZ, midtownRadius)) in
  let wobble = 0.08 * Math.sin(x * 0.004 + s) * Math.cos(z * 0.0035 - s) in
  Math.clamp01(core + wobble)

let buildingHeight = (d: float, roll: float): float =>
  (sprawlHeight + (downtownHeight - sprawlHeight) * Math.pow(d, heightCurve))
  * (0.55 + 0.9 * roll)

// The street-facing facade that gets signs: 1 west, 2 east, 3 south, 4 north,
// 0 none (the centre lot of a 3×3 block). Corner lots pick their west/east
// side, so north/south facades only get signs on a 3×3 block's middle column.
let streetSide = (li: float, lj: float, n: float): float =>
  if li == 0.0 then 1.0
  else if li == n - 1.0 then 2.0
  else if lj == 0.0 then 3.0
  else if lj == n - 1.0 then 4.0
  else 0.0

let blockLots = (s: float, i: float, j: float) =>
  let x0 = blockStart(i) + sidewalk in
  let z0 = blockStart(j) + sidewalk in
  let inner = blockSize - 2.0 * sidewalk in
  let d = district(s, x0 + inner / 2.0, z0 + inner / 2.0) in
  let n = if d > bigLotDistrict then 2.0 else 3.0 in
  let size = inner / n in
  List.range(n * n)
  |> List.map((k) =>
       let li = Math.mod(k, n) in
       let lj = Math.floor(k / n) in
       { s: s, a: i * 8.0 + li, b: j * 8.0 + lj, x0: x0 + li * size, z0: z0 + lj * size,
         size: size, district: d, side: streetSide(li, lj, n) })

let building = (lot) =>
  let r = (salt) => hash(lot.s, lot.a, lot.b, salt) in
  let foot = lot.size - lotGap in
  let w = foot * (0.72 + 0.28 * r(1.0)) in
  let d = foot * (0.72 + 0.28 * r(2.0)) in
  { s: lot.s, a: lot.a, b: lot.b,
    x: lot.x0 + lotGap / 2.0 + w / 2.0 + (foot - w) * r(3.0),
    z: lot.z0 + lotGap / 2.0 + d / 2.0 + (foot - d) * r(4.0),
    w: w, d: d, h: buildingHeight(lot.district, r(5.0)), side: lot.side }

let cityFor = (s: float) =>
  List.range(blocksPerSide * blocksPerSide)
  |> List.concatMap((k) => blockLots(s, Math.mod(k, blocksPerSide), Math.floor(k / blocksPerSide)))
  |> List.filter((lot) => hash(s, lot.a, lot.b, 6.0) > emptyLotChance)
  |> List.map(building)

let buildings = cityFor(seed)

// A building's stacked boxes: one, or three setback tiers for a spire.
let massesOf = (b) =>
  let tier = (t, scale, y0, h) =>
    { s: b.s, a: b.a + t * 0.37, b: b.b, top: false,
      x: b.x, z: b.z, w: b.w * scale, d: b.d * scale, y0: y0, h: h } in
  if b.h < spireHeight then [{ tier(0.0, 1.0, 0.0, b.h) with top: true }]
  else
    let h1 = b.h * 0.6 in
    let h2 = b.h * 0.28 in
    [ tier(0.0, 1.0, 0.0, h1),
      tier(1.0, 0.72, h1, h2),
      { tier(2.0, 0.42, h1 + h2, b.h - h1 - h2) with top: true } ]

let masses = buildings |> List.concatMap(massesOf)

// ---- Neon pieces (plain data; turned into instances below) -------------------

// One emissive box: centre, size, palette slot, brightness, and animation group
// (0 steady, 1 and 2 pulse out of phase, 3 flickers).
let neon = (x, y, z, sx, sy, sz, color, glow, group) =>
  { x: x, y: y, z: z, sx: sx, sy: sy, sz: sz, color: color, glow: glow, group: group }

let paletteSlot = (roll: float): float => Math.floor(roll * 4.999)
let animGroup = (roll: float): float =>
  if roll < 0.1 then 1.0 else if roll < 0.2 then 2.0 else if roll < 0.25 then 3.0 else 0.0

let corners = [{ u: -1.0, v: -1.0 }, { u: 1.0, v: 1.0 }, { u: 1.0, v: -1.0 }, { u: -1.0, v: 1.0 }]

// Vertical strips up two or four corners of a mass.
let edgeStrips = (m) =>
  let r = (salt) => hash(m.s, m.a, m.b, salt) in
  if r(11.0) > stripChance then []
  else
    corners
    |> List.take(if r(13.0) > 0.5 then 4.0 else 2.0)
    |> List.map((c) => neon(m.x + c.u * m.w / 2.0, m.y0 + m.h / 2.0, m.z + c.v * m.d / 2.0,
                            0.35, m.h, 0.35, paletteSlot(r(12.0)), 1.0, 0.0))

// Rings of light around a mass every few floors, brightest at the crown.
let bands = (m) =>
  let r = (salt) => hash(m.s, m.a, m.b, salt) in
  if m.h < 12.0 || r(21.0) > bandChance then []
  else
    let count = Math.min(maxBands, Math.floor(m.h / bandSpacing)) in
    let step = m.h / (count + 1.0) in
    let color = paletteSlot(r(22.0)) in
    List.range(count)
    |> List.map((k) =>
         let crown = k == count - 1.0 in
         neon(m.x, m.y0 + step * (k + 1.0), m.z, m.w + 0.4, if crown then 0.9 else 0.5, m.d + 0.4,
              color, if crown then 1.0 else bandGlow, 0.0))

// A sign standing on a low roof.
let roofSign = (m) =>
  let r = (salt) => hash(m.s, m.a, m.b, salt) in
  if not m.top || m.y0 + m.h > 70.0 || r(31.0) > roofSignChance then []
  else
    let h = 3.0 + 4.0 * r(32.0) in
    let alongX = r(33.0) > 0.5 in
    [ neon(m.x, m.y0 + m.h + h / 2.0 + 0.5, m.z,
           if alongX then m.w * 0.6 else 0.4, h, if alongX then 0.4 else m.d * 0.6,
           paletteSlot(r(34.0)), 1.0, animGroup(r(35.0))) ]

// Street level: a storefront strip and a blade sign on the street-facing side.
let streetSigns = (b) =>
  let r = (salt) => hash(b.s, b.a, b.b, salt) in
  if b.side == 0.0 then []
  else
    let west = b.side == 1.0 in
    let east = b.side == 2.0 in
    let south = b.side == 3.0 in
    let alongZ = west || east in
    let out = if west || south then -1.0 else 1.0 in
    let fx = if alongZ then b.x + out * b.w / 2.0 else b.x in
    let fz = if alongZ then b.z else b.z + out * b.d / 2.0 in
    let facadeLength = if alongZ then b.d else b.w in
    let slide = (r(41.0) - 0.5) * (facadeLength - 4.0) in
    let store =
      if r(42.0) > storefrontChance then []
      else [ neon(fx + (if alongZ then out * 0.2 else 0.0), 3.6, fz + (if alongZ then 0.0 else out * 0.2),
                  if alongZ then 0.3 else b.w * 0.9, 0.35, if alongZ then b.d * 0.9 else 0.3,
                  paletteSlot(r(43.0)), 1.0, 0.0) ] in
    // Blades hang from 4.5 m and stay below the roof.
    let h = Math.min(3.0 + 6.0 * r(44.0), b.h - 5.5) in
    let blade =
      if h < 2.0 || r(45.0) > streetSignChance then []
      else [ neon(fx + (if alongZ then out * 1.1 else slide), 4.5 + h / 2.0,
                  fz + (if alongZ then slide else out * 1.1),
                  if alongZ then 1.8 else 0.35, h, if alongZ then 0.35 else 1.8,
                  paletteSlot(r(46.0)), 1.0, animGroup(r(47.0))) ] in
    store |> List.append(blade)

// Glowing curbs around every block: magenta on avenues, cyan on streets.
let curbs =
  List.range(blocksPerSide * blocksPerSide)
  |> List.concatMap((k) =>
       let i = Math.mod(k, blocksPerSide) in
       let j = Math.floor(k / blocksPerSide) in
       let x0 = blockStart(i) in
       let z0 = blockStart(j) in
       let mid = blockSize / 2.0 in
       let y = sidewalkHeight + 0.04 in
       let slot = (street) => if isAvenue(street) then 1.0 else 0.0 in
       [ neon(x0, y, z0 + mid, 0.2, 0.08, blockSize, slot(i), 0.6, 0.0),
         neon(x0 + blockSize, y, z0 + mid, 0.2, 0.08, blockSize, slot(i + 1.0), 0.6, 0.0),
         neon(x0 + mid, y, z0, blockSize, 0.08, 0.2, slot(j), 0.6, 0.0),
         neon(x0 + mid, y, z0 + blockSize, blockSize, 0.08, 0.2, slot(j + 1.0), 0.6, 0.0) ])

// Dashed orange centre lines down both directions of every avenue.
let laneDashes =
  let avenueCentres =
    List.range(blocksPerSide + 1.0)
    |> List.filter(isAvenue)
    |> List.map((k) => blockStart(k) - avenueWidth / 2.0) in
  let along = List.range(Math.floor(cityExtent / dashSpacing))
              |> List.map((n) => n * dashSpacing + dashSpacing / 2.0 - halfExtent) in
  avenueCentres
  |> List.concatMap((c) =>
       along |> List.concatMap((t) =>
         [ neon(c, 0.03, t, 0.3, 0.04, 4.0, 4.0, 0.5, 0.0),
           neon(t, 0.03, c, 4.0, 0.04, 0.3, 4.0, 0.5, 0.0) ]))

let neonPieces =
  masses |> List.concatMap((m) => edgeStrips(m) |> List.append(bands(m)) |> List.append(roofSign(m)))
  |> List.append(buildings |> List.concatMap(streetSigns))
  |> List.append(curbs)
  |> List.append(laneDashes)

// ---- Instances ---------------------------------------------------------------

// Synthwave palette: cyan, magenta, pink, violet, orange.
let palette = (slot: float, glow: float) =>
  if slot == 0.0 then Color.rgb(0.0, 0.95 * glow, 1.0 * glow)
  else if slot == 1.0 then Color.rgb(1.0 * glow, 0.1 * glow, 0.85 * glow)
  else if slot == 2.0 then Color.rgb(1.0 * glow, 0.4 * glow, 0.65 * glow)
  else if slot == 3.0 then Color.rgb(0.62 * glow, 0.3 * glow, 1.0 * glow)
  else Color.rgb(1.0 * glow, 0.55 * glow, 0.1 * glow)

let neonInstance = (p) =>
  Instance.trs(Vec3.make(p.x, p.y, p.z), 0deg, p.sx, p.sy, p.sz)
  |> Instance.tint(palette(p.color, p.glow))

let neonGroup = (g: float) =>
  neonPieces |> List.filter((p) => p.group == g) |> List.map(neonInstance)

let steadyNeon = neonGroup(0.0)
let pulseNeonA = neonGroup(1.0)
let pulseNeonB = neonGroup(2.0)
let flickerNeon = neonGroup(3.0)

// Dark, slightly hue-shifted facades, plus one raised sidewalk slab per block.
let facadeInstance = (m) =>
  let r = (salt) => hash(m.s, m.a, m.b, salt) in
  Instance.trs(Vec3.make(m.x, m.y0 + m.h / 2.0, m.z), 0deg, m.w, m.h, m.d)
  |> Instance.tint(Color.rgb(0.75 + 0.25 * r(51.0), 0.7 + 0.2 * r(52.0), 0.85 + 0.15 * r(53.0)))

let slabInstances =
  List.range(blocksPerSide * blocksPerSide)
  |> List.map((k) =>
       Instance.trs(
         Vec3.make(blockStart(Math.mod(k, blocksPerSide)) + blockSize / 2.0, sidewalkHeight / 2.0,
                   blockStart(Math.floor(k / blocksPerSide)) + blockSize / 2.0),
         0deg, blockSize, sidewalkHeight, blockSize)
       |> Instance.tint(Color.rgb(0.5, 0.5, 0.55)))

let buildingNode =
  Scene.cube()
  |> Scene.lit(Color.rgb(0.42, 0.38, 0.52))
  |> Scene.instanced(masses |> List.map(facadeInstance) |> List.append(slabInstances))

let emissiveCubes = (brightness: float, instances) =>
  Scene.cube()
  |> Scene.emissive(Color.rgb(brightness, brightness, brightness))
  |> Scene.instanced(instances)

let steadyNode = emissiveCubes(1.0, steadyNeon)

// Hover cars: light streaks gliding along the avenues, high above the street.
let cars =
  let avenues = List.range(blocksPerSide + 1.0) |> List.filter(isAvenue) in
  List.range(carCount)
  |> List.map((c) =>
       let r = (salt) => hash(seed, c, 0.0, salt) in
       let k = List.nth(Math.floor(r(61.0) * List.length(avenues)), avenues)
               |> Option.defaultValue(0.0) in
       let dir = if r(62.0) > 0.5 then 1.0 else -1.0 in
       { alongZ: Math.mod(c, 2.0) == 0.0,
         lane: blockStart(k) - avenueWidth / 2.0 + dir * 5.0,
         dir: dir,
         y: 28.0 + 50.0 * r(63.0),
         speed: 30.0 + 30.0 * r(64.0),
         phase: r(65.0) * cityExtent })

let carNode = (tts: float) =>
  cars
  |> List.map((car) =>
       let t = Math.mod(car.phase + car.dir * car.speed * tts, cityExtent) - halfExtent in
       let tint = if car.dir > 0.0 then Color.rgb(1.0, 0.9, 0.8) else Color.rgb(1.0, 0.12, 0.25) in
       (if car.alongZ then Instance.trs(Vec3.make(car.lane, car.y, t), 0deg, 0.4, 0.4, 9.0)
        else Instance.trs(Vec3.make(t, car.y, car.lane), 0deg, 9.0, 0.4, 0.4))
       |> Instance.tint(tint))
  |> emissiveCubes(1.0)

// Everything above the street, as one list so it can be mirrored below it.
// Pulses and flicker vary only the template's brightness — the instance lists
// are built once, at load.
let cityLayers = (tts: float) =>
  let flicker = if hash(seed, Math.floor(tts * flickerRate), 0.0, 71.0) > 0.3 then 1.0 else 0.15 in
  [ buildingNode,
    steadyNode,
    emissiveCubes(0.55 + 0.45 * Math.sin(tts * pulseSpeed), pulseNeonA),
    emissiveCubes(0.55 - 0.45 * Math.sin(tts * pulseSpeed), pulseNeonB),
    emissiveCubes(flicker, flickerNeon),
    carNode(tts) ]

let street =
  Scene.plane()
  |> Scene.scale(cityExtent * 3.0)
  |> Scene.lit(Color.rgb(0.03, 0.02, 0.05))
  |> Scene.opacity(streetOpacity)

// Counted once for the HUD: each layer draws twice (it and its reflection),
// plus the street.
let drawCount = 2.0 * List.length(cityLayers(0.0)) + 1.0
let neonCount = List.length(neonPieces)

// ---- Game --------------------------------------------------------------------

let sky = Skybox.files("sky_px.png", "sky_nx.png", "sky_py.png", "sky_ny.png", "sky_pz.png", "sky_nz.png")

type Pointer =
  | NoPointer
  | PointerAt(x: float, y: float)

// Just short of the city's central crossing, looking up the avenue toward
// downtown and the sun.
let centreStreet = Math.floor(blocksPerSide / 2.0)
let centre = blockStart(centreStreet) - streetWidthAt(centreStreet) / 2.0
let spawnBack = 150.0

let init = {
  held: { up: false, down: false, left: false, right: false },
  spaceHeld: false,
  flying: false,
  x: centre,
  z: centre - spawnBack,
  eyeY: walkEye,
  yaw: 0.0,
  pitch: 0.12,
  lastMouse: NoPointer,
}

let setHeld = (held, key, isDown) =>
  match key with
  | Key.W => { held with up: isDown }
  | Key.Up => { held with up: isDown }
  | Key.S => { held with down: isDown }
  | Key.Down => { held with down: isDown }
  | Key.A => { held with left: isDown }
  | Key.Left => { held with left: isDown }
  | Key.D => { held with right: isDown }
  | Key.Right => { held with right: isDown }
  | _ => held

// Space toggles street level / overview on the press edge (key repeats also
// arrive as isDown = true).
let input = (model, key, isDown) =>
  match key with
  | Key.Space =>
    if isDown && not model.spaceHeld then
      { model with spaceHeld: true, flying: not model.flying }
    else { model with spaceHeld: isDown }
  | _ => { model with held: setHeld(model.held, key, isDown) }

let mouseMove = (model, x, y) =>
  match model.lastMouse with
  | NoPointer => { model with lastMouse: PointerAt(x, y) }
  | PointerAt(lastX, lastY) =>
    { model with
        yaw: model.yaw - (x - lastX) * sensitivity,
        pitch: model.pitch - (y - lastY) * sensitivity |> Math.clamp(-pitchLimit, pitchLimit),
        lastMouse: PointerAt(x, y) }

let axis = (negative, positive) =>
  (if positive then 1.0 else 0.0) - (if negative then 1.0 else 0.0)

// Walk (or fly) on the XZ plane; no building collision in this kata.
let tick = (model, dt, tts) =>
  let forward = axis(model.held.down, model.held.up) in
  let right = axis(model.held.left, model.held.right) in
  let speed = if model.flying then flySpeed else walkSpeed in
  // Normalised so a diagonal is no faster than straight ahead.
  let norm = Math.max(1.0, Math.sqrt(forward * forward + right * right)) in
  let dx = (forward * Math.sin(model.yaw) - right * Math.cos(model.yaw)) / norm in
  let dz = (forward * Math.cos(model.yaw) + right * Math.sin(model.yaw)) / norm in
  let target = if model.flying then flyEye else walkEye in
  { model with
      x: model.x + dx * speed * dt |> Math.clamp(-halfExtent, halfExtent),
      z: model.z + dz * speed * dt |> Math.clamp(-halfExtent, halfExtent),
      eyeY: model.eyeY + (target - model.eyeY) * Math.min(1.0, dt * climbRate) }

let draw = (model, tts) =>
  let camera =
    Camera3D.firstPerson(
      Vec3.make(model.x, model.eyeY, model.z),
      Angle.radians(model.yaw), Angle.radians(model.pitch), 70deg)
    |> Camera3D.clip(0.3, 3000.0) in
  let city = Scene.group(cityLayers(tts)) in
  Frame.createLit(
    camera,
    Scene.group([ city, city |> Scene.scaleXYZ(1.0, -1.0, 1.0), street ]),
    [ Light.ambient(Color.rgb(0.42, 0.34, 0.58)),
      Light.directional(Vec3.make(0.2, -0.35, -1.0), Color.rgb(1.0, 0.45, 0.6), 0.7) ])
  |> Frame.withFog(Fog.linear(fogNear, fogFar, fogColor))
  |> Frame.withSkybox(sky)

let ui = (model) =>
  let toggle = if model.flying then "street" else "fly" in
  Ui.text($"neon city · {List.length(buildings)} buildings · {neonCount} neon · {drawCount} draws · WASD + mouse · SPACE {toggle}")
  |> Ui.panel(Ui.topLeft())

// ---- Tests -------------------------------------------------------------------

let overlaps = (lo1: float, hi1: float, lo2: float, hi2: float): bool => lo1 < hi2 && lo2 < hi1

// Pinned counts for the default seed: a generator change shows up here first.
expect List.length(streets) == blocksPerSide + 1.0
expect List.length(buildings) == 4456.0
expect List.length(masses) == 4892.0
expect neonCount == 22414.0
expect drawCount == 13.0

// Same seed, same city; a different seed, a different one.
expect cityFor(seed) == buildings
expect cityFor(seed + 1.0) != buildings

// No building mass reaches into a street, along either axis.
expect (
  masses |> List.all((m) =>
    not (streets |> List.any((st) =>
      overlaps(m.x - m.w / 2.0, m.x + m.w / 2.0, st.lo, st.hi)
      || overlaps(m.z - m.d / 2.0, m.z + m.d / 2.0, st.lo, st.hi)))))

// The street grid tiles the city exactly: streets and blocks alternate edge to edge.
expect Math.abs(blockStart(blocksPerSide) - halfExtent) < 0.001
expect (streets |> List.head |> Option.map((st) => st.lo) |> Option.defaultValue(0.0)) == -halfExtent

// Downtown is taller than the sprawl.
expect district(seed, downtownX, downtownZ) > 0.9
expect district(seed, halfExtent, -halfExtent) < 0.2

// A diagonal is no faster than straight ahead.
expect (
  let moved = (held) =>
    let m = tick({ init with held: held }, 1.0, 0.0) in
    Math.sqrt((m.x - init.x) * (m.x - init.x) + (m.z - init.z) * (m.z - init.z)) in
  Math.abs(moved({ up: true, down: false, left: false, right: true })
           - moved({ up: true, down: false, left: false, right: false })) < 0.001
)
