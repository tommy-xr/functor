// The Quest tool APK's boot scene: what the headset shows before any game is
// pushed over the network (POST /reload-source replaces it live, model
// preserved). The site's loader in 3D: the Functor mark, milled from nine
// slices, spins ~2.5m ahead at eye height above a glowing grid floor that
// fades into fog. The authored camera is the reference center-eye pose on
// the headset; live head/eye motion composes onto it, so desktop and VR
// start with the same framing.

let bg = Color.rgb(0.059, 0.047, 0.114)
// cyan → plum by t in [0, 1].
let tint = (t) => Color.rgb(0.255 + 0.102 * t, 0.847 - 0.706 * t, 0.902 - 0.463 * t)

// The mark's three polygons in its SVG space (y down, 690 units tall),
// centred and flipped so +y is up.
let svg = (pts) =>
  pts |> List.map((p) => { x: p.x - 558.5, y: 508.0 - p.y }) |> Sketch.polygon
let mark = Sketch.union([
  svg([{ x: 382.0, y: 341.0 }, { x: 545.0, y: 179.0 }, { x: 735.0, y: 179.0 },
       { x: 615.0, y: 300.0 }, { x: 548.0, y: 300.0 }, { x: 382.0, y: 466.0 }]),
  svg([{ x: 382.0, y: 493.0 }, { x: 497.0, y: 380.0 }, { x: 497.0, y: 748.0 },
       { x: 382.0, y: 837.0 }]),
  svg([{ x: 518.0, y: 426.0 }, { x: 693.0, y: 426.0 }, { x: 597.0, y: 522.0 },
       { x: 518.0, y: 522.0 }]),
])
let markSize = 0.55 / 690.0

// Solid.toScene turns mm Z-up into metres Y-up; undo both so a sketch stands
// facing -z at 1 unit per metre, extruded toward the viewer.
let upright = (solid) =>
  Solid.toScene(solid) |> Scene.rotateX(Angle.degrees(90.0)) |> Scene.scale(1000.0)

// Nine slices along z, cyan at the front fading to plum at the back, as on
// the site. One mesh, nine tints.
let slice = Solid.extrude(14.0, mark) |> upright
let slices = Scene.group(List.range(9.0) |> List.map((i) =>
  slice
    |> Scene.lit(tint(i * 0.12))
    |> Scene.translate(Vec3.make(0.0, 0.0, (4.0 - i) * 21.0 - 7.0))))

// 1 m grid lines, 30 m square, just above the floor.
let gridLines =
  (List.range(31.0) |> List.map((i) =>
     Instance.at(Vec3.make(-15.0 + i, 0.005, -5.0)) |> Instance.scaleXYZ(0.012, 0.01, 30.0)))
  |> List.append(List.range(31.0) |> List.map((i) =>
     Instance.at(Vec3.make(0.0, 0.005, -20.0 + i)) |> Instance.scaleXYZ(30.0, 0.01, 0.012)))

let floor = Scene.group([
  Scene.plane() |> Scene.scale(40.0) |> Scene.lit(Color.rgb(0.07, 0.055, 0.13)),
  Scene.cube() |> Scene.emissive(Color.rgb(0.2, 0.42, 0.5)) |> Scene.instanced(gridLines),
])

let init = {}

let tick = (model, dt, tts) => model

let draw = (model, tts) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(0.0, 1.6, 0.5), Vec3.make(0.0, 1.35, -2.5)),
    Scene.group([
      floor,
      slices
        |> Scene.scale(markSize)
        |> Scene.rotateY(Angle.radians(tts * 6.2832 / 3.6))
        |> Scene.rotateX(Angle.degrees(-11.0))
        |> Scene.translate(Vec3.make(0.0, 1.4 + 0.02 * Math.sin(tts * 6.2832 / 3.4), -2.5)),
    ]),
    [
      Light.ambient(Color.rgb(0.25, 0.22, 0.32)),
      Light.directional(Vec3.make(0.3, -0.6, -1.0), Color.rgb(1.0, 0.96, 0.9), 0.9),
    ])
    |> Frame.withClearColor(bg)
    |> Frame.withFog(Fog.linear(3.0, 16.0, bg))
