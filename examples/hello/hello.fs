module Hello

open Functor
open Functor.Math

let GAME_HEIGHT = 600.0f
let GAME_WIDTH = 800.0f

type Paddle = { 
    position: Point2
    size: Vector2
}

module Paddle =
    let initial = { position = Point2.zero; size = Vector2.xy 0.1f 0.3f }

type Ball = { 
    position: Point2
    velocity: Vector2
    radius: float32 }

module Ball = 
    let initial = { position = Point2.zero; velocity = Vector2.zero; radius = 0.05f }

/// Which movement keys are currently held. Reconstructed from KeyDown/KeyUp
/// events in `input` so `tick` can apply smooth, frame-rate-independent
/// movement. (A future input *snapshot* will let the runtime hand this to the
/// game directly — see todo.md.)
type HeldKeys = {
    up: bool
    down: bool
    left: bool
    right: bool
}

module HeldKeys =
    let none = { up = false; down = false; left = false; right = false }

type Model = {
    paddle1: Paddle
    paddle2: Paddle
    ball: Ball
    counter: int
    // First-person camera state, driven by input: WASD moves the eye, the
    // mouse turns it (yaw/pitch). lastMouse holds the previous cursor position
    // so we can turn by the per-frame delta (None until the first event).
    held: HeldKeys
    eye: Vector3
    yaw: float32
    pitch: float32
    lastMouse: (float32 * float32) option
}

module Model =
    let initial = {
        paddle1 = Paddle.initial
        paddle2 = Paddle.initial
        ball = Ball.initial
        counter = 0
        held = HeldKeys.none
        eye = Vector3.xyz 0.0f 0.0f -5.0f
        yaw = 0.0f
        pitch = 0.0f
        lastMouse = None
    }

type Msg =
    | MovePaddle1
    | MovePaddle2
    | Tick

let game: Game<Model, Msg> = GameBuilder.local Model.initial

let update model msg =
    match msg with
    | Tick ->
        // Driven by the `Sub.every` subscription below: a message produced by a
        // subscription arrives here every second, just like any other message.
        let newModel = { model with counter = model.counter + 1 }
        Debug.log (sprintf "Tick! counter = %d" newModel.counter)
        (newModel, Effect.none())
    | _ ->
        printfn "Running update"
        (model, Effect.none())

let subscriptions model =
    Sub.every (Duration.fromSeconds 1.0) Tick

// Map WASD and the arrow keys onto the held-key flags. Both KeyDown and KeyUp
// flow through here; we just record whether each direction is currently held.
let private setHeld (held: HeldKeys) (key: Input.Key) (isDown: bool) =
    match key with
    | Input.W | Input.Up -> { held with up = isDown }
    | Input.S | Input.Down -> { held with down = isDown }
    | Input.A | Input.Left -> { held with left = isDown }
    | Input.D | Input.Right -> { held with right = isDown }
    | _ -> held

// Mouse sensitivity, radians of rotation per pixel of motion.
let private mouseSensitivity = 0.003f
// Clamp pitch just short of straight up/down (~85 degrees) to avoid flipping.
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
            // First sample: just record the position, don't jump the view.
            ({ model with lastMouse = Some (mx, my) }, Effect.none())
        | Some (lastX, lastY) ->
            let dx = mx - lastX
            let dy = my - lastY
            // Mouse right turns the view right; mouse up looks up.
            let newYaw = model.yaw - dx * mouseSensitivity
            let newPitch =
                model.pitch - dy * mouseSensitivity
                |> min pitchLimit
                |> max -pitchLimit
            ({ model with yaw = newYaw; pitch = newPitch; lastMouse = Some (mx, my) }, Effect.none())
    | Input.Mouse _ -> (model, Effect.none())

let tick model (tick: Time.FrameTime) =
    
    let applyVelocity (tick: Time.FrameTime) ball = 
        let newBallPosition = (ball.position
        |> Point2.add (Vector2.scale tick.dts ball.velocity));
        { ball with position = newBallPosition }

    let handleCollisionWithTopAndBottomWalls ball =
        if ball.position.y <= 0.0f || ball.position.y >= GAME_HEIGHT then 
            { ball with velocity = Vector2.xy ball.velocity.x -ball.velocity.y }
        else ball

    let handleCollisionWithPaddle (paddle: Paddle) (ball: Ball) = 
        let ballTop = ball.position.y - ball.radius
        let ballBottom = ball.position.y + ball.radius
        let ballLeft = ball.position.x - ball.radius
        let ballRight = ball.position.x + ball.radius
        let paddleTop = paddle.position.y - paddle.size.y / 2.0f
        let paddleBottom = paddle.position.y + paddle.size.y / 2.0f
        let paddleLeft = paddle.position.x - paddle.size.x / 2.0f
        let paddleRight = paddle.position.x + paddle.size.x / 2.0f
        if ballTop >= paddleBottom && ballBottom <= paddleTop && ballLeft <= paddleRight && ballRight >= paddleLeft then
            { ball with velocity = Vector2.xy -ball.velocity.x ball.velocity.y }
        else ball

    let newBall = (model.ball
        |> applyVelocity tick
        |> handleCollisionWithTopAndBottomWalls
        |> handleCollisionWithPaddle model.paddle1
        |> handleCollisionWithPaddle model.paddle2)

    // Move the eye from the held keys, relative to where we're looking.
    // Scaling by tick.dts keeps the speed frame-rate-independent.
    let speed = 3.0f
    let axis neg pos = (if pos then 1.0f else 0.0f) - (if neg then 1.0f else 0.0f)
    // Forward/right in the ground plane from the current yaw (yaw = 0 -> +Z).
    let forward = Vector3.xyz (sin model.yaw) 0.0f (cos model.yaw)
    let right = Vector3.xyz -(cos model.yaw) 0.0f (sin model.yaw)
    let move =
        Vector3.add
            (Vector3.scale (axis model.held.down model.held.up) forward)
            (Vector3.scale (axis model.held.left model.held.right) right)
    let newEye = model.eye |> Vector3.add (Vector3.scale (speed * tick.dts) move)

    ( { model with ball = newBall; counter = model.counter + 3; eye = newEye }, Effect.wrapped (MovePaddle2) |> Effect.map(fun _a -> MovePaddle1)  )

