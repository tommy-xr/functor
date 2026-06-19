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

// Same palette as mpserver, keyed by player id, so a player is the same color in
// every pane (server + clients).
let colorFor (pid: int) : float32 * float32 * float32 =
    match pid % 4 with
    | 0 -> (0.90f, 0.35f, 0.35f)
    | 1 -> (0.35f, 0.60f, 0.95f)
    | 2 -> (0.45f, 0.85f, 0.45f)
    | _ -> (0.95f, 0.80f, 0.35f)

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun model _frameTime ->
        // One colored cube per player in the last snapshot the server sent (same
        // framing + palette as mpserver, so panes line up in the netsim viewer).
        let playerNodes =
            model.world
            |> List.map (fun (pid, x, z) ->
                let (r, g, b) = colorFor pid
                material (
                    Material.lit (r, g, b, 1.0f),
                    [| cube () |> Transform.scale 0.6f |> Transform.translateX x |> Transform.translateZ z |]
                ))
            |> List.toArray

        let ground =
            material (Material.lit (0.18f, 0.2f, 0.28f, 1.0f), [| plane () |> Transform.scale 8.0f |])

        let scene = group (Array.append [| ground |] playerNodes)

        let lights =
            [| Light.ambient (Color.rgb 0.35f 0.35f 0.42f)
               Light.directional
                   { Direction = Vector3.xyz -0.4f -1.0f -0.35f
                     Color = Color.rgb 1.0f 0.95f 0.85f
                     Intensity = 1.1f } |]

        let camera =
            Graphics.Camera.firstPerson
                (Vector3.xyz 0.0f 9.0f -2.0f)
                (Math.Angle.radians 0.0f)
                (Math.Angle.radians -1.2f)
                (Math.Angle.degrees 70.0f)

        Graphics.Frame.createLit camera scene lights)
    |> GameBuilder.update update
    |> GameBuilder.input input
    |> GameBuilder.tick tick
    |> GameBuilder.subscriptions subscriptions
    |> Runtime.runGame
