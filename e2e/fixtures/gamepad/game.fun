// gamepad e2e fixture: log ONE line when the sampled snapshot first carries a
// pad with south held, echoing the stick so the test can assert the whole
// standard-mapping conversion (value + up-positive Y) crossed the boundary.

type Model = { logged: bool }

let init = { logged: false }

let sampledInput = (m: Model, snap: Input.snapshot) =>
  match snap.gamepad with
  | Option.Some(pad) =>
    if pad.south && not m.logged then
      let line: string =
        Debug.log(
          "e2e-gamepad",
          $"south x={pad.leftStick.x} y={pad.leftStick.y} rt={pad.rightTrigger}",
        ) in
      { logged: Text.length(line) > 0.0 }
    else m
  | Option.None => m

let tick = (m: Model, dt, tts) => m

let draw = (m: Model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube(),
  )
