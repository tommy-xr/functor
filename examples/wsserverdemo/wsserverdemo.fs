module WsServerDemo

// A minimal WebSocket *server* sample (native-only). `Sub.listen` binds an
// address; each accepted client surfaces through the decoder carrying its own
// `ConnectionId`, which we use to reply with `Effect.send`. Driven headlessly in
// tests/net_wsserver.rs by injecting per-client events.

open Functor
open Functor.Math
open Graphics

let bind = "127.0.0.1:9100"

type Model =
    { clients: int
      status: string
      lastMsg: string }

module Model =
    let initial = { clients = 0; status = "listening"; lastMsg = "" }

// Decode connection events into game-specific messages (rather than wrapping
// NetEvent in a single-case union, which Fable's Rust backend miscompiles).
type Msg =
    | ClientConnected of Net.ConnectionId
    | Received of (Net.ConnectionId * string)
    | ClientLeft of Net.ConnectionId
    | ConnError of string

let toMsg (event: Net.NetEvent) : Msg =
    match event with
    | Net.Connected id -> ClientConnected id
    | Net.Message(id, text) -> Received(id, text)
    | Net.Disconnected id -> ClientLeft id
    | Net.Error(_, message) -> ConnError message

let game: Game<Model, Msg> = GameBuilder.local Model.initial

let update model msg =
    match msg with
    | ClientConnected id ->
        // A new client: greet just that client (by its id).
        Debug.log "wsserver: client connected"
        ({ model with
            clients = model.clients + 1
            status = "client-connected" },
         Effect.send id "welcome")
    | Received(id, text) ->
        // Echo back to whoever sent it.
        Debug.log (sprintf "wsserver: echo %s" text)
        ({ model with status = "echoed"; lastMsg = text }, Effect.send id text)
    | ClientLeft _ ->
        ({ model with
            clients = model.clients - 1
            status = "client-left" },
         Effect.none ())
    | ConnError e -> ({ model with status = "error: " + e }, Effect.none ())

// Listening is declared the same way a client connection is.
let subscriptions _model = Sub.listen bind toMsg

let tick model (_: Time.FrameTime) = (model, Effect.none ())
let input model (_: Input.t) = (model, Effect.none ())

open Fable.Core.Rust
open Graphics.Scene3D

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun _model _frameTime ->
        let mat = Material.emissive (0.9f, 0.5f, 0.3f, 1.0f)
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
