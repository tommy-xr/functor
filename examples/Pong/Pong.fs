module Pong

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
    | MovePaddle1 of float
    | MovePaddle2 of float

let game: Game<Model, Msg> = GameBuilder.local Model.initial

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

    ( { model with ball = newBall; counter = model.counter + 3 }, Effect.none ) 

open Fable.Core.Rust

open Graphics.Scene3D;

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun model frameTime -> 
        
        let colorMaterial = Material.color(1.0f, 0.0f, 1.0f, 1.0f);
        let textureMaterial = Material.texture( Texture.file("crate.png"));

        material (textureMaterial, [|
            cylinder() |> Transform.translateY -1.0f;
            cube()
            |> Transform.rotateZ (Math.Angle.degrees (frameTime.tts * 40.0f))
        |])
        |> Transform.translateZ ((sin (frameTime.tts * 5.0f)) * 1.0f)

    )
    |> GameBuilder.tick tick
    |> Runtime.runGame