module NetDemo

// A minimal networking sample: on startup it fires an HTTP GET, then reflects the
// response into its model. It exercises the Phase 1 HTTP path end to end --
// `Effect.httpGet` (outbound) and `Sub.httpResponses` (inbound, correlated by
// token) -- and is meant to be driven *headlessly*: the model is fully visible
// via the debug server's /state, and a response can be injected with /net/inbox
// without a real server (see docs/multiplayer.md). draw3d is deliberately tiny;
// the interesting state is textual.

open Functor
open Functor.Math
open Graphics

// The correlation token for our one request. The response carries it back so we
// can tell it apart from any other in-flight request (here there's just one).
let helloToken = 1

let endpoint = "http://127.0.0.1:9000/hello"

/// Where the one request is in its lifecycle. Plain data -- it rides in the model
/// and (as messages) through the effect queue, so it stays hot-reload safe.
type Phase =
    | Idle
    | Waiting
    | Done of int * string // status, body
    | Failed of string // transport error

type Model = { phase: Phase; frame: int }

module Model =
    let initial = { phase = Idle; frame = 0 }

type Msg =
    // Kick off (or retry) the request.
    | Fetch
    // A decoded HTTP result. We carry plain fields, not the response handle, so
    // the message is plain data on the queue.
    | GotResponse of token: int * ok: bool * status: int * body: string

let game: Game<Model, Msg> = GameBuilder.local Model.initial

let update model msg =
    match msg with
    | Fetch -> ({ model with phase = Waiting }, Effect.httpGet helloToken endpoint)
    | GotResponse(token, ok, status, body) when token = helloToken ->
        let phase = if ok then Done(status, body) else Failed body
        Debug.log (sprintf "netdemo: got response token=%d ok=%b status=%d" token ok status)
        ({ model with phase = phase }, Effect.none ())
    | GotResponse _ -> (model, Effect.none ()) // not ours; ignore

// Listen for HTTP results and decode each into a plain-data message. The executor
// hands every result that lands this frame to this decoder; we filter by token in
// `update`.
let subscriptions _model =
    Sub.httpResponses (fun resp -> GotResponse(resp.token, resp.ok, resp.status, resp.body))

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
    // Seed the first fetch as the startup effect (drains on the first tick, runs
    // once -- not re-run across hot reload).
    |> GameBuilder.init (Effect.wrapped Fetch)
    |> GameBuilder.subscriptions subscriptions
    |> Runtime.runGame
