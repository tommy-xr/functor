// Mixed-reality parts viewer: printable parts (parts.fun) at true scale in
// your real room, before you print them. 1 mm in a part is 1 mm in the room.
// The stdlib's PartMR does the work; this file picks the parts.
//
// Quest:   functor -d examples/parts-mr run vr
//          squeeze the right grip near a part to hold it; let go and it drops
//          onto the real surface below. B takes it apart (each piece held on
//          its own) and back together; the right thumbstick turns and tilts,
//          A cycles parts, X toggles passthrough
// Desktop: functor -d examples/parts-mr run native --emulate-xr
//          a stand-in room with a desk; Enter squeezes, arrows are the
//          thumbstick, 1 (or N) cycles, B takes apart, P toggles passthrough

// The parts A cycles through. Built once here, so each is meshed once.
let views = PartMR.views([
  { name: "Tag", pieces: Parts.tag.pieces },
  { name: "Bookmark", pieces: Parts.bookmark.pieces },
  { name: "Cube", pieces: Parts.calibrationCube.pieces },
])

let config = PartMR.config()
let settings = PartMR.settings
let physics = PartMR.physics
let init = PartMR.init(views)
let sampledInput = (m, s) => PartMR.sampledInput(views, m, s)
let tick = (m, dt, tts) => PartMR.tick(views, m, dt)
let draw = (m, tts) => PartMR.draw(views, m)

expect PartMR.mm(60.0) == "60"
expect PartMR.mm(1.8000001) == "1.8"
expect PartMR.cycled(views, { init with index: List.length(views) - 1.0 }).index == 0.0

// A desk-top at 75 cm wherever a part is.
let desk = (p) => Option.Some(0.75)
let settled = PartMR.tickWith(views, init, 10.0, desk)
// Released, the 4 mm tag drops onto the surface: its foot on the desk.
expect Math.abs(settled.whole.position.y - 0.752) < 0.000001
// Over nothing seen yet, it stays where it is.
expect PartMR.tickWith(views, init, 10.0, (p) => Option.None).whole.position.y == init.whole.position.y

// Assembled, the grip takes the whole tag; apart, the piece nearest it.
let atTag = { settled with gripTracked: true, grip: { settled.grip with position: settled.whole.position } }
expect PartMR.inReach(views, atTag) == 0.0
let apart = PartMR.apartToggled(views, atTag)
expect apart.apart && List.length(apart.parts) == 2.0
// Taken apart, nothing jumps or sinks: each piece stays exactly where the
// assembly had it, even after a tick, until it is held and let go.
let near = (a, b) => Math.abs(a.x - b.x) + Math.abs(a.y - b.y) + Math.abs(a.z - b.z) < 0.000001
let still = PartMR.tickWith(views, apart, 0.5, desk)
expect List.zip(apart.parts, still.parts) |> List.all((pair) => let (b, a) = pair in near(a.position, b.position))
// Turned and tilted, the pieces still sit where the assembly draws them:
// the relief's home is 1.6 mm above the plate's middle, so tilted 90° it
// lies 1.6 mm toward the viewer's side of z.
let tilted = PartMR.apartToggled(views, { atTag with whole: { atTag.whole with pitch: Math.pi / 2.0 } })
expect List.nth(1.0, tilted.parts) |> Option.map((p) => Math.abs(p.position.z - atTag.whole.position.z) > 0.0001) |> Option.defaultValue(false)
// Held, a piece follows the hand and keeps where it was held.
let held = PartMR.grabbed(views, apart)
let moved = { held with grip: { held.grip with position: PartMR.plus(held.grip.position, PartMR.point(0.3, 0.0, 0.0)) } }
let carried = PartMR.tickWith(views, moved, 0.1, desk)
expect held.held >= 0.0
expect List.nth(held.held, carried.parts) |> Option.map((p) => Math.abs(p.position.x - 0.3 - (List.nth(held.held, apart.parts) |> Option.map((q) => q.position.x) |> Option.defaultValue(0.0))) < 0.000001) |> Option.defaultValue(false)
// The thumbstick turns the part in the hand.
let spun = PartMR.tickWith(views, { held with stick: { x: 1.0, y: 0.0 } }, 0.5, desk)
expect List.nth(held.held, spun.parts) |> Option.map((p) => Math.abs(p.yaw - 1.0) < 0.000001) |> Option.defaultValue(false)
// Let go, it rests on the desk.
let dropped = PartMR.tickWith(views, PartMR.released(carried), 10.0, desk)
expect List.nth(held.held, dropped.parts) |> Option.map((p) => p.position.y < 0.76) |> Option.defaultValue(false)
// Tipped from level to straight down or up, a hand keeps facing ahead.
let pose = (fy, fz, uy, uz) => { position: PartMR.point(0.0, 0.0, 0.0), forward: PartMR.point(0.0, fy, fz), up: PartMR.point(0.0, uy, uz) }
expect [pose(0.0, 1.0, 1.0, 0.0), pose(-0.98, 0.2, 0.2, 0.98), pose(-1.0, 0.0, 0.0, 1.0), pose(0.98, 0.2, 0.2, -0.98), pose(1.0, 0.0, 0.0, -1.0)]
  |> List.all((p) => Math.abs(PartMR.heading(p)) < 0.000001)
// A squeeze near halfway neither drops nor retakes.
expect PartMR.squeezed(true, 0.5) && not PartMR.squeezed(false, 0.5)
// Back together, it is one item again.
expect List.length(PartMR.items(views, PartMR.apartToggled(views, apart))) == 1.0
