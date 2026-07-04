namespace Graphics

open Fable.Core
open Functor.Math

/// A directional ("sun") light: parallel rays travelling along `Direction`.
type DirectionalLight = { Direction: Vector3; Color: Color; Intensity: float32 }
/// An omnidirectional point light at `Position`, fading to nothing by `Range`.
type PointLight = { Position: Vector3; Color: Color; Intensity: float32; Range: float32 }
/// A cone of light from `Position` aimed along `Direction`. `ConeAngle` is the
/// half-angle of the cone in radians; `Range` is the distance falloff.
type SpotLight =
    { Position: Vector3; Direction: Vector3; Color: Color; Intensity: float32; Range: float32; ConeAngle: float32 }

[<Erase; Emit("functor_runtime_common::Scene3D")>] type Scene3D = | Noop

[<Erase; Emit("functor_runtime_common::TextureDescription")>] type Texture = | Noop
[<Erase; Emit("functor_runtime_common::ModelDescription")>] type Model = | Noop
[<Erase; Emit("functor_runtime_common::MaterialDescription")>] type Material = | Noop
[<Erase; Emit("functor_runtime_common::MeshSelector")>] type MeshSelector = | Noop
[<Erase; Emit("functor_runtime_common::MeshOverride")>] type MeshOverride = | Noop
[<Erase; Emit("functor_runtime_common::Light")>] type Light = | Noop
[<Erase; Emit("functor_runtime_common::RenderTargetDescriptor")>] type RenderTarget = | Noop
[<Erase; Emit("functor_runtime_common::Fog")>] type Fog = | Noop

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

        // Diffuse-lit, with a tangent-space normal map perturbing the surface
        // normal (the bumps catch the lights / specular). `(r,g,b,a)` is the
        // albedo tint.
        [<Emit("functor_runtime_common::MaterialDescription::lit_normal_mapped($0, $1, $2, $3, $4)")>]
        let litNormalMapped (r: float32, g: float32, b: float32, a: float32, normalMap: Texture): Material = nativeOnly

    module Light =
        // Private float-based FFI shims; the public record API destructures into
        // these so the Rust boundary stays simple and the records stay pure F#.
        [<Emit("functor_runtime_common::Light::ambient($0, $1, $2)")>]
        let private ambientRaw (r: float32, g: float32, b: float32): Light = nativeOnly

        [<Emit("functor_runtime_common::Light::directional($0, $1, $2, $3, $4, $5, $6)")>]
        let private directionalRaw (dx: float32, dy: float32, dz: float32, r: float32, g: float32, b: float32, intensity: float32): Light = nativeOnly

        [<Emit("functor_runtime_common::Light::point($0, $1, $2, $3, $4, $5, $6, $7)")>]
        let private pointRaw (px: float32, py: float32, pz: float32, r: float32, g: float32, b: float32, intensity: float32, range: float32): Light = nativeOnly

        [<Emit("functor_runtime_common::Light::spot($0, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)")>]
        let private spotRaw (px: float32, py: float32, pz: float32, dx: float32, dy: float32, dz: float32, r: float32, g: float32, b: float32, intensity: float32, range: float32, coneAngle: float32): Light = nativeOnly

        /// Uniform light added to every lit surface regardless of orientation.
        let ambient (color: Color): Light =
            ambientRaw (color.r, color.g, color.b)

        /// A "sun": parallel rays travelling along `Direction`.
        let directional (l: DirectionalLight): Light =
            directionalRaw (l.Direction.x, l.Direction.y, l.Direction.z, l.Color.r, l.Color.g, l.Color.b, l.Intensity)

        /// An omnidirectional point light, fading to nothing by `Range`.
        let point (l: PointLight): Light =
            pointRaw (l.Position.x, l.Position.y, l.Position.z, l.Color.r, l.Color.g, l.Color.b, l.Intensity, l.Range)

        /// A cone of light from `Position` along `Direction`; `ConeAngle` in radians.
        let spot (l: SpotLight): Light =
            spotRaw (l.Position.x, l.Position.y, l.Position.z, l.Direction.x, l.Direction.y, l.Direction.z, l.Color.r, l.Color.g, l.Color.b, l.Intensity, l.Range, l.ConeAngle)

        /// Opt a light into casting shadows (directional or spot for now).
        [<Emit("functor_runtime_common::Light::cast_shadows($0)")>]
        let castShadows (light: Light): Light = nativeOnly

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

        /// The texture a render target is drawn into (see Frame.withRenderTarget).
        /// Usable anywhere a Texture is: Material.texture / emissiveTexture /
        /// litTexture. Sampling a target no frame declares shows magenta.
        /// Gotcha: a quad's front is +Z, so a camera looking down +Z at an
        /// unrotated quad sees its BACK — a mirrored feed. Rotate the monitor
        /// to face the viewer.
        [<Emit("functor_runtime_common::TextureDescription::render_target($0)")>]
        let renderTarget (target: RenderTarget): Texture = nativeOnly

    module Fog =
        // Private float-based FFI shims; the public API takes a Color record
        // (the Light module's pattern).
        [<Emit("functor_runtime_common::Fog::linear($0, $1, $2, $3, $4)")>]
        let private linearRaw (near: float32, far: float32, r: float32, g: float32, b: float32): Fog = nativeOnly

        [<Emit("functor_runtime_common::Fog::exp($0, $1, $2, $3)")>]
        let private expRaw (density: float32, r: float32, g: float32, b: float32): Fog = nativeOnly

        /// Linear distance fog: fully clear at `near`, fully `color` by `far`
        /// (world units, radial distance from the camera). The color also
        /// becomes the frame's clear color, so geometry dissolves into the
        /// horizon instead of silhouetting against the background.
        let linear (near: float32) (far: float32) (color: Color): Fog =
            linearRaw (near, far, color.r, color.g, color.b)

        /// Exponential fog: `factor = exp(-density * distance)` — the classic
        /// atmospheric falloff.
        let exp (density: float32) (color: Color): Fog =
            expRaw (density, color.r, color.g, color.b)

    module RenderTarget =
        /// A named offscreen render target, 512x512 until piped through `sized`.
        /// Declare once and use the value at both sites: the writer
        /// (Frame.withRenderTarget) and the reader (Texture.renderTarget); the
        /// name is the target's identity across frames and hot reloads.
        [<Emit("functor_runtime_common::RenderTargetDescriptor::named($0)")>]
        let named (name: string): RenderTarget = nativeOnly

        [<Emit("functor_runtime_common::RenderTargetDescriptor::sized($2, $0, $1)")>]
        let sized (width: float32) (height: float32) (target: RenderTarget): RenderTarget = nativeOnly

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