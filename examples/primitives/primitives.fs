module Primitives

open Functor
open Functor.Math

// An asset-free golden scene: primitives + lights only, no glTF (nothing to
// fetch), a fixed camera, and all animation driven from `frameTime.tts` (the
// pinned total time) so it renders deterministically under `--fixed-time`. This
// is the headless-CI render-regression guard — it exercises the lit pipeline
// (diffuse + specular), a directional shadow, and colored point lights without
// any external asset dependency.

type Model = unit

type Msg =
    | Noop

let game: Game<Model, Msg> = GameBuilder.local ()

// The camera is fixed and the scene is a pure function of frame time, so there
// is no per-frame state to evolve — every callback is a no-op.
let update model _msg = (model, Effect.none())
let input model _event = (model, Effect.none())
let tick model _frameTime = (model, Effect.none())

open Fable.Core.Rust
open Graphics.Scene3D

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun _world frameTime ->

        // Near-white lit surfaces so the colored point lights tint them; a
        // neutral grey ground.
        let litWhite = Material.lit(0.9f, 0.9f, 0.9f, 1.0f)
        let litGround = Material.lit(0.6f, 0.6f, 0.62f, 1.0f)

        // Two colored point lights orbiting the scene, animated by total time,
        // each with an emissive marker sphere at its position.
        let pointColors = [|
            Color.rgb 1.0f 0.3f 0.25f   // red
            Color.rgb 0.35f 0.5f 1.0f   // blue
        |]
        let orbitRadius = 3.2f
        let tau = 6.2831855f
        let spin = frameTime.tts * 0.6f
        let pointPos i =
            let a = spin + float32 i * (tau / 2.0f)
            Vector3.xyz (cos a * orbitRadius) 2.2f (sin a * orbitRadius)

        // Ground + three primitives resting on it (centered at half-height).
        let objects =
            group([|
                material (litGround, [| plane() |> Transform.scale 24.0f |])
                material (litWhite, [|
                    sphere()   |> Transform.translateX -2.2f |> Transform.translateY 0.8f |> Transform.scale 0.8f
                    cube()     |> Transform.translateX 2.2f  |> Transform.translateY 0.5f
                    cylinder() |> Transform.translateZ 2.2f  |> Transform.translateY 0.5f
                |])
            |])

        let markers =
            pointColors
            |> Array.mapi (fun i c ->
                let p = pointPos i
                material (Material.emissive(c.r, c.g, c.b, 1.0f), [|
                    sphere() |> Transform.translateX p.x |> Transform.translateY p.y |> Transform.translateZ p.z |> Transform.scale 0.15f
                |]))

        let scene = group(Array.append [| objects |] markers)

        // Fixed camera: above and behind, looking forward and slightly down.
        let camera =
            Graphics.Camera.firstPerson
                (Vector3.xyz 0.0f 3.5f -8.0f)
                (Math.Angle.radians 0.0f)
                (Math.Angle.radians -0.3f)
                (Math.Angle.degrees 60.0f)

        let pointLights =
            pointColors
            |> Array.mapi (fun i c ->
                Light.point({ Position = pointPos i; Color = c; Intensity = 1.4f; Range = 4.0f }))

        let lights =
            Array.append
                [|
                    Light.ambient(Color.rgb 0.1f 0.1f 0.13f)
                    // The shadow-casting "sun": objects rake shadows across the ground.
                    Light.directional({ Direction = Vector3.xyz 0.5f -1.0f 0.35f; Color = Color.rgb 1.0f 0.98f 0.95f; Intensity = 0.85f })
                    |> Light.castShadows
                |]
                pointLights

        Graphics.Frame.createLit camera scene lights
    )
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    |> Runtime.runGame
