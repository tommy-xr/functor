// touch e2e fixture: logs "e2e-touch surface" once when the snapshot first
// carries the touch domain (capability — Some with empty lists), then one
// "e2e-touch tap …" line once a press AND a release edge have both been
// sampled (a quick tap can put them in the same snapshot), echoing the press
// position so the test can assert canvas-relative CSS coordinates.

type Model = { surfaced: bool, taps: float, rels: float, x: float, y: float, logged: bool }

let init = { surfaced: false, taps: 0.0, rels: 0.0, x: 0.0, y: 0.0, logged: false }

let sampledInput = (m: Model, snap: Input.snapshot) =>
  match snap.touch with
  | Option.Some(t) =>
    let surfaced =
      if not m.surfaced then
        // Text.length is only a sequencing device: binding the log's return
        // forces it before the flag flips.
        let line: string = Debug.log("e2e-touch", "surface") in
        Text.length(line) > 0.0
      else m.surfaced in
    let pressed =
      t.pressed
      |> List.head
      |> Option.map((p: Input.touchPoint) => { m with taps: m.taps + 1.0, x: p.x, y: p.y })
      |> Option.defaultValue(m) in
    let counted =
      { pressed with surfaced: surfaced, rels: pressed.rels + List.length(t.released) } in
    if not counted.logged && counted.taps > 0.0 && counted.rels > 0.0 then
      let line: string =
        Debug.log("e2e-touch", $"tap x={counted.x} y={counted.y} rels={counted.rels}") in
      { counted with logged: Text.length(line) > 0.0 }
    else counted
  | Option.None => m

let tick = (m: Model, dt, tts) => m

let draw = (m: Model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.translate(Vec3.make(m.x / 100.0, 0.0, 0.0)),
  )
