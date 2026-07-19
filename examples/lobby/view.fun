// view.fun — shared scene scaffolding for the three lobby roles (file =
// module): the same ground plane, camera, and lights, so the role panes read
// consistently side by side.

let frame = (nodes: List<Scene.t>) =>
  let ground =
    Scene.plane() |> Scene.scale(8.0) |> Scene.lit(Color.rgb(0.18, 0.2, 0.28)) in
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 4.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([ground, ..nodes]),
    [ Light.ambient(Color.rgb(0.35, 0.35, 0.42)),
      Light.directional(Vec3.make(-0.4, -1.0, -0.35), Color.rgb(1.0, 0.95, 0.85), 1.1) ])
