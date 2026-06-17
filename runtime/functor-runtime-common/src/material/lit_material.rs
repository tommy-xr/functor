use cgmath::Matrix4;
use cgmath::Vector4;
use glow::HasContext;

use crate::light::{pack_lights, MAX_LIGHTS};
use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

// Diffuse-lit surface: albedo (a color, optionally modulated by a texture) shaded
// by a bounded array of lights (ambient / directional / point / spot) via Lambert
// plus distance + cone falloff. Reads the frame's lights from `RenderContext`.
// Needs the vertex normal (attribute location 2). `MAX_LIGHTS` must match the
// `pack_lights` cap.
const VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec3 inNormal;

        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;
        out vec3 worldNormal;
        out vec3 worldPos;

        void main() {
            texCoord = inTex;
            worldNormal = mat3(world) * inNormal;
            vec4 wp = world * vec4(inPos, 1.0);
            worldPos = wp.xyz;
            gl_Position = projection * view * wp;
        }
"#;

// `__MAX_LIGHTS__` is substituted at build time so the GLSL array size matches
// the Rust cap.
const FRAGMENT_SHADER_TEMPLATE: &str = r#"
        #define MAX_LIGHTS __MAX_LIGHTS__

        out vec4 fragColor;

        in vec2 texCoord;
        in vec3 worldNormal;
        in vec3 worldPos;

        uniform vec4 baseColor;
        uniform sampler2D texture1;
        uniform int useTexture;

        uniform int numLights;
        uniform int lightType[MAX_LIGHTS];      // 0=ambient 1=directional 2=point 3=spot
        uniform vec3 lightColor[MAX_LIGHTS];     // already * intensity
        uniform vec3 lightPosition[MAX_LIGHTS];  // point / spot
        uniform vec3 lightDirection[MAX_LIGHTS]; // directional / spot (travel dir)
        uniform float lightRange[MAX_LIGHTS];    // point / spot falloff distance
        uniform float lightConeCos[MAX_LIGHTS];  // spot: cos(cone angle)

        // Directional shadow map (the first casting directional light only, for now).
        uniform sampler2D shadowMap;
        uniform mat4 lightSpaceMatrix;
        uniform int shadowEnabled;

        // Inverse of the depth material's packDepth (RGBA8 -> [0,1] depth).
        float unpackDepth(vec4 rgba) {
            return dot(rgba, vec4(1.0, 1.0 / 255.0, 1.0 / 65025.0, 1.0 / 16581375.0));
        }

        // 0 = fully lit, 1 = fully shadowed. 3x3 PCF; out-of-frustum reads as lit.
        float directionalShadow(vec3 n, vec3 lightDir) {
            vec4 lightSpacePos = lightSpaceMatrix * vec4(worldPos, 1.0);
            vec3 proj = lightSpacePos.xyz / lightSpacePos.w;
            proj = proj * 0.5 + 0.5;
            if (proj.z > 1.0 || proj.x < 0.0 || proj.x > 1.0 || proj.y < 0.0 || proj.y > 1.0) {
                return 0.0;
            }
            // Slope-scaled bias to fight shadow acne on grazing surfaces.
            float bias = max(0.0015 * (1.0 - dot(n, -lightDir)), 0.0008);
            vec2 texelSize = 1.0 / vec2(textureSize(shadowMap, 0));
            float shadow = 0.0;
            for (int x = -1; x <= 1; x++) {
                for (int y = -1; y <= 1; y++) {
                    float closest = unpackDepth(texture(shadowMap, proj.xy + vec2(x, y) * texelSize));
                    shadow += (proj.z - bias > closest) ? 1.0 : 0.0;
                }
            }
            return shadow / 9.0;
        }

        void main() {
            vec3 n = normalize(worldNormal);
            vec3 lighting = vec3(0.0);

            for (int i = 0; i < numLights; i++) {
                int t = lightType[i];
                if (t == 0) {
                    lighting += lightColor[i];
                } else if (t == 1) {
                    vec3 ld = normalize(lightDirection[i]);
                    float ndotl = max(dot(n, -ld), 0.0);
                    float shadow = (shadowEnabled == 1) ? directionalShadow(n, ld) : 0.0;
                    lighting += lightColor[i] * ndotl * (1.0 - shadow);
                } else {
                    // Point (t == 2) or spot (t == 3): both attenuate by distance.
                    vec3 toLight = lightPosition[i] - worldPos;
                    float dist = length(toLight);
                    vec3 l = toLight / max(dist, 1e-4);
                    float ndotl = max(dot(n, l), 0.0);

                    float range = max(lightRange[i], 1e-4);
                    float att = clamp(1.0 - (dist * dist) / (range * range), 0.0, 1.0);
                    att *= att;

                    float spot = 1.0;
                    if (t == 3) {
                        float cosAngle = dot(-l, normalize(lightDirection[i]));
                        // Soft edge over a small band inside the cone.
                        float outer = lightConeCos[i];
                        float inner = mix(1.0, outer, 0.85);
                        spot = clamp((cosAngle - outer) / max(inner - outer, 1e-4), 0.0, 1.0);
                    }

                    lighting += lightColor[i] * ndotl * att * spot;
                }
            }

            vec4 albedo = baseColor;
            if (useTexture == 1) {
                albedo = texture(texture1, texCoord) * baseColor;
            }
            fragColor = vec4(albedo.rgb * lighting, albedo.a);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    base_color_loc: UniformLocation,
    texture_loc: UniformLocation,
    use_texture_loc: UniformLocation,
    num_lights_loc: UniformLocation,
    light_type_loc: UniformLocation,
    light_color_loc: UniformLocation,
    light_position_loc: UniformLocation,
    light_direction_loc: UniformLocation,
    light_range_loc: UniformLocation,
    light_cone_cos_loc: UniformLocation,
    shadow_map_loc: UniformLocation,
    light_space_matrix_loc: UniformLocation,
    shadow_enabled_loc: UniformLocation,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct LitMaterial {
    color: Vector4<f32>,
    use_texture: bool,
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
                    FRAGMENT_SHADER_TEMPLATE.replace("__MAX_LIGHTS__", &MAX_LIGHTS.to_string());
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
                    num_lights_loc: shader.get_uniform_location(ctx.gl, "numLights"),
                    light_type_loc: shader.get_uniform_location(ctx.gl, "lightType"),
                    light_color_loc: shader.get_uniform_location(ctx.gl, "lightColor"),
                    light_position_loc: shader.get_uniform_location(ctx.gl, "lightPosition"),
                    light_direction_loc: shader.get_uniform_location(ctx.gl, "lightDirection"),
                    light_range_loc: shader.get_uniform_location(ctx.gl, "lightRange"),
                    light_cone_cos_loc: shader.get_uniform_location(ctx.gl, "lightConeCos"),
                    shadow_map_loc: shader.get_uniform_location(ctx.gl, "shadowMap"),
                    light_space_matrix_loc: shader
                        .get_uniform_location(ctx.gl, "lightSpaceMatrix"),
                    shadow_enabled_loc: shader.get_uniform_location(ctx.gl, "shadowEnabled"),
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
        let lights = pack_lights(ctx.lights);
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

                p.set_uniform_1i(ctx.gl, &uniforms.num_lights_loc, lights.count);
                p.set_uniform_1iv(ctx.gl, &uniforms.light_type_loc, &lights.types);
                p.set_uniform_vec3v(ctx.gl, &uniforms.light_color_loc, &lights.colors);
                p.set_uniform_vec3v(ctx.gl, &uniforms.light_position_loc, &lights.positions);
                p.set_uniform_vec3v(ctx.gl, &uniforms.light_direction_loc, &lights.directions);
                p.set_uniform_1fv(ctx.gl, &uniforms.light_range_loc, &lights.ranges);
                p.set_uniform_1fv(ctx.gl, &uniforms.light_cone_cos_loc, &lights.cone_cos);

                // Directional shadow map on texture unit 1 (unit 0 is albedo).
                p.set_uniform_1i(ctx.gl, &uniforms.shadow_map_loc, 1);
                match ctx.shadow {
                    Some(shadow) => {
                        p.set_uniform_1i(ctx.gl, &uniforms.shadow_enabled_loc, 1);
                        p.set_uniform_matrix4(
                            ctx.gl,
                            &uniforms.light_space_matrix_loc,
                            &shadow.light_space_matrix,
                        );
                        ctx.gl.active_texture(glow::TEXTURE0 + 1);
                        ctx.gl
                            .bind_texture(glow::TEXTURE_2D, Some(shadow.depth_texture));
                        ctx.gl.active_texture(glow::TEXTURE0);
                    }
                    None => {
                        p.set_uniform_1i(ctx.gl, &uniforms.shadow_enabled_loc, 0);
                    }
                }
            }
        }

        true
    }
}

impl LitMaterial {
    /// `use_texture` expects a texture bound to unit 0 (the caller binds it); the
    /// sampled texel is multiplied by `color` as a tint/albedo.
    pub fn create(color: Vector4<f32>, use_texture: bool) -> Box<dyn Material> {
        Box::new(LitMaterial { color, use_texture })
    }
}
