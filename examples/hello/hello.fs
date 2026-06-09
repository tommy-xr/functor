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
    // Player-controlled offset applied to the rendered scene, driven by input.
    offset: Point2
    held: HeldKeys
}

module Model =
    let initial = {
        paddle1 = Paddle.initial
        paddle2 = Paddle.initial
        ball = Ball.initial
        counter = 0
        offset = Point2.zero
        held = HeldKeys.none
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

let input model (event: Input.t) =
    match event with
    | Input.Keyboard (Input.KeyboardEvent.KeyDown key) ->
        ({ model with held = setHeld model.held key true }, Effect.none())
    | Input.Keyboard (Input.KeyboardEvent.KeyUp key) ->
        ({ model with held = setHeld model.held key false }, Effect.none())
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

    // Integrate the player offset from the currently held keys. Scaling by
    // tick.dts keeps the speed frame-rate-independent.
    let speed = 2.0f
    let axis neg pos = (if pos then 1.0f else 0.0f) - (if neg then 1.0f else 0.0f)
    let velocity =
        Vector2.xy
            (axis model.held.left model.held.right)
            (axis model.held.down model.held.up)
        |> Vector2.scale speed
    let newOffset = model.offset |> Point2.add (Vector2.scale tick.dts velocity)

    ( { model with ball = newBall; counter = model.counter + 3; offset = newOffset }, Effect.wrapped (MovePaddle2) |> Effect.map(fun _a -> MovePaddle1)  )

open Fable.Core.Rust

open Graphics.Scene3D;

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun world frameTime -> 
        
        let eff = Effect.wrapped (MovePaddle2) |> Effect.map (fun _a -> MovePaddle1);
        let colorMaterial = Material.color(0.0f, 1.0f, 0.0f, 1.0f);
        let textureMaterial = Material.texture( Texture.file("crate.png"));
        // let barrelModel = Model.file("ExplodingBarrel.glb");
        // let renderModel = (Graphics.Scene3D.model barrelModel) |> Transform.scale 1f;
        // let renderModel = Model.file ("ExplodingBarrel.glb") |> Graphics.Scene3D.model |> Transform.scale 0.5f;
        // let modify = Model.modify (MeshSelector.all ()) (MeshOverride.material (textureMaterial));
        // let renderModel = Model.file ("vr_glove_model2.glb") |> modify |> Graphics.Scene3D.model |> Transform.scale 5f;

        let renderModel = 
            "shark.glb"
            |> Model.file 
            |> Graphics.Scene3D.model 
            |> Transform.translateZ 10.0f
            |> Transform.scale 0.004f;

        group([|
            material (textureMaterial, [|
                cylinder() |> Transform.translateY -1.0f;
                renderModel
                |> Transform.rotateY (Math.Angle.degrees (180.0f + 10.0f * frameTime.tts * 0.5f))
                |> Transform.rotateX (Math.Angle.degrees (0.0f * sin frameTime.tts * 2.0f))
            |])
            |> Transform.translateZ ((sin (frameTime.tts * 5.0f)) * 1.0f)
        |])
        // Apply the input-driven offset: WASD / arrow keys move the scene.
        |> Transform.translateX world.offset.x
        |> Transform.translateY world.offset.y
    )
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    |> GameBuilder.init (Effect.wrapped MovePaddle1)
    |> GameBuilder.subscriptions subscriptions
    |> Runtime.runGame