// view.fun — the scene both roles render: one colored cube per player row on
// the arena ground plane. Client and server share the exact framing and
// palette, so their panes line up in the netsim viewer.

let world = (rows: List<Protocol.Row>) =>
  let playerNodes =
    rows |> List.map((r) =>
      let (cr, cg, cb) = Protocol.colorFor(r.pid) in
      Scene.cube()
      |> Scene.scale(0.6)
      |> Scene.translate(Vec3.make(r.x, 0.0, r.z))
      |> Scene.lit(Color.rgb(cr, cg, cb))) in
  // Ground sized to the wrap boundary, so the playfield edges are visible.
  let ground =
    Scene.plane()
    |> Scene.scale(2.0 * Protocol.arena)
    |> Scene.lit(Color.rgb(0.18, 0.2, 0.28)) in
  let scene = Scene.group([ground, ..playerNodes]) in
  // Top-down-ish view so player movement stays on screen.
  let camera =
    Camera.firstPerson(
      Vec3.make(0.0, 9.0, -2.0),
      Angle.radians(0.0), Angle.radians(-1.2), Angle.degrees(70.0)) in
  Frame.createLit(
    camera, scene,
    [ Light.ambient(Color.rgb(0.35, 0.35, 0.42)),
      Light.directional(Vec3.make(-0.4, -1.0, -0.35), Color.rgb(1.0, 0.95, 0.85), 1.1) ])
