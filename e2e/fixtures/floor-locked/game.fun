// e2e/floor-locked.mjs's fixture: records the XR head, rig-local and mapped
// through a level rig at the player's feet. The harness strips `config` for
// the legacy (head-centred) run.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.floorLocked(true))

let rig =
  Camera3D.firstPerson(
    Vec3.make(2.0, 0.0, 3.0), Angle.radians(0.0), Angle.radians(0.0), Angle.degrees(60.0))

let init = { head: Option.None, eye: Option.None }

let sampledInput = (m, s: Input.snapshot) =>
  match s.xr with
  | Option.None => m
  | Option.Some(xr) =>
    match xr.head with
    | Option.None => m
    | Option.Some(head) =>
      { head: Option.Some(head), eye: Option.Some(Camera3D.mapTrackedPose(rig, head)) }

let tick = (m, dt, tts) => m

let draw = (m, tts) => Frame.create(rig, Scene.group([]))
