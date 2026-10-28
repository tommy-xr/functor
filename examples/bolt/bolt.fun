// examples/bolt — a printable M8 hex bolt and nut with real ISO threads.
// The nut carries the `Fit` gap, so it takes a metal M8 bolt too. Both print
// standing up, side by side.
//
//   functor -d examples/bolt export -o bolt.stl
//   functor -d examples/bolt run native
//
// Units are millimetres, Z up.

let size = Thread.m8
let fit = Fit.clearance
let length = 30.0
// ISO 4017 head: 13 mm across flats, 5.3 mm tall.
let headWidth = 13.0
let headHeight = 5.3
let nutHeight = Hardware.m8.nutHeight

let lift = (z: float, s) => Solid.translate(Vec3.make(0.0, 0.0, z), s)

// The thread starts 1 mm inside the head so the two fuse.
let shankStart = headHeight - 1.0
let tip = headHeight + length

// A 45° chamfer at the tip helps the nut start: a cone narrowing to 1 mm
// under the major radius at the tip, as wide as it is tall.
let chamfer = Solid.cone(size.diameter / 2.0 - 1.0 + tip, size.diameter / 2.0 - 1.0, tip) |> lift(tip / 2.0)

let bolt =
  Solid.union([
    Hardware.hex(headWidth, headHeight),
    Thread.rod(size, length + 1.0) |> lift(shankStart),
  ])
    |> Solid.intersect(chamfer)

// The nut's thread runs 1 mm past both faces.
let nut = Hardware.hex(Hardware.m8.nutWidth, nutHeight) |> Solid.subtract(Thread.tap(size, fit, nutHeight + 2.0) |> lift(-1.0))

let part = Solid.group([
  bolt |> Solid.translate(Vec3.make(-12.0, 0.0, 0.0)),
  nut |> Solid.translate(Vec3.make(12.0, 0.0, 0.0)),
])

expect Math.abs(Vec3.z(Solid.bounds(bolt).max) - tip) < 0.001
expect Math.abs(Vec3.z(Solid.bounds(nut).max) - nutHeight) < 0.001

// The fit still leaves over half the thread depth engaged.
expect Thread.engagement(size, fit) > Thread.engagement(size, Fit.gap(0.0)) / 2.0

// Screwed on — its thread a whole number of pitches from the bolt's — the nut
// doesn't touch the bolt; half a pitch off, it collides: the threads engage.
let nutAt = (pitches: float) => nut |> lift(shankStart + 1.0 + pitches * size.pitch)
expect Solid.volume(Solid.intersect(bolt, nutAt(8.0))) < 0.000001
expect Solid.volume(Solid.intersect(bolt, nutAt(8.5))) > 1.0
