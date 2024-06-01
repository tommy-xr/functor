module Pong

open Functor
open Functor.Math

let GAME_HEIGHT = 600.0
let GAME_WIDTH = 800.0

type Paddle = { 
    position: Point2
    size: Vector2
}

module Paddle =
    let initial = { position = Point2.zero; size = Vector2.xy 0.1 0.3 }

type Ball = { 
    position: Point2
    velocity: Vector2
    radius: float }

module Ball = 
    let initial = { position = Point2.zero; velocity = Vector2.zero; radius = 0.05 }

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

let tick model (tick: Tick.t) =
    
    let applyVelocity (tick: Tick.t) ball = 
        let newBallPosition = (ball.position
        |> Point2.add (Vector2.scale tick.dts ball.velocity));
        { ball with position = newBallPosition }

    let handleCollisionWithTopAndBottomWalls ball =
        if ball.position.y <= 0.0 || ball.position.y >= GAME_HEIGHT then 
            { ball with velocity = Vector2.xy ball.velocity.x -ball.velocity.y }
        else ball

    let handleCollisionWithPaddle (paddle: Paddle) (ball: Ball) = 
        let ballTop = ball.position.y - ball.radius
        let ballBottom = ball.position.y + ball.radius
        let ballLeft = ball.position.x - ball.radius
        let ballRight = ball.position.x + ball.radius
        let paddleTop = paddle.position.y - paddle.size.y / 2.0
        let paddleBottom = paddle.position.y + paddle.size.y / 2.0
        let paddleLeft = paddle.position.x - paddle.size.x / 2.0
        let paddleRight = paddle.position.x + paddle.size.x / 2.0
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

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun model -> 
        printfn "Tick: %d" model.counter;
        Graphics.Scene3D.sphere()
        )
    |> GameBuilder.tick tick
    |> Runtime.runGame


// [<EntryPoint>]
// let main _args =
//     printfn "Hello from Pong2"
//     game
//     |> Game.draw3d (fun _ -> Graphics.Primitives3D.Sphere)
//     |> Game.tick tick
//     |> Game.run
//     0