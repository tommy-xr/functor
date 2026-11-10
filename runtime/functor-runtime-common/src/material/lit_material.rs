use cgmath::Matrix4;
use cgmath::Vector4;

use crate::math::normal_matrix;

use crate::fog::{FogUniforms, FOG_GLSL};
use crate::light::{lighting_glsl, LightingUniforms};
use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

/// Box mapping for `Scene.litWorldTexture`: a face samples the texture over
/// the two world axes it spans (picked by its normal's largest component),
/// one repeat every `tile` metres, unmirrored seen from outside (walls
/// upright). Texels stay square however the mesh is scaled. The gradients
/// are made isotropic, so it is one trilinear tap at any angle: walls seen
/// edge-on along a street cost no anisotropic taps, whatever the texture
/// requests. All highp: squared texel-scale gradients underflow mediump.
/// It repeats only on a repeating (`Srgb`) texture.
pub(crate) const WORLD_TEXTURE_GLSL: &str = r#"
        highp vec2 worldAxes(highp vec3 q, vec3 n) {
            vec3 a = abs(n);
            if (a.y > max(a.x, a.z)) {
                return vec2(n.y > 0.0 ? q.x : -q.x, q.z);
            }
            if (a.x > a.z) {
                return vec2(n.x > 0.0 ? -q.z : q.z, -q.y);
            }
            return vec2(n.z > 0.0 ? q.x : -q.x, -q.y);
        }

        vec4 worldTexture(sampler2D tex, highp vec3 p, vec3 n, highp float tile) {
            highp vec2 uv = worldAxes(p, n) / tile;
            highp vec2 dx = worldAxes(dFdx(p), n) / tile;
            highp vec2 dy = worldAxes(dFdy(p), n) / tile;
            highp float g = max(length(dx), length(dy));
            return textureGrad(tex, uv, vec2(g, 0.0), vec2(0.0, g));
        }
"#;

// Diffuse-lit surface: albedo (a color, optionally modulated by a texture) shaded
// by a bounded array of lights (ambient / directional / point / spot) via Lambert
// plus distance + cone falloff. Reads the frame's lights from `RenderContext`.
// Needs the vertex normal (attribute location 2), which is transformed by the
// normal matrix (inverse-transpose) so non-uniform scales shade correctly. The light loop + shadow
// sampling live in the shared `lighting_glsl` snippet (also used by the skinned
// lit shader).
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec3 inNormal;
        layout (location = 3) in vec4 inTangent;
        layout (location = 6) in vec4 inColor; // white unless a model's mesh has colours

        uniform mat4 world;
        // transpose(inverse(mat3(world))) — normals are covectors, so a
        // non-uniform scale transforms them by the inverse-transpose. Tangents
        // are ordinary directions and keep mat3(world).
        uniform mat3 normalMatrix;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;
        out vec3 worldNormal;
        out vec3 worldTangent;
        out vec3 worldBitangent;
        out vec3 worldPos;
        out vec4 vertexColor;

        void main() {
            vertexColor = inColor;
            texCoord = inTex;
            // Unit length here, at the vertex stage's highp: a small world scale
            // (Solid.toScene's 0.001) grows the normal ~1000x, and its squared
            // length overflows a mediump fragment `normalize` on GLES.
            vec3 n = normalize(normalMatrix * inNormal);
            vec3 t = mat3(world) * inTangent.xyz;
            worldNormal = n;
            worldTangent = t;
            // Bitangent from normal/tangent and the glTF handedness sign.
            worldBitangent = cross(n, t) * inTangent.w;
            vec4 wp = world * vec4(inPos, 1.0);
            worldPos = wp.xyz;
            gl_Position = projection * view * wp;
        }
"#;

