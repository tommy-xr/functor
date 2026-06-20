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
    // Delivered when the spacebar gunshot finishes playing (Audio.playThen).
    | GunshotDone

/// World position of the fountain — shared by its visual (draw3d) and its
/// positioned audio loop (soundScape) so the sound always comes from the object.
let fountainPos = Vector3.xyz 5.0f 0.5f 0.0f

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
    // Spacebar fires a one-shot sound, with a message delivered when it ends.
    | Input.Keyboard (Input.KeyboardEvent.KeyDown Input.Space) ->
        (model, Audio.playThen "gunshot.wav" GunshotDone)
    // 'B' fires a spatialized gunshot at a fixed point off to the right, so it
    // pans and attenuates as you look around / move (Audio.playAt).
    | Input.Keyboard (Input.KeyboardEvent.KeyDown Input.B) ->
        (model, Audio.playAt "gunshot.wav" (Vector3.xyz 5.0f 1.0f 0.0f))
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
        // Normal-mapped: a tangent-space bumps map perturbs the surface normal,
        // so the lights and specular highlights play across the bumps.
        let litBumps =
            Material.litNormalMapped(0.9f, 0.9f, 0.92f, 1.0f, Texture.file("bumps-normal.png"))

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
                    cylinder() |> Transform.translateZ 2.5f  |> Transform.translateY 0.5f
                |])
                // The cube is normal-mapped, so its flat faces show the bumps.
                material (litBumps, [|
                    cube() |> Transform.translateX 2.5f |> Transform.translateY 0.5f
                |])
            |])

        let markers =
            pointColors
            |> Array.mapi (fun i c ->
                let p = pointPos i
                material (Material.emissive(c.r, c.g, c.b, 1.0f), [|
                    sphere() |> Transform.translateX p.x |> Transform.translateY p.y |> Transform.translateZ p.z |> Transform.scale 0.12f
                |]))

        // An animated (skinned) shark swimming above the ground — it casts a
        // deforming shadow as it swims (skinned shadow caster).
        let shark =
            Model.file "shark.glb" |> Graphics.Scene3D.model
            |> Transform.translateX 1.5f |> Transform.translateY 1.8f |> Transform.translateZ 0.5f
            |> Transform.rotateY (Math.Angle.degrees 90.0f)
            |> Transform.scale 0.003f

        // The fountain: a stone basin with a glowing pool of water, sitting at
        // `fountainPos` — the source of the positioned water-loop you hear pan as
        // you walk around it. The water bobs gently so it reads as alive.
        // Transforms compose right-to-left (the last in the pipe is applied to the
        // geometry first), so translate first, then scale — otherwise the scale
        // would shrink the translation offset too.
        let bob = 0.05f * sin (frameTime.tts * 2.0f)
        let fountain =
            group([|
                // Wide, short basin resting on the ground.
                material (litWhite, [|
                    cylinder() |> Transform.translateY 0.25f
                               |> Transform.scaleY 0.5f |> Transform.scaleX 1.4f |> Transform.scaleZ 1.4f
                |])
                // Emissive "water" pooled in the basin (self-lit so it glows).
                material (Material.emissive(0.3f, 0.65f, 1.0f, 1.0f), [|
                    sphere() |> Transform.translateY (0.6f + bob)
                             |> Transform.scaleY 0.4f |> Transform.scale 0.55f
                |])
            |])
            |> Transform.translateX fountainPos.x |> Transform.translateZ fountainPos.z

        let scene = group(Array.append [| objects; shark; fountain |] markers)

        let camera =
            Graphics.Camera.firstPerson
                world.eye
                (Math.Angle.radians world.yaw)
                (Math.Angle.radians world.pitch)
                (Math.Angle.degrees 60.0f)

        // A "searchlight" spot high above, slowly sweeping across the objects —
        // this is the shadow caster, so the object shadows rake across the ground
        // and track as it sweeps.
        // High and behind the objects, angled forward so shadows rake toward the
        // camera (and aren't hidden directly under each object).
        let spotPos = Vector3.xyz 0.0f 7.0f 5.0f
        let sweepX = sin (frameTime.tts * 0.5f) * 3.0f
        let spotDir = Vector3.xyz (sweepX - spotPos.x) (0.3f - spotPos.y) (0.0f - spotPos.z)

        let pointLights =
            pointColors
            |> Array.mapi (fun i c ->
                Light.point({ Position = pointPos i; Color = c; Intensity = 1.4f; Range = 4.0f }))

        let lights =
            Array.append
                [|
                    Light.ambient(Color.rgb 0.08f 0.08f 0.11f)
                    // A dim, non-casting fill so areas outside the spot aren't black.
                    Light.directional({ Direction = Vector3.xyz 0.4f -1.0f 0.3f; Color = Color.rgb 0.9f 0.92f 1.0f; Intensity = 0.25f })
                |]
                (Array.append pointLights [|
                    // The shadow-casting searchlight.
                    Light.spot({ Position = spotPos; Direction = spotDir; Color = Color.rgb 1.0f 1.0f 0.95f; Intensity = 5.0f; Range = 18.0f; ConeAngle = 0.5f })
                    |> Light.castShadows
                |])

        Graphics.Frame.createLit camera scene lights
    )
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    // The continuous soundscape: a non-spatial wind bed plus a positioned
    // fountain you can walk around (WASD) and hear pan/attenuate. Both loop and
    // are reconciled by key each frame — they keep playing as the camera moves.
    |> GameBuilder.soundScape (fun _world ->
        AudioScene.create [|
            AudioSource.ambient "wind" "wind-loop.wav" |> AudioSource.gain 0.35f
            AudioSource.at "fountain" "water-loop.wav" fountainPos
            |> AudioSource.gain 0.8f
        |])
    |> Runtime.runGame
