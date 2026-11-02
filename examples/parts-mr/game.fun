// Mixed-reality parts viewer: printable parts (parts.fun) at true scale on
// your real desk, before you print them. 1 mm in a part is 1 mm in the room.
// The stdlib's PartMR does the work; this file picks the parts and the desk.
//
// Quest:   functor -d examples/parts-mr run vr
//          right trigger grabs, right thumbstick turns (left/right) and
//          tilts (forward/back), A cycles parts, X toggles passthrough
// Desktop: functor -d examples/parts-mr run native --emulate-xr
//          a stand-in room; mouse + left-click (or Space) grabs, arrows are
//          the thumbstick, 1 (or N) cycles parts, P toggles passthrough
//
// Start it seated, facing your desk: the desk is placed ahead of where you
// faced at launch.

// Your desk's height above the floor, in metres. Edit and save: the part
// settles onto the new height without restarting.
let desk = { PartMR.desk with height: 0.75 }

// The parts A cycles through. Built once here, so each is meshed once.
let views = PartMR.views([
  { name: "Tag", pieces: Parts.tag.pieces },
  { name: "Bookmark", pieces: Parts.bookmark.pieces },
  { name: "Cube", pieces: Parts.calibrationCube.pieces },
])

let config = PartMR.config()
let settings = PartMR.settings
let init = PartMR.init(views, desk)
let sampledInput = (m, s) => PartMR.sampledInput(views, m, s)
let tick = (m, dt, tts) => PartMR.tick(views, desk, m, dt)
let draw = (m, tts) => PartMR.draw(views, desk, m)

expect PartMR.mm(60.0) == "60"
expect PartMR.mm(1.8000001) == "1.8"
let tag = List.head(views)
expect tag |> Option.map((v) => Math.abs(PartMR.restY(desk, v, 0.0) - (desk.height + 0.002)) < 0.000001) |> Option.defaultValue(false)
// Stood on its long edge, the tag rests on half its 30 mm depth.
expect tag |> Option.map((v) => Math.abs(PartMR.restY(desk, v, Math.pi / 2.0) - (desk.height + 0.015)) < 0.000001) |> Option.defaultValue(false)
expect PartMR.cycled(views, { init with index: 2.0 }).index == 0.0
// Released off the desk, a part settles back inside its edges.
expect tick({ init with position: PartMR.point(2.0, 1.0, 0.0) }, 10.0, 0.0).position.x == desk.width / 2.0