// Concatenated after `FOG_GLSL` + `lighting_glsl()` (the shared light loop /
// shadow sampling / specular uniforms).
const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec2 texCoord;
        in vec3 worldNormal;
        in vec3 worldTangent;
        in vec3 worldBitangent;
        in highp vec3 worldPos; // full precision for the light loop (see lighting_glsl)
        in vec4 vertexColor;

        uniform vec4 baseColor;
        uniform sampler2D texture1;
        uniform int useTexture;

        // Tangent-space normal map (unit 2). `useNormalMap` gates it; the surface
        // tangent frame comes from the interpolated worldTangent/worldBitangent.
        uniform sampler2D normalMap;
        uniform int useNormalMap;
        uniform float specularScale;
        // > 0: box-map the texture by world position (WORLD_TEXTURE_GLSL).
        uniform highp float worldTile;

        void main() {
            vec3 n = normalize(worldNormal);
            vec3 faceNormal = n;
            // Perturb the surface normal by the tangent-space normal map.
            if (useNormalMap == 1) {
                vec3 tn = texture(normalMap, texCoord).xyz * 2.0 - 1.0;
                mat3 tbn = mat3(
                    normalize(worldTangent),
                    normalize(worldBitangent),
                    n);
                n = normalize(tbn * tn);
            }
            vec3 diffuseLight;
            vec3 specularLight;
            accumulateLights(n, worldPos, diffuseLight, specularLight);

            vec4 albedo = baseColor;
            if (useTexture == 1) {
                vec4 texel = worldTile > 0.0
                    ? worldTexture(texture1, worldPos, faceNormal, worldTile)
                    : texture(texture1, texCoord);
                albedo = texel * baseColor;
            }
            albedo *= vertexColor;
            fragColor = functorOutput(
                vec4(applyFog(albedo.rgb * diffuseLight + specularLight * specularScale, worldPos), albedo.a));
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    normal_matrix_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    base_color_loc: UniformLocation,
    texture_loc: UniformLocation,
    use_texture_loc: UniformLocation,
    normal_map_loc: UniformLocation,
    use_normal_map_loc: UniformLocation,
    specular_scale_loc: UniformLocation,
    world_tile_loc: UniformLocation,
    lighting: LightingUniforms,
    fog: FogUniforms,
    output: crate::color_space::OutputEncodeUniform,
    pass: crate::PassUniforms,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct LitMaterial {
    color: Vector4<f32>,
    use_texture: bool,
    use_normal_map: bool,
    /// Multiplies the shared Blinn-Phong highlight: 1 for authored lit
    /// surfaces, 0 for glTF models (see `LitMaterial::model`).
    specular_scale: f32,
    /// Metres per repeat of a world box-mapped texture; 0 reads mesh UVs.
    world_tile: f32,
}

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for LitMaterial {
    fn initialize(&mut self, ctx: &RenderContext) {
        unsafe {
            #[allow(static_mut_refs)]
            if SHADER_PROGRAM.is_none() {
                let vertex_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Vertex,
                    VERTEX_SHADER_SOURCE,
                    ctx.shader_version,
                );

                let fragment_source = format!(
                    "{}\n{}\n{}\n{}",
                    FOG_GLSL,
                    lighting_glsl(),
                    WORLD_TEXTURE_GLSL,
                    FRAGMENT_SHADER_SOURCE
                );
                let fragment_shader =
                    Shader::build(ctx.gl, ShaderType::Fragment, &fragment_source, ctx.shader_version);

                let shader = crate::shader_program::ShaderProgram::link(
                    &ctx.gl,
                    &vertex_shader,
                    &fragment_shader,
                );

                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    normal_matrix_loc: shader.get_uniform_location(ctx.gl, "normalMatrix"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
                    base_color_loc: shader.get_uniform_location(ctx.gl, "baseColor"),
                    texture_loc: shader.get_uniform_location(ctx.gl, "texture1"),
                    use_texture_loc: shader.get_uniform_location(ctx.gl, "useTexture"),
                    normal_map_loc: shader.get_uniform_location(ctx.gl, "normalMap"),
                    use_normal_map_loc: shader.get_uniform_location(ctx.gl, "useNormalMap"),
                    specular_scale_loc: shader.get_uniform_location(ctx.gl, "specularScale"),
                    world_tile_loc: shader.get_uniform_location(ctx.gl, "worldTile"),
                    lighting: LightingUniforms::get(&shader, ctx.gl),
                    fog: FogUniforms::get(&shader, ctx.gl),
                    output: crate::color_space::OutputEncodeUniform::get(&shader, ctx.gl),
                    pass: Default::default(),
                };

                SHADER_PROGRAM = Some((shader, uniforms));
            }
        }
    }

    fn draw_opaque(
        &self,
        ctx: &RenderContext,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        _skinning_data: &[Matrix4<f32>],
    ) -> bool {
        unsafe {
            #[allow(static_mut_refs)]
            if let Some((shader, uniforms)) = &SHADER_PROGRAM {
                let p = shader;
                p.use_program(ctx.gl);

                if uniforms.pass.begin(ctx) {
                    p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                    p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);
                    p.set_uniform_1i(ctx.gl, &uniforms.texture_loc, 0);
                    // Normal map on texture unit 2 (0 = albedo, 1 = shadow map);
                    // the caller binds the texture when `use_normal_map`.
                    p.set_uniform_1i(ctx.gl, &uniforms.normal_map_loc, 2);
                    crate::render::pin_white_vertex_color(ctx.gl);
                    uniforms.lighting.set_pass(p, ctx, view_matrix);
                    uniforms.fog.set(p, ctx.gl, ctx.fog, ctx.sky_gradient, &ctx.camera_pos);
                }

                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix3(
                    ctx.gl,
                    &uniforms.normal_matrix_loc,
                    &normal_matrix(world_matrix),
                );

                // Authored albedo: decode sRGB→linear at the uniform boundary
                // (the sampled texture decodes in hardware — sRGB storage).
                p.set_uniform_vec4(
                    ctx.gl,
                    &uniforms.base_color_loc,
                    &crate::color_space::srgb_to_linear_vec4(self.color),
                );
                p.set_uniform_1i(ctx.gl, &uniforms.use_texture_loc, self.use_texture as i32);
                p.set_uniform_1i(
                    ctx.gl,
                    &uniforms.use_normal_map_loc,
                    self.use_normal_map as i32,
                );
                p.set_uniform_1f(ctx.gl, &uniforms.specular_scale_loc, self.specular_scale);
                p.set_uniform_1f(ctx.gl, &uniforms.world_tile_loc, self.world_tile);

                LightingUniforms::bind_shadow_map(ctx);
                // Per draw: a translucent subtree switches the alpha mode.
                uniforms.output.set(p, ctx.gl, ctx.output_srgb_encode, ctx.output_alpha());
            }
        }

        true
    }
}

impl LitMaterial {
    /// `use_texture` expects an albedo texture bound to unit 0 (the caller binds
    /// it); the sampled texel is multiplied by `color` as a tint/albedo.
    /// `use_normal_map` expects a tangent-space normal map bound to unit 2.
    /// `world_tile` > 0 box-maps the texture by world position instead.
    pub fn create(
        color: Vector4<f32>,
        use_texture: bool,
        use_normal_map: bool,
        world_tile: f32,
    ) -> Box<dyn Material> {
        Box::new(LitMaterial {
            color,
            use_texture,
            use_normal_map,
            specular_scale: 1.0,
            world_tile,
        })
    }

    /// A static glTF model's lit material: its base-colour texture on unit 0,
    /// and no highlight. The loader reads no metallic-roughness data, and
    /// glTF's default roughness is 1 (fully rough), so the shared Blinn-Phong
    /// highlight — sized for authored shiny primitives — would only paint
    /// colourless glints across photoscanned rock and bark.
    pub fn model() -> Box<dyn Material> {
        Box::new(LitMaterial {
            color: Vector4::new(1.0, 1.0, 1.0, 1.0),
            use_texture: true,
            use_normal_map: false,
            specular_scale: 0.0,
            world_tile: 0.0,
        })
    }
}
