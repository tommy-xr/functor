// The Quest tool APK's boot scene: what the headset shows before any game is
// pushed over the network (POST /reload-source replaces it live, model
// preserved). VR-adapted from the CLI's 3d template: a floor plane for
// spatial reference, and a spinning cube + sphere ~2.5m in front of the
// stage origin at standing eye height. The camera below is ignored on the
// headset (per-eye HMD-pose cameras own the view — look around!); it makes
// the same scene sensible if run on desktop.

let init = {}

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 1.6, 0.5), Vec3.make(0.0, 1.2, -2.5)),
    Scene.group([
      Scene.plane()
        |> Scene.scale(20.0)
        |> Scene.lit(Color.rgb(0.35, 0.38, 0.42)),
      Scene.cube()
        |> Scene.scale(0.5)
        |> Scene.rotateY(Angle.radians(tts * 0.5))
        |> Scene.translate(Vec3.make(0.0, 1.2, -2.5))
        |> Scene.lit(Color.rgb(0.25, 0.65, 1.0)),
      Scene.sphere()
        |> Scene.scale(0.3)
        |> Scene.translate(Vec3.make(-1.0, 1.0, -2.0))
        |> Scene.lit(Color.rgb(1.0, 0.35, 0.25)),
    ]),
    [
      Light.ambient(Color.rgb(0.12, 0.12, 0.16)),
      Light.directional(Vec3.make(0.5, -1.0, 0.35), Color.rgb(1.0, 0.96, 0.9), 1.0)
        |> Light.castShadows,
    ])
