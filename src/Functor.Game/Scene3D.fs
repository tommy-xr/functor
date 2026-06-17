namespace Graphics

open Fable.Core
[<Erase; Emit("functor_runtime_common::Scene3D")>] type Scene3D = | Noop

[<Erase; Emit("functor_runtime_common::TextureDescription")>] type Texture = | Noop
[<Erase; Emit("functor_runtime_common::ModelDescription")>] type Model = | Noop
[<Erase; Emit("functor_runtime_common::MaterialDescription")>] type Material = | Noop
[<Erase; Emit("functor_runtime_common::MeshSelector")>] type MeshSelector = | Noop
[<Erase; Emit("functor_runtime_common::MeshOverride")>] type MeshOverride = | Noop
[<Erase; Emit("functor_runtime_common::Light")>] type Light = | Noop

module Scene3D =

    [<Emit("functor_runtime_common::Scene3D::cube()")>]
    let cube(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::sphere()")>]
    let sphere(): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::cylinder()")>]
    let cylinder(): Scene3D = nativeOnly

    // A unit square in the XZ plane (the ground); size it with Transform.scale.
    [<Emit("functor_runtime_common::Scene3D::plane()")>]
    let plane(): Scene3D = nativeOnly

    // A unit square in the XY plane (screen/wall-facing); for sprites/billboards.
    [<Emit("functor_runtime_common::Scene3D::quad()")>]
    let quad(): Scene3D = nativeOnly

    // A subdivided grid in the XZ plane (the ground) displaced by `heights`
    // (row-major, length rows*cols). Spans the unit square; size with
    // Transform.scale. UVs tile one texture cell per grid quad.
    [<Emit("functor_runtime_common::Scene3D::heightmap($0, $1, $2)")>]
    let heightmap (rows: int) (cols: int) (heights: float32[]): Scene3D = nativeOnly

    // Build a heightmap whose height at grid coords (row, col) is `f row col`.
    let heightmapFn (rows: int) (cols: int) (f: int -> int -> float32): Scene3D =
        let heights = Array.init (rows * cols) (fun i -> f (i / cols) (i % cols))
        heightmap rows cols heights

    [<Emit("functor_runtime_common::Scene3D::group($0)")>]
    let group (items: Scene3D[]): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::material($0, $1)")>]
    let material (material: Material, items: Scene3D[]): Scene3D = nativeOnly

    [<Emit("functor_runtime_common::Scene3D::model($0)")>]
    let model (model: Model): Scene3D = nativeOnly
    
    module Material =
        [<Emit("functor_runtime_common::MaterialDescription::color($0, $1, $2, $3)")>]
        let color (r: float32, b: float32, g: float32, a: float32): Material = nativeOnly

        [<Emit("functor_runtime_common::MaterialDescription::texture($0)")>]
        let texture (texture: Texture): Material = nativeOnly

        // Self-lit surfaces (neon / UI): rendered fullbright, unaffected by lighting.
        [<Emit("functor_runtime_common::MaterialDescription::emissive($0, $1, $2, $3)")>]
        let emissive (r: float32, g: float32, b: float32, a: float32): Material = nativeOnly

        [<Emit("functor_runtime_common::MaterialDescription::emissive_texture($0)")>]
        let emissiveTexture (texture: Texture): Material = nativeOnly

        // Diffuse-lit surfaces: shaded by the frame's ambient + directional lights.
        [<Emit("functor_runtime_common::MaterialDescription::lit($0, $1, $2, $3)")>]
        let lit (r: float32, g: float32, b: float32, a: float32): Material = nativeOnly

        [<Emit("functor_runtime_common::MaterialDescription::lit_texture($0)")>]
        let litTexture (texture: Texture): Material = nativeOnly

    module Light =
        // A "sun": parallel rays travelling along (dx,dy,dz), colored (r,g,b) * intensity.
        [<Emit("functor_runtime_common::Light::directional($0, $1, $2, $3, $4, $5, $6)")>]
        let directional (dx: float32, dy: float32, dz: float32, r: float32, g: float32, b: float32, intensity: float32): Light = nativeOnly

        // Uniform light added to every lit surface regardless of orientation.
        [<Emit("functor_runtime_common::Light::ambient($0, $1, $2)")>]
        let ambient (r: float32, g: float32, b: float32): Light = nativeOnly

        // An omnidirectional point light at (px,py,pz), fading to nothing by `range`.
        [<Emit("functor_runtime_common::Light::point($0, $1, $2, $3, $4, $5, $6, $7)")>]
        let point (px: float32, py: float32, pz: float32, r: float32, g: float32, b: float32, intensity: float32, range: float32): Light = nativeOnly

        // A cone of light from (px,py,pz) along (dx,dy,dz); `coneAngle` in radians, `range` is falloff distance.
        [<Emit("functor_runtime_common::Light::spot($0, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)")>]
        let spot (px: float32, py: float32, pz: float32, dx: float32, dy: float32, dz: float32, r: float32, g: float32, b: float32, intensity: float32, range: float32, coneAngle: float32): Light = nativeOnly

    module MeshSelector =
        [<Emit("functor_runtime_common::MeshSelector::all()")>]
        let all (): MeshSelector = nativeOnly

    module MeshOverride =
        [<Emit("functor_runtime_common::MeshOverride::material($0)")>]
        let material (material: Material): MeshOverride = nativeOnly

    module Model =
        [<Emit("functor_runtime_common::ModelDescription::file($0)")>]
        let file (str: string): Model = nativeOnly

        [<Emit("functor_runtime_common::ModelDescription::modify($2, $0, $1)")>]
        let modify (selector: MeshSelector) (meshOverride: MeshOverride) (model: Model): Model = nativeOnly

    module Texture =
        [<Emit("functor_runtime_common::TextureDescription::file($0)")>]
        let file (str: string): Texture = nativeOnly

    module Transform = 
        [<Emit("functor_runtime_common::Scene3D::translate_x($1, $0)")>]
        let translateX (x: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::translate_y($1, $0)")>]
        let translateY (y: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::translate_z($1, $0)")>]
        let translateZ (z: float32) (scene: Scene3D): Scene3D = nativeOnly

        [<Emit("functor_runtime_common::Scene3D::scale_x($1, $0)")>]

        let scaleX (x: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::scale_y($1, $0)")>]
        let scaleY (y: float32) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::scale_z($1, $0)")>]
        let scaleZ (z: float32) (scene: Scene3D): Scene3D = nativeOnly

        let scale (s: float32) (scene: Scene3D) = 
            scene
            |> scaleX s
            |> scaleY s
            |> scaleZ s

        [<Emit("functor_runtime_common::Scene3D::rotate_x($1, $0)")>]
        let rotateX (angle: Math.Angle) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::rotate_y($1, $0)")>]
        let rotateY (angle: Math.Angle) (scene: Scene3D): Scene3D = nativeOnly
        [<Emit("functor_runtime_common::Scene3D::rotate_z($1, $0)")>]
        let rotateZ (angle: Math.Angle) (scene: Scene3D): Scene3D = nativeOnly