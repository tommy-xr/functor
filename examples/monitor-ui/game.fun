// examples/monitor-ui — UI on a render target (the scoreboard demo).
//
// A Ui.* tree renders into the "scoreboard" render target each frame
// (Frame.withUiTarget), and a monitor mesh in the world shows it live
// (Scene.screen) — a world-space screen driven by the model. The view is
// display-only: buttons/sliders on a target render but their handlers are
// inert. All animation derives from tts, so captures are deterministic
// under --fixed-time.

// The branded target: declared ONCE, used at the writer (draw's
// Frame.withUiTarget) and the reader (the monitor's Scene.screen).
let board = RenderTarget.named("scoreboard") |> RenderTarget.sized(512.0, 256.0)

// The scoreboard content — an ordinary Ui view, laid out by the same egui
// pass as the `ui` hook, but sized to the TARGET (512x256), not the window.
let scoreboard = (score: float, tts: float) =>
  Ui.column([
    Ui.textColor(Color.rgb(0.3, 1.0, 0.9), "== ARENA SCOREBOARD =="),
    Ui.text(""),
    Ui.row([Ui.text("home"), Ui.textColor(Color.rgb(1.0, 0.8, 0.2), $"  {score}")]),
    Ui.row([Ui.text("away"), Ui.textColor(Color.rgb(1.0, 0.4, 0.4), $"  {Math.floor(score / 2.0)}")]),
    Ui.text(""),
    Ui.text($"match clock {Math.floor(tts)}s"),
  ])

// A little arena for the monitor to overlook.
let arena = (tts: float) =>
  Scene.group([
    Scene.plane() |> Scene.scale(18.0) |> Scene.lit(Color.rgb(0.5, 0.55, 0.5)),
    Scene.sphere()
      |> Scene.scale(0.5)
      |> Scene.emissive(Color.rgb(1.0, 0.55, 0.15))
      |> Scene.translate(Vec3.make(Math.cos(tts) * 2.5, 0.5, 2.0 + Math.sin(tts) * 1.5)),
    Scene.cube() |> Scene.lit(Color.rgb(0.3, 0.5, 0.9)) |> Scene.translate(Vec3.make(-2.5, 0.5, 2.5)),
  ])

// The monitor: a dark bezel block with the screen on its camera-facing side.
// A quad's front is +Z and the main camera looks down +Z, so the screen is
// rotated 180° to face the viewer. The screen is 2:1, matching the target.
let monitor = () =>
  Scene.group([
    Scene.cube() |> Scene.scaleXYZ(4.6, 2.5, 0.4) |> Scene.lit(Color.rgb(0.08, 0.08, 0.09)),
    Scene.quad()
      |> Scene.screen(board)
      |> Scene.rotateY(Angle.degrees(180.0))
      |> Scene.scaleXYZ(4.2, 2.1, 1.0)
      |> Scene.translate(Vec3.make(0.0, 0.0, -0.25)),
  ])
    |> Scene.translate(Vec3.make(0.0, 2.6, 2.0))

let lights = () => [
  Light.ambient(Color.rgb(0.15, 0.15, 0.18)),
  Light.directional(Vec3.make(0.4, -1.0, 0.3), Color.rgb(1.0, 0.97, 0.9), 0.9) |> Light.castShadows,
]

let init = { score: 0.0 }

// The home side scores every 4 seconds of match time — the screen is a pure
// function of the model, so it rewinds with the scrubber like everything else.
let tick = (m, dt, tts: float) => { score: Math.floor(tts / 4.0) }

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 3.0, -8.5), Vec3.make(0.0, 1.8, 0.0)),
    Scene.group([arena(tts), monitor()]),
    lights())
  |> Frame.withUiTarget(board, scoreboard(m.score, tts))

expect (
  let frame = draw(init, 0.0) in
  Frame.equals(frame, draw(init, 0.0))
)
