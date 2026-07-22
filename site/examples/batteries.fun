// batteries.fun — batteries included. A rigged character streamed straight from
// a CDN and animated with a blend of its idle/walk/run clips. There is no asset
// pipeline to wire up: Asset.model takes the URL, Anim.clip names a clip, and
// the runtime handles loading, decoding, skinning, and blending — the character
// auto-cycles idle → walk → run and back.

let hero = Asset.model("https://cdn.jsdelivr.net/gh/BabylonJS/Assets@master/meshes/Xbot.glb")

let init = { t: 0.0 }

let tick = (model, dt: float, tts: float) => { model with t: model.t + dt }

let absF = (x: float): float => if x < 0.0 then 0.0 - x else x

// A 1D blend: each clip's weight peaks at its point on the speed axis (idle at
// 0, walk at 0.5, run at 1) and fades to its neighbours; Anim.blend normalises.
let idleWeight = (s: float): float => Math.clamp01(1.0 - s * 2.0)
let walkWeight = (s: float): float => Math.clamp01(1.0 - absF(s - 0.5) * 2.0)
let runWeight = (s: float): float => Math.clamp01(s * 2.0 - 1.0)

let pose = (tts: float): Anim.t =>
  let s = (1.0 - Math.cos(tts * 0.5)) * 0.5 in
  Anim.blend([
    (Anim.clip("idle", tts), idleWeight(s)),
    (Anim.clip("walk", tts), walkWeight(s)),
    (Anim.clip("run", tts), runWeight(s)),
  ])

let draw = (model, tts: float) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 1.4, -3.4), Vec3.make(0.0, 0.9, 0.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(10.0) |> Scene.lit(Color.rgb(0.4, 0.45, 0.55)),
      Scene.model(hero)
        |> Scene.animate(pose(tts))
        |> Scene.rotateY(Angle.degrees(180.0)),
    ]),
    [
      Light.ambient(Color.rgb(0.28, 0.28, 0.34)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 0.96, 0.88), 1.0),
    ])
