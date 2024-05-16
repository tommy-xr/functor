module Pong

open Game

let game: Game<unit, unit> = local ()


[<EntryPoint>]
let main _args =
    printfn "Hello from Pong2"
    game
    |> draw3d (fun _ -> Graphics.Primitives3D.Sphere)
    |> run
    0