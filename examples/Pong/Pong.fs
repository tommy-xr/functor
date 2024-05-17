module Pong

open Functor
open Functor.Math

type Paddle = { 
    position: Point2
    size: Vector2
}
type Ball = { 
    position: Point2
    velocity: Vector2
    radius: float }

type Model = {
    paddle1: Paddle
    paddle2: Paddle
    ball: Ball
}

type Msg =
    | MovePaddle1 of float
    | MovePaddle2 of float

let initialState = {
    paddle1 = { position = Point2.zero; size = Vector2.xy 0.1 0.3 }
    paddle2 = { position = Point2.zero; size = Vector2.xy 0.1 0.3 }
    ball = { position = Point2.zero; velocity = Vector2.zero; radius = 0.05 }
}

let game: Functor.Game<unit, unit> = Game.local ()

let tick model tick = (model, Effect.none)


[<EntryPoint>]
let main _args =
    printfn "Hello from Pong2"
    game
    |> Game.draw3d (fun _ -> Graphics.Primitives3D.Sphere)
    |> Game.run
    0