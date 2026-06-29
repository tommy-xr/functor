module Synthwave

open Functor
open Functor.Math

// A retro-synthwave scene: a neon grid terrain of rolling hills receding toward
// the horizon and rolling toward the camera over time, a glowing sun low on the
// horizon, and a dark gradient sky. The whole scene is a pure function of time
// (frameTime.tts) — there is no per-frame model state to carry.
type Model = unit
type Msg = Noop

let game: Game<Model, Msg> = GameBuilder.local ()

let update (model: Model) (_msg: Msg) = (model, Effect.none ())

open Fable.Core.Rust
open Graphics.Scene3D

// --- Scene constants -------------------------------------------------------

// Grid resolution. One emissive texture cell tiles per grid quad, so this also
// sets how many neon grid lines you see.
let private rows = 80
let private cols = 80

// World footprint of the terrain (the heightmap spans the unit square, so we
// stretch XZ wide while leaving Y at author scale for the hill heights below).
let private terrainSize = 160.0f

// How fast the hills roll toward the camera (grid rows / second of phase).
let private scrollSpeed = 4.0f

/// Terrain height (in world units) at grid coords (row, col), scrolled by
/// `phase`. Rows run along +Z (into the distance); cols run along X. A couple of
/// sine ridges in depth give rolling hills, gently modulated across X; the floor
/// keeps valleys from dipping too far so the grid reads cleanly.
let private terrainHeight (phase: float32) (r: int) (c: int) : float32 =
    let z = float32 r + phase
    let x = float32 c
    let ridges =
        sin (z * 0.35f) * 1.6f
        + sin (z * 0.16f + x * 0.10f) * 1.1f
        + sin (x * 0.22f) * 0.5f
    // Lift the whole field so it mostly sits at/above y = 0.
    ridges + 2.0f

[<OuterAttr("no_mangle")>]
let init (_args: array<string>) =
    game
    |> GameBuilder.draw3d (fun _model (frameTime: Time.FrameTime) ->
        let t = frameTime.tts
        let phase = t * scrollSpeed

        // The neon grid terrain: an emissive grid-cell texture (dark interior,
        // hot-magenta edges) tiled one cell per quad, so the glowing lines align
        // with the mesh grid. Emissive = self-lit, so the grid glows in the dark.
        let gridMat = Material.emissiveTexture (Texture.file "grid-neon.png")
        let terrain =
            material (gridMat, [|
                heightmapFn rows cols (terrainHeight phase)
                |> Transform.translateY -2.0f
                |> Transform.scaleX terrainSize
                |> Transform.scaleZ terrainSize
            |])

        // The glowing sun, low on the horizon down +Z. A big emissive sphere;
        // its warm color pops against the dark sky behind it. (The camera's far
        // clip is 100 units, so the sun + sky backdrop are kept within that
        // budget.) Transforms compose right-to-left (rightmost in the pipe is
        // applied to the geometry first), so scale first, then translate —
        // otherwise the scale would multiply the translation offsets.
        let sun =
            material (Material.emissive (1.0f, 0.45f, 0.65f, 1.0f), [|
                sphere ()
                |> Transform.translateY 9.0f
                |> Transform.translateZ 78.0f
                |> Transform.scale 16.0f
            |])

        // A gradient sky backdrop (dark indigo up top → warm magenta at the
        // horizon) on a large emissive quad just behind the sun. The quad lies in
        // the XY plane facing the camera; emissive so it ignores scene lighting.
        let sky =
            material (Material.emissiveTexture (Texture.file "sky.png"), [|
                quad ()
                |> Transform.translateY 60.0f
                |> Transform.translateZ 84.0f
                |> Transform.scaleX 500.0f
                |> Transform.scaleY 280.0f
            |])

        let scene = group [| sky; sun; terrain |]

        // Low camera near the front edge, looking down +Z across the hills to the
        // sun on the horizon. A hair of downward pitch frames the grid.
        let camera =
            Graphics.Camera.firstPerson
                (Vector3.xyz 0.0f 5.0f -12.0f)
                (Math.Angle.radians 0.0f)
                (Math.Angle.radians -0.05f)
                (Math.Angle.degrees 70.0f)

        // Every surface here is emissive (self-lit), so the scene needs no
        // lights — `Frame.create` (no light list) is all it takes.
        Graphics.Frame.create camera scene
    )
    |> GameBuilder.update update
    |> Runtime.runGame
