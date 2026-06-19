module MpClient

// Multiplayer client: connects to mpserver, sends its movement input, and renders
// the world snapshots the server broadcasts. It holds NO authoritative state --
// it just draws what the server last sent (naive, so movement lags by the round
// trip; that lag is what a prediction layer would later hide). WASD drives it
// live; in the netsim it auto-moves on connect so a test needs no input injection.

open Functor
open Functor.Math
open Graphics
open Fable.Core

// Authority matches mpserver's bind so the netsim routes us to it.
let serverUrl = "ws://127.0.0.1:9001/play"

[<Emit("$0.trim().parse::<i32>().unwrap_or(0)")>]
let private parseInt (s: string) : int = nativeOnly

type Model =
    { conn: Net.ConnectionId option
      world: (int * float32 * float32) list // (pid, x, z) from the latest snapshot
      status: string }

module Model =
    let initial = { conn = None; world = []; status = "connecting" }

type Msg =
    | Connected of Net.ConnectionId
    | Snapshot of string
    | Disconnected
    | ConnError of string

let toMsg (event: Net.NetEvent) : Msg =
    match event with
    | Net.Connected id -> Connected id
    | Net.Message(_, text) -> Snapshot text
    | Net.Disconnected _ -> Disconnected
    | Net.Error(_, message) -> ConnError message

let game: Game<Model, Msg> = GameBuilder.local Model.initial

// "pid,x*100,z*100|..." -> [(pid, x, z)].
let private parseSnapshot (s: string) =
    if s = "" then
        []
    else
        s.Split('|')
        |> Array.toList
        |> List.choose (fun part ->
            let f = part.Split(',')
            if f.Length = 3 then
                Some(parseInt f.[0], float32 (parseInt f.[1]) / 100.0f, float32 (parseInt f.[2]) / 100.0f)
            else
                None)

let update model msg =
    match msg with
    | Connected id ->
        Debug.log "mpclient: connected, moving"
        // Auto-move +x so a headless test produces motion without input.
        ({ model with conn = Some id; status = "connected" }, Effect.send id "1 0")
    | Snapshot text ->
        ({ model with world = parseSnapshot text; status = "in-world" }, Effect.none ())
    | Disconnected -> ({ model with conn = None; status = "disconnected" }, Effect.none ())
    | ConnError message -> ({ model with status = "error: " + message }, Effect.none ())

let subscriptions _model = Sub.connect serverUrl toMsg

let tick model (_: Time.FrameTime) = (model, Effect.none ())

// Top-level helper (a nested `match` on a union miscompiles in Fable's Rust
// backend; keeping it here sidesteps that, like hello.fs's input helpers).
let private velocityFor (key: Input.Key) : string =
    match key with
    | Input.W -> "0 1"
    | Input.S -> "0 -1"
    | Input.A -> "-1 0"
    | Input.D -> "1 0"
    | _ -> ""

// WASD -> send a velocity to the server (live demo); release -> stop.
let input model (event: Input.t) =
    match event with
    | Input.Keyboard(Input.KeyboardEvent.KeyDown key) ->
        match model.conn, velocityFor key with
        | Some id, v when v <> "" -> (model, Effect.send id v)
        | _ -> (model, Effect.none ())
    | Input.Keyboard(Input.KeyboardEvent.KeyUp _) ->
        match model.conn with
        | Some id -> (model, Effect.send id "0 0")
        | None -> (model, Effect.none ())
    | _ -> (model, Effect.none ())

open Fable.Core.Rust
open Graphics.Scene3D

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun model _frameTime ->
        // A cube per player in the last snapshot the server sent.
        let mat = Material.emissive (0.3f, 0.7f, 0.9f, 1.0f)
        let cubes =
            model.world
            |> List.map (fun (_, x, z) -> cube () |> Transform.translateX x |> Transform.translateZ z)
            |> List.toArray

        let scene = material (mat, cubes)

        let camera =
            Graphics.Camera.firstPerson
                (Vector3.xyz 0.0f 4.0f -6.0f)
                (Math.Angle.radians 0.0f)
                (Math.Angle.radians -0.4f)
                (Math.Angle.degrees 60.0f)

        Graphics.Frame.create camera scene)
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    |> GameBuilder.subscriptions subscriptions
    |> Runtime.runGame
