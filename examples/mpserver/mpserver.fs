module MpServer

// Authoritative multiplayer server: tracks a player per connection, integrates
// their movement, and broadcasts the whole world to every client each tick.
// Naive (full-state, no delta, no prediction) -- enough to prove the loop and to
// drive deterministically through the netsim. Pairs with mpclient.

open Functor
open Functor.Math
open Graphics
open Fable.Core

let bind = "127.0.0.1:9001"
let speed = 2.0f

// Parse an i32 out of a (possibly space-padded) string. Fable's string->int
// support is patchy, so go straight to Rust.
[<Emit("$0.trim().parse::<i32>().unwrap_or(0)")>]
let private parseInt (s: string) : int = nativeOnly

type Player =
    { cid: Net.ConnectionId
      pid: int
      x: float32
      z: float32
      vx: int
      vz: int }

type Model = { players: Player list; nextPid: int }

module Model =
    let initial = { players = []; nextPid = 0 }

// Distinct messages (not a single-case `Net of NetEvent` wrapper, which Fable's
// Rust backend miscompiles).
type Msg =
    | Joined of Net.ConnectionId
    | Input of (Net.ConnectionId * string)
    | Left of Net.ConnectionId
    | NetErr of string

let toMsg (event: Net.NetEvent) : Msg =
    match event with
    | Net.Connected cid -> Joined cid
    | Net.Message(cid, text) -> Input(cid, text)
    | Net.Disconnected cid -> Left cid
    | Net.Error(_, message) -> NetErr message

let game: Game<Model, Msg> = GameBuilder.local Model.initial

// Wire format: "pid,x*100,z*100|pid,x*100,z*100|...". Integer fixed-point keeps
// the client's parsing trivial.
let private encode (players: Player list) =
    players
    |> List.map (fun p -> sprintf "%d,%d,%d" p.pid (int (p.x * 100.0f)) (int (p.z * 100.0f)))
    |> String.concat "|"

let private parseInput (text: string) =
    let parts = text.Split(' ')
    if parts.Length = 2 then Some(parseInt parts.[0], parseInt parts.[1]) else None

let update model msg =
    match msg with
    | Joined cid ->
        // Spawn the new player, offset on x so they don't overlap.
        Debug.log "mpserver: client joined"
        let p = { cid = cid; pid = model.nextPid; x = float32 model.nextPid; z = 0.0f; vx = 0; vz = 0 }
        ({ model with
            players = p :: model.players
            nextPid = model.nextPid + 1 },
         Effect.none ())
    | Input(cid, text) ->
        match parseInput text with
        | Some(vx, vz) ->
            let players =
                model.players
                |> List.map (fun p -> if p.cid = cid then { p with vx = vx; vz = vz } else p)
            ({ model with players = players }, Effect.none ())
        | None -> (model, Effect.none ())
    | Left cid ->
        ({ model with players = model.players |> List.filter (fun p -> p.cid <> cid) }, Effect.none ())
    | NetErr _ -> (model, Effect.none ())

let subscriptions _model = Sub.listen bind toMsg

let tick model (t: Time.FrameTime) =
    // Integrate, then broadcast the snapshot to every client.
    let players =
        model.players
        |> List.map (fun p ->
            { p with
                x = p.x + float32 p.vx * speed * t.dts
                z = p.z + float32 p.vz * speed * t.dts })

    let model = { model with players = players }
    let snapshot = encode players

    let sends =
        players |> List.map (fun p -> Effect.send p.cid snapshot) |> List.toArray

    (model, Effect.batch sends)

let input model (_: Input.t) = (model, Effect.none ())

open Fable.Core.Rust
open Graphics.Scene3D

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun model _frameTime ->
        // One cube per connected player, at its authoritative position.
        let mat = Material.emissive (0.9f, 0.5f, 0.3f, 1.0f)
        let cubes =
            model.players
            |> List.map (fun p -> cube () |> Transform.translateX p.x |> Transform.translateZ p.z)
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
