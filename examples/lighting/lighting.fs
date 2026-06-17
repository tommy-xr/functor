module Lighting

open Functor
open Functor.Math

/// Which movement keys are currently held; reconstructed from KeyDown/KeyUp in
/// `input` so `tick` can move the camera smoothly (see hello.fs for the same
/// pattern and the future input-snapshot note).
type HeldKeys = {
    up: bool
    down: bool
    left: bool
    right: bool
}

module HeldKeys =
    let none = { up = false; down = false; left = false; right = false }

type Model = {
    // First-person camera: WASD moves the eye, the mouse turns yaw/pitch.
    held: HeldKeys
    eye: Vector3
    yaw: float32
    pitch: float32
    lastMouse: (float32 * float32) option
}

module Model =
    let initial = {
        held = HeldKeys.none
        // Above and behind the scene, looking forward and slightly down.
        eye = Vector3.xyz 0.0f 3.0f -8.0f
        yaw = 0.0f
        pitch = -0.25f
        lastMouse = None
    }

type Msg =
    | Noop

let game: Game<Model, Msg> = GameBuilder.local Model.initial

let update model _msg = (model, Effect.none())

let private setHeld (held: HeldKeys) (key: Input.Key) (isDown: bool) =
    match key with
    | Input.W | Input.Up -> { held with up = isDown }
    | Input.S | Input.Down -> { held with down = isDown }
    | Input.A | Input.Left -> { held with left = isDown }
    | Input.D | Input.Right -> { held with right = isDown }
    | _ -> held

let private mouseSensitivity = 0.003f
let private pitchLimit = 1.5f

let input model (event: Input.t) =
    match event with
    | Input.Keyboard (Input.KeyboardEvent.KeyDown key) ->
        ({ model with held = setHeld model.held key true }, Effect.none())
    | Input.Keyboard (Input.KeyboardEvent.KeyUp key) ->
        ({ model with held = setHeld model.held key false }, Effect.none())
    | Input.Mouse (Input.MouseEvent.MouseMove (x, y)) ->
        let mx = float32 x
        let my = float32 y
        match model.lastMouse with
        | None ->
            ({ model with lastMouse = Some (mx, my) }, Effect.none())
        | Some (lastX, lastY) ->
            let dx = mx - lastX
            let dy = my - lastY
            let newYaw = model.yaw - dx * mouseSensitivity
            let newPitch =
                model.pitch - dy * mouseSensitivity
                |> min pitchLimit
                |> max -pitchLimit
            ({ model with yaw = newYaw; pitch = newPitch; lastMouse = Some (mx, my) }, Effect.none())
    | Input.Mouse _ -> (model, Effect.none())

let tick model (tick: Time.FrameTime) =
    // Move the eye from the held keys, relative to where we're looking.
    let speed = 4.0f
    let axis neg pos = (if pos then 1.0f else 0.0f) - (if neg then 1.0f else 0.0f)
    let forward = Vector3.xyz (sin model.yaw) 0.0f (cos model.yaw)
    let right = Vector3.xyz -(cos model.yaw) 0.0f (sin model.yaw)
    let move =
        Vector3.add
            (Vector3.scale (axis model.held.down model.held.up) forward)
            (Vector3.scale (axis model.held.left model.held.right) right)
    let newEye = model.eye |> Vector3.add (Vector3.scale (speed * tick.dts) move)
    ({ model with eye = newEye }, Effect.none())

open Fable.Core.Rust
open Graphics.Scene3D

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun world frameTime ->

        // Lit (diffuse) materials — shaded by the frame's lights. Near-white so
        // the colored lights tint them; the ground is a neutral grey.
        let litWhite = Material.lit(0.9f, 0.9f, 0.9f, 1.0f)
        let litGround = Material.lit(0.6f, 0.6f, 0.62f, 1.0f)

        // Three colored point lights orbiting the scene (120 degrees apart),
        // animated by total time. Each has an emissive marker sphere at its
        // position so you can see where the light is.
        let pointColors = [|
            Color.rgb 1.0f 0.25f 0.2f   // red
            Color.rgb 0.3f 1.0f 0.35f   // green
            Color.rgb 0.4f 0.5f 1.0f    // blue
        |]
        let orbitRadius = 3.5f
        let orbitHeight = 2.6f
        let spin = frameTime.tts * 0.6f
        let tau = 6.2831855f
        let pointPos i =
            let a = spin + float32 i * (tau / 3.0f)
            Vector3.xyz (cos a * orbitRadius) orbitHeight (sin a * orbitRadius)

        // Ground + a few objects sitting on it (each centered at y = its
        // half-height so it rests on y = 0).
        let objects =
            group([|
                material (litGround, [| plane() |> Transform.scale 24.0f |])
                material (litWhite, [|
                    sphere()   |> Transform.translateX -2.5f |> Transform.translateY 0.8f |> Transform.scale 0.8f
                    cube()     |> Transform.translateX 2.5f  |> Transform.translateY 0.5f
                    cylinder() |> Transform.translateZ 2.5f  |> Transform.translateY 0.5f
                |])
            |])

        let markers =
            pointColors
            |> Array.mapi (fun i c ->
                let p = pointPos i
                material (Material.emissive(c.r, c.g, c.b, 1.0f), [|
                    sphere() |> Transform.translateX p.x |> Transform.translateY p.y |> Transform.translateZ p.z |> Transform.scale 0.12f
                |]))

        let scene = group(Array.append [| objects |] markers)

        let camera =
            Graphics.Camera.firstPerson
                world.eye
                (Math.Angle.radians world.yaw)
                (Math.Angle.radians world.pitch)
                (Math.Angle.degrees 60.0f)

        // The full gamut: a dim cool ambient, a soft directional fill, the three
        // orbiting colored point lights, and a white spot angled down at the
        // cylinder.
        let pointLights =
            pointColors
            |> Array.mapi (fun i c ->
                Light.point({ Position = pointPos i; Color = c; Intensity = 4.0f; Range = 7.0f }))

        let lights =
            Array.append
                [|
                    Light.ambient(Color.rgb 0.06f 0.06f 0.09f)
                    Light.directional({ Direction = Vector3.xyz 0.2f -1.0f 0.25f; Color = Color.rgb 1.0f 0.97f 0.9f; Intensity = 0.3f })
                |]
                (Array.append pointLights [|
                    Light.spot({ Position = Vector3.xyz 0.0f 5.0f 2.5f; Direction = Vector3.xyz 0.0f -1.0f 0.0f; Color = Color.rgb 1.0f 1.0f 0.95f; Intensity = 6.0f; Range = 14.0f; ConeAngle = 0.45f })
                |])

        Graphics.Frame.createLit camera scene lights
    )
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    |> Runtime.runGame
