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

type Model = {
    paddle1: Paddle
    paddle2: Paddle
    ball: Ball
    counter: int
}

module Model =
    let initial = {
        paddle1 = Paddle.initial
        paddle2 = Paddle.initial
        ball = Ball.initial
        counter = 0
    }

type Msg =
    | MovePaddle1
    | MovePaddle2

let game: Game<Model, Msg> = GameBuilder.local Model.initial

let update model msg =
    printfn "Running update"
    (model, Effect.none())

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

    ( { model with ball = newBall; counter = model.counter + 3 }, Effect.wrapped (MovePaddle2) |> Effect.map(fun _a -> MovePaddle1)  ) 

open Fable.Core.Rust

open Graphics.Scene3D;

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun world frameTime -> 
        
        let eff = Effect.wrapped (MovePaddle2) |> Effect.map (fun _a -> MovePaddle1);
        let colorMaterial = Material.color(0.0f, 1.0f, 0.0f, 1.0f);
        let textureMaterial = Material.texture( Texture.file("vr_glove_color.jpg"));
        // let barrelModel = Model.file("ExplodingBarrel.glb");
        // let renderModel = (Graphics.Scene3D.model barrelModel) |> Transform.scale 1f;
        // let renderModel = Model.file ("ExplodingBarrel.glb") |> Graphics.Scene3D.model |> Transform.scale 0.5f;
        // let modify = Model.modify (MeshSelector.all ()) (MeshOverride.material (textureMaterial));
        // let renderModel = Model.file ("vr_glove_model2.glb") |> Graphics.Scene3D.model |> Transform.scale 5f;

        let renderModel = 
            "fish.glb"
            |> Model.file 
            |> Graphics.Scene3D.model 
            |> Transform.translateY -5.0f
            |> Transform.translateZ 10.0f
            |> Transform.scale 0.004f;

        let sharkModel = 
            "shark.glb"
            |> Model.file 
            |> Graphics.Scene3D.model 
            |> Transform.translateZ 10.0f
            |> Transform.scale 0.004f;

        group([|
            material (textureMaterial, [|
                cylinder() |> Transform.translateY -1.0f;
                sharkModel;
                renderModel
                |> Transform.rotateY (Math.Angle.degrees (90.0f + 10.0f * frameTime.tts * 0.5f))
                |> Transform.rotateX (Math.Angle.degrees (0.0f * sin frameTime.tts * 2.0f))
            |])
            |> Transform.translateZ ((sin (frameTime.tts * 5.0f)) * 1.0f)
        |])
    )
    |> GameBuilder.update update
    |> GameBuilder.tick tick
    |> Runtime.runGame