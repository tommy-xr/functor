// A VR Functor starter: a floor, both tracked controllers, and a cube to pick
// up. Hold a trigger with the controller at the cube to carry it; let go to
// leave it where it is (there is no physics, so it stays in the air). Run it:
//
//   functor run vr                    # on an adb-attached headset
//   functor run native --emulate-xr   # desktop: the mouse moves the right
//                                     #   controller, Space is its trigger
//
// The same `sampledInput` reads a headset and the desktop emulator. Without
// `--emulate-xr` the desktop shows the scene with no controllers.

// The head's starting view: standing, looking down +Z. Tracked poses are
// relative to it, so `Camera3D.mapTrackedPose` places them in the world.
let camera = Camera3D.firstPerson(Vec3.make(0.0, 1.6, 0.0), 0deg, 0deg, 70deg)

let reach = 0.15   // how close (metres) a controller must be to grab the cube
let size = 0.08    // the cube's edge

// One controller as the game sees it: its world pose, `None` while untracked.
type Hand = { pose: Option.t<Camera3D.mappedPose>, trigger: bool }

type Holder =
  | Nobody
  | LeftHand
  | RightHand

let untracked: Hand = { pose: Option.None, trigger: false }

let init = {
  left: untracked,
  right: untracked,
  holder: Nobody,
  cube: { x: -0.1, y: 1.35, z: 0.55 },
  offset: { x: 0.0, y: 0.0, z: 0.0 },   // the cube from the holding hand
}

let readHand = (controller: Input.controller): Hand =>
  if controller.active then
    {
      pose: controller.grip |> Option.map((pose) => Camera3D.mapTrackedPose(camera, pose)),
      trigger: controller.trigger > 0.5,
    }
  else untracked

let near = (hand: Hand, p: Input.point3) =>
  match hand.pose with
  | Option.None => false
  | Option.Some(pose) =>
    let dx = pose.position.x - p.x in
    let dy = pose.position.y - p.y in
    let dz = pose.position.z - p.z in
    dx * dx + dy * dy + dz * dz < reach * reach

// A trigger pressed (this step, not held from before) at the free cube grabs
// it; releasing the holding trigger drops it.
let grabs = (before: Hand, now: Hand, cube) =>
  now.trigger && not before.trigger && near(now, cube)

let nextHolder = (model, left: Hand, right: Hand) =>
  match model.holder with
  | Nobody =>
    if grabs(model.right, right, model.cube) then RightHand
    else if grabs(model.left, left, model.cube) then LeftHand
    else Nobody
  | LeftHand => if left.trigger then LeftHand else Nobody
  | RightHand => if right.trigger then RightHand else Nobody

let holding = (holder, left: Hand, right: Hand) =>
  match holder with
  | Nobody => Option.None
  | LeftHand => left.pose
  | RightHand => right.pose

let minus = (a: Input.point3, b: Input.point3): Input.point3 =>
  { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z }

let plus = (a: Input.point3, b: Input.point3): Input.point3 =>
  { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }

// One step: read who holds the cube, then move a held cube with its hand.
let step = (model, left: Hand, right: Hand) =>
  let holder = nextHolder(model, left, right) in
  let next = { model with left: left, right: right, holder: holder } in
  match holding(holder, left, right) with
  | Option.None => next
  | Option.Some(pose) =>
    // The cube keeps its offset from the hand that grabbed it, so it does not
    // jump into the hand.
    let offset =
      if model.holder == Nobody then minus(model.cube, pose.position) else model.offset
    in
    { next with offset: offset, cube: plus(pose.position, offset) }

let sampledInput = (model, snapshot: Input.snapshot) =>
  match snapshot.xr with
  | Option.None => step(model, untracked, untracked)
  | Option.Some(xr) => step(model, readHand(xr.left), readHand(xr.right))

let tick = (model, dt, tts) => model

let at = (p: Input.point3, scene) => scene |> Scene.translate(Vec3.make(p.x, p.y, p.z))

// A controller is a small box along its grip, drawn only while tracked.
let controller = (hand: Hand, color) =>
  match hand.pose with
  | Option.None => Scene.group([])
  | Option.Some(pose) =>
    Scene.cube()
      |> Scene.scaleXYZ(0.04, 0.03, 0.1)
      |> Scene.lit(if hand.trigger then Color.rgb(1.0, 1.0, 1.0) else color)
      |> Scene.atPose(pose)

let draw = (model, tts) =>
  Frame.createLit(
    camera,
    Scene.group([
      Scene.plane() |> Scene.scale(40.0) |> Scene.lit(Color.rgb(0.3, 0.32, 0.36)),
      // A pedestal under the cube's starting spot.
      Scene.cylinder()
        |> Scene.scaleXYZ(0.15, 1.3, 0.15)
        |> Scene.translate(Vec3.make(-0.1, 0.65, 0.55))
        |> Scene.lit(Color.rgb(0.2, 0.22, 0.3)),
      Scene.cube()
        |> Scene.scale(size)
        |> Scene.lit(
          if model.holder == Nobody then Color.rgb(0.25, 0.65, 1.0)
          else Color.rgb(1.0, 0.35, 0.25))
        |> at(model.cube),
      controller(model.left, Color.rgb(0.55, 0.45, 1.0)),
      controller(model.right, Color.rgb(0.2, 0.9, 0.6)),
    ]),
    [
      Light.ambient(Color.rgb(0.2, 0.2, 0.25)),
      Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.96, 0.9), 1.0),
    ])

// `functor test` runs every `expect` in the project. Controller poses are plain
// records, so grabbing is testable with no headset.
let handAt = (x, y, z, trigger): Hand => {
  pose: Option.Some({
    position: { x: x, y: y, z: z },
    forward: { x: 0.0, y: 0.0, z: 1.0 },
    up: { x: 0.0, y: 1.0, z: 0.0 },
  }),
  trigger: trigger,
}

expect step(init, untracked, handAt(-0.1, 1.4, 0.6, true)).holder == RightHand
expect step(init, untracked, handAt(0.5, 1.4, 0.6, true)).holder == Nobody
// A grabbed cube follows its hand, and stays put once the trigger is released.
expect (
  let held = step(init, untracked, handAt(-0.1, 1.4, 0.6, true)) in
  let moved = step(held, untracked, handAt(0.0, 1.4, 0.6, true)) in
  let dropped = step(moved, untracked, handAt(0.3, 1.4, 0.6, false)) in
  moved.cube.x == 0.0 && dropped.holder == Nobody && dropped.cube.x == 0.0
)
