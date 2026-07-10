use cgmath::Matrix4;
use cgmath::Vector4;

use crate::fog::{FogUniforms, FOG_GLSL};
use crate::light::{lighting_glsl, LightingUniforms};
use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

// Diffuse-lit surface: albedo (a color, optionally modulated by a texture) shaded
// by a bounded array of lights (ambient / directional / point / spot) via Lambert
// plus distance + cone falloff. Reads the frame's lights from `RenderContext`.
// Needs the vertex normal (attribute location 2). The light loop + shadow
// sampling live in the shared `lighting_glsl` snippet (also used by the skinned
// lit shader).
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec3 inNormal;
        layout (location = 3) in vec4 inTangent;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;
        out vec3 worldNormal;
        out vec3 worldTangent;
        out vec3 worldBitangent;
        out vec3 worldPos;

        void main() {
            texCoord = inTex;
            vec3 n = mat3(world) * inNormal;
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
        in vec3 worldPos;

        uniform vec4 baseColor;
        uniform sampler2D texture1;
        uniform int useTexture;

        // Tangent-space normal map (unit 2). `useNormalMap` gates it; the surface
        // tangent frame comes from the interpolated worldTangent/worldBitangent.
        uniform sampler2D normalMap;
        uniform int useNormalMap;

        void main() {
            vec3 n = normalize(worldNormal);
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
                albedo = texture(texture1, texCoord) * baseColor;
            }
            fragColor = vec4(applyFog(albedo.rgb * diffuseLight + specularLight, worldPos), albedo.a);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    base_color_loc: UniformLocation,
    texture_loc: UniformLocation,
    use_texture_loc: UniformLocation,
    normal_map_loc: UniformLocation,
    use_normal_map_loc: UniformLocation,
    lighting: LightingUniforms,
    fog: FogUniforms,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct LitMaterial {
    color: Vector4<f32>,
    use_texture: bool,
    use_normal_map: bool,
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

                let fragment_source =
                    format!("{}\n{}\n{}", FOG_GLSL, lighting_glsl(), FRAGMENT_SHADER_SOURCE);
                let fragment_shader =
                    Shader::build(ctx.gl, ShaderType::Fragment, &fragment_source, ctx.shader_version);

                let shader = crate::shader_program::ShaderProgram::link(
                    &ctx.gl,
                    &vertex_shader,
                    &fragment_shader,
                );

                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
                    base_color_loc: shader.get_uniform_location(ctx.gl, "baseColor"),
                    texture_loc: shader.get_uniform_location(ctx.gl, "texture1"),
                    use_texture_loc: shader.get_uniform_location(ctx.gl, "useTexture"),
                    normal_map_loc: shader.get_uniform_location(ctx.gl, "normalMap"),
                    use_normal_map_loc: shader.get_uniform_location(ctx.gl, "useNormalMap"),
                    lighting: LightingUniforms::get(&shader, ctx.gl),
                    fog: FogUniforms::get(&shader, ctx.gl),
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

                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);

                p.set_uniform_vec4(ctx.gl, &uniforms.base_color_loc, &self.color);
                p.set_uniform_1i(ctx.gl, &uniforms.texture_loc, 0);
                p.set_uniform_1i(ctx.gl, &uniforms.use_texture_loc, self.use_texture as i32);

                // Normal map on texture unit 2 (0 = albedo, 1 = shadow map); the
                // caller binds the texture when `use_normal_map`.
                p.set_uniform_1i(ctx.gl, &uniforms.normal_map_loc, 2);
                p.set_uniform_1i(
                    ctx.gl,
                    &uniforms.use_normal_map_loc,
                    self.use_normal_map as i32,
                );

                uniforms.lighting.set(p, ctx, view_matrix);
                uniforms.fog.set(p, ctx.gl, ctx.fog, &ctx.camera_pos);
            }
        }

        true
    }
}

impl LitMaterial {
    /// `use_texture` expects an albedo texture bound to unit 0 (the caller binds
    /// it); the sampled texel is multiplied by `color` as a tint/albedo.
    /// `use_normal_map` expects a tangent-space normal map bound to unit 2.
    pub fn create(
        color: Vector4<f32>,
        use_texture: bool,
        use_normal_map: bool,
    ) -> Box<dyn Material> {
        Box::new(LitMaterial {
            color,
            use_texture,
            use_normal_map,
        })
    }
}
