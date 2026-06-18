module WsDemo

// A minimal WebSocket sample. `Sub.connect` declares a persistent connection;
// when it opens, the runtime hands us a `ConnectionId` via `Connected`, which we
// store and use with `Effect.send`. Incoming messages flow to the same decoder.
// Like netdemo it is driven *headlessly* (see tests/net_ws.rs): the connection
// commands are visible, and events can be injected without a real socket.

open Functor
open Functor.Math
open Graphics

let endpoint = "ws://127.0.0.1:9001/echo"

type Model =
    { conn: Net.ConnectionId option
      status: string
      lastMsg: string }

module Model =
    let initial = { conn = None; status = "connecting"; lastMsg = "" }

type Msg =
    | Ws of Net.NetEvent
    | Send of string

let game: Game<Model, Msg> = GameBuilder.local Model.initial

let update model msg =
    match msg with
    | Ws(Net.Connected id) ->
        // Connection is open: stash the handle and greet the server.
        Debug.log "wsdemo: connected"
        ({ model with conn = Some id; status = "connected" }, Effect.send id "hello")
    | Ws(Net.Message(_, text)) ->
        Debug.log (sprintf "wsdemo: message %s" text)
        ({ model with lastMsg = text; status = "got-message" }, Effect.none ())
    | Ws(Net.Disconnected _) -> ({ model with conn = None; status = "disconnected" }, Effect.none ())
    | Ws(Net.Error(_, e)) -> ({ model with status = "error: " + e }, Effect.none ())
    | Send text ->
        match model.conn with
        | Some id -> (model, Effect.send id text)
        | None -> (model, Effect.none ()) // not connected yet

// Declaring the connection in `subscriptions` is what keeps it open.
let subscriptions _model = Sub.connect endpoint Ws

let tick model (_: Time.FrameTime) = (model, Effect.none ())
let input model (_: Input.t) = (model, Effect.none ())

open Fable.Core.Rust
open Graphics.Scene3D

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun _model _frameTime ->
        let mat = Material.emissive (0.6f, 0.4f, 0.9f, 1.0f)
        let scene = material (mat, [| cube () |])

        let camera =
            Graphics.Camera.firstPerson
                (Vector3.xyz 0.0f 0.0f -5.0f)
                (Math.Angle.radians 0.0f)
                (Math.Angle.radians 0.0f)
                (Math.Angle.degrees 60.0f)

        Graphics.Frame.create camera scene)
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    |> GameBuilder.subscriptions subscriptions
    |> Runtime.runGame
