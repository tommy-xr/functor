// examples/monitor — render targets (the security-camera demo).
//
// A panning camera gadget films the courtyard; its view renders into the
// "security" render target each frame (Frame.withRenderTarget), and the
// monitor's screen shows it live (Scene.screen). The feed is a full frame of
// its own — camera, scene, lights — so it is lit and shadowed like the main
// view. All animation derives from tts, so captures are deterministic under
// --fixed-time.

// The branded target: declared ONCE, used at the writer (draw's
// Frame.withRenderTarget) and the reader (the monitor's Scene.screen).
let feed = RenderTarget.named("security") |> RenderTarget.sized(256.0, 256.0)

// The security camera's pan angle (radians), shared by the visible gadget
// and the feed camera so the picture matches the prop.
// Amplitude stays inside the feed's ~±22° half-fov, so the courtyard never
// fully leaves the picture.
let pan = (tts: float) => Math.sin(tts * 0.5) * 0.5

// An orbiting glow so the feed visibly animates.
let orbiter = (tts: float) =>
  Scene.sphere()
    |> Scene.scale(0.45)
    |> Scene.emissive(Color.rgb(1.0, 0.55, 0.15))
    |> Scene.translate(Math.cos(tts * 0.8) * 3.0, 0.6, 3.0 + Math.sin(tts * 0.8) * 2.0)

// The courtyard both cameras film.
let courtyard = (tts: float) =>
  Scene.group([
    Scene.plane() |> Scene.scale(22.0) |> Scene.lit(Color.rgb(0.55, 0.6, 0.55)),
    Scene.cube() |> Scene.lit(Color.rgb(0.85, 0.3, 0.25)) |> Scene.translate(-2.4, 0.5, 2.0),
    Scene.cube()
      |> Scene.scale(0.7)
      |> Scene.rotateY(Angle.degrees(30.0))
      |> Scene.lit(Color.rgb(0.3, 0.5, 0.9))
      |> Scene.translate(2.2, 0.35, 3.4),
    Scene.cylinder() |> Scene.lit(Color.rgb(0.9, 0.8, 0.3)) |> Scene.translate(0.5, 0.5, 5.2),
    orbiter(tts),
  ])

let lights = () => [
  Light.ambient(Color.rgb(0.12, 0.12, 0.15)),
  Light.directional(0.4, -1.0, 0.3, Color.rgb(1.0, 0.97, 0.9), 0.9) |> Light.castShadows,
]

// The visible camera prop: a head with a lens (a cylinder's axis is Y;
// rotateX aims it along +Z), yawed by the SAME pan driving the feed camera.
let cameraGadget = (tts: float) =>
  Scene.group([
    Scene.cube() |> Scene.scale(0.45) |> Scene.lit(Color.rgb(0.25, 0.25, 0.28)),
    Scene.cylinder()
      |> Scene.scale(0.2)
      |> Scene.rotateX(Angle.degrees(90.0))
      |> Scene.lit(Color.rgb(0.1, 0.1, 0.1))
      |> Scene.translate(0.0, 0.0, 0.35),
  ])
    |> Scene.rotateY(Angle.radians(pan(tts)))
    |> Scene.scale(0.8)
    |> Scene.translate(0.0, 3.2, -5.0)

// What the security camera sees: its own full frame — the courtyard from the
// gadget's mount, looking where the gadget points. (The feed deliberately
// films only the courtyard: a target's scene is independent of the main one.)
let feedFrame = (tts: float) =>
  Frame.createLit(
    Camera.lookAt(
      0.0, 3.2, -5.0,
      Math.sin(pan(tts)) * 8.0, 0.5, -5.0 + Math.cos(pan(tts)) * 8.0),
    courtyard(tts),
    lights())

// The monitor: a dark bezel block with the screen on its camera-facing side.
// A quad's front is +Z and the main camera looks down +Z, so the screen is
// rotated 180° to face the viewer — an unrotated quad would show its back,
// a mirrored feed.
let monitor = () =>
  Scene.group([
    Scene.cube() |> Scene.scale(2.4) |> Scene.lit(Color.rgb(0.08, 0.08, 0.09)),
    Scene.quad()
      |> Scene.screen(feed)
      |> Scene.rotateY(Angle.degrees(180.0))
      |> Scene.scale(2.0)
      |> Scene.translate(0.0, 0.0, -1.25),
  ])
    |> Scene.rotateY(Angle.degrees(25.0))
    |> Scene.translate(3.1, 1.5, -0.6)

let init = {}
let tick = (m, dt, tts) => m

let draw = (m, tts: float) =>
  Frame.createLit(
    Camera.lookAt(0.0, 2.6, -9.5, 0.0, 1.2, 0.0),
    Scene.group([courtyard(tts), cameraGadget(tts), monitor()]),
    lights())
  |> Frame.withRenderTarget(feed, feedFrame(tts))