open Fable.Core.Rust

open Graphics.Scene3D;

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun world frameTime -> 
        
        let eff = Effect.wrapped (MovePaddle2) |> Effect.map (fun _a -> MovePaddle1);
        let colorMaterial = Material.color(0.0f, 1.0f, 0.0f, 1.0f);
        let textureMaterial = Material.texture( Texture.file("crate.png"));
        let gridMaterial = Material.texture( Texture.file("grid.png"));
        // let barrelModel = Model.file("ExplodingBarrel.glb");
        // let renderModel = (Graphics.Scene3D.model barrelModel) |> Transform.scale 1f;
        // let renderModel = Model.file ("ExplodingBarrel.glb") |> Graphics.Scene3D.model |> Transform.scale 0.5f;
        // let modify = Model.modify (MeshSelector.all ()) (MeshOverride.material (textureMaterial));
        // let renderModel = Model.file ("vr_glove_model2.glb") |> modify |> Graphics.Scene3D.model |> Transform.scale 5f;

        // A lineup of glTF samples from BabylonJS/Assets exercising the model
        // pipeline. Skinned + animated: shark, fish, Xbot. Non-skinned:
        // ExplodingBarrel. Raw model units vary wildly (the barrel is ~72 units
        // tall, Xbot is Mixamo-style cm scale), hence the per-model scales.
        let sample (path: string) = path |> Model.file |> Graphics.Scene3D.model

        let scene =
            group([|
                // Synthwave terrain: a grid-textured heightmap (XZ, Y-up) with
                // gentle static ripples, beneath the lineup.
                material (gridMaterial, [|
                    heightmapFn 32 32 (fun r c ->
                        0.05f * (sin (float32 c * 0.5f) + cos (float32 r * 0.5f)))
                    |> Transform.translateY -2.5f |> Transform.translateZ 4.0f |> Transform.scale 30.0f;
                |]);
                // A row of primitives in front of the lineup — the clearest
                // subjects for the `--debug-render normals` view (a sphere reads
                // as a smooth RGB gradient; a cube as six flat face colors).
                material (textureMaterial, [|
                    cylinder() |> Transform.translateY -2.5f;
                    sphere() |> Transform.translateX -2.0f |> Transform.translateY 1.0f |> Transform.translateZ 2.0f |> Transform.scale 0.7f;
                    cube() |> Transform.translateX 2.0f |> Transform.translateY 1.0f |> Transform.translateZ 2.0f;
                |]);
                sample "shark.glb"
                |> Transform.translateX 3.0f |> Transform.translateY 1.0f |> Transform.translateZ 3.0f
                |> Transform.rotateY (Math.Angle.degrees 180.0f)
                |> Transform.scale 0.002f;
                sample "fish.glb"
                |> Transform.translateX -3.0f |> Transform.translateY 1.0f |> Transform.translateZ 3.0f
                |> Transform.scale 0.002f;
                sample "Xbot.glb"
                |> Transform.translateX 1.5f |> Transform.translateY -1.0f |> Transform.translateZ 3.0f
                |> Transform.scale 0.015f;
                sample "ExplodingBarrel.glb"
                |> Transform.translateY -1.5f |> Transform.translateZ 3.0f
                |> Transform.scale 0.02f;
            |])

        // First-person camera: WASD moves world.eye, the mouse turns yaw/pitch.
        let camera =
            Graphics.Camera.firstPerson
                world.eye
                (Math.Angle.radians world.yaw)
                (Math.Angle.radians world.pitch)
                (Math.Angle.degrees 60.0f)

        Graphics.Frame.create camera scene
    )
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    |> GameBuilder.init (Effect.wrapped MovePaddle1)
    |> GameBuilder.subscriptions subscriptions
    |> Runtime.runGame