module NetDemo

// A minimal networking sample, in the Elm `Http.get { expect = ... }` style: on
// startup it fires an HTTP GET whose result is tagged into a message, and reflects
// it into the model. No subscription is involved. It is meant to be driven
// *headlessly* -- the model is fully visible via the debug server's /state, and a
// response can be injected without a real server (see docs/multiplayer.md and
// tests/net_http.rs). draw3d is deliberately tiny; the interesting state is text.

open Functor
open Functor.Math
open Graphics

let endpoint = "http://127.0.0.1:9000/hello"

/// Where the request is in its lifecycle.
type Phase =
    | Loading
    | Done of int * string // status, body
    | Failed of string // transport error

type Model = { phase: Phase; frame: int }

module Model =
    let initial = { phase = Loading; frame = 0 }

type Msg =
    // The HTTP result, tagged back to us by the request below.
    | GotResponse of Net.HttpResponse

let game: Game<Model, Msg> = GameBuilder.local Model.initial

// The request: GET the endpoint and deliver the result as `GotResponse` (the Elm
// `expect`). The runtime applies this tagger when the response lands.
let fetch = Effect.httpGet endpoint GotResponse

let update model msg =
    match msg with
    | GotResponse resp ->
        let phase = if resp.ok then Done(resp.status, resp.body) else Failed resp.error
        Debug.log (sprintf "netdemo: got response ok=%b status=%d" resp.ok resp.status)
        ({ model with phase = phase }, Effect.none ())

let subscriptions _model = Sub.none ()

let tick model (_: Time.FrameTime) =
    ({ model with frame = model.frame + 1 }, Effect.none ())

let input model (_: Input.t) = (model, Effect.none ())

open Fable.Core.Rust
open Graphics.Scene3D

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun _model _frameTime ->
        // A single emissive cube so there's something on screen; the meaningful
        // state is the textual phase, inspected via /state.
        let mat = Material.emissive (0.2f, 0.9f, 0.6f, 1.0f)
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
    // Fire the request on startup (runs once on the first tick; not re-run across
    // a hot reload).
    |> GameBuilder.init fetch
    |> GameBuilder.subscriptions subscriptions
    |> Runtime.runGame
