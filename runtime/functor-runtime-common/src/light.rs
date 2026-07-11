use cgmath::{Matrix4, SquareMatrix, Vector3};
use glow::HasContext;
use serde::{Deserialize, Serialize};

use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::RenderContext;

/// A light source. Pure data in the `Frame`, so lights serialize for `/scene`
/// introspection. Colors/directions are plain `[f32; 3]` (Serialize-friendly).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Light {
    /// Uniform light from every direction: its color is added to every lit
    /// surface regardless of orientation.
    Ambient { color: [f32; 3] },
    /// A distant "sun": parallel rays travelling along `direction`. `intensity`
    /// scales `color`. `casts_shadows` opts it into rendering a shadow map.
    Directional {
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        #[serde(default)]
        casts_shadows: bool,
    },
    /// An omnidirectional point light at `position`, fading to nothing by
    /// `range` (world units). (`casts_shadows` is carried for the API, but point
    /// shadows — a cube map — are not implemented yet.)
    Point {
        position: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        range: f32,
        #[serde(default)]
        casts_shadows: bool,
    },
    /// A cone of light from `position` aimed along `direction`, with a soft edge
    /// at `cone_angle` (radians from the axis) and distance falloff to `range`.
    Spot {
        position: [f32; 3],
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        range: f32,
        cone_angle: f32,
        #[serde(default)]
        casts_shadows: bool,
    },
}

impl Light {
    pub fn ambient(r: f32, g: f32, b: f32) -> Light {
        Light::Ambient { color: [r, g, b] }
    }

    pub fn directional(
        dx: f32,
        dy: f32,
        dz: f32,
        r: f32,
        g: f32,
        b: f32,
        intensity: f32,
    ) -> Light {
        Light::Directional {
            direction: [dx, dy, dz],
            color: [r, g, b],
            intensity,
            casts_shadows: false,
        }
    }

    pub fn point(px: f32, py: f32, pz: f32, r: f32, g: f32, b: f32, intensity: f32, range: f32) -> Light {
        Light::Point {
            position: [px, py, pz],
            color: [r, g, b],
            intensity,
            range,
            casts_shadows: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spot(
        px: f32,
        py: f32,
        pz: f32,
        dx: f32,
        dy: f32,
        dz: f32,
        r: f32,
        g: f32,
        b: f32,
        intensity: f32,
        range: f32,
        cone_angle: f32,
    ) -> Light {
        Light::Spot {
            position: [px, py, pz],
            direction: [dx, dy, dz],
            color: [r, g, b],
            intensity,
            range,
            cone_angle,
            casts_shadows: false,
        }
    }

    /// Opt this light into casting shadows (renders a shadow map). No-op for
    /// ambient lights.
    pub fn cast_shadows(self) -> Light {
        match self {
            Light::Directional {
                direction,
                color,
                intensity,
                ..
            } => Light::Directional {
                direction,
                color,
                intensity,
                casts_shadows: true,
            },
            Light::Point {
                position,
                color,
                intensity,
                range,
                ..
            } => Light::Point {
                position,
                color,
                intensity,
                range,
                casts_shadows: true,
            },
            Light::Spot {
                position,
                direction,
                color,
                intensity,
                range,
                cone_angle,
                ..
            } => Light::Spot {
                position,
                direction,
                color,
                intensity,
                range,
                cone_angle,
                casts_shadows: true,
            },
            Light::Ambient { .. } => self,
        }
    }

    /// Whether this light is opted into casting shadows.
    pub fn casts_shadows(&self) -> bool {
        match self {
            Light::Directional { casts_shadows, .. }
            | Light::Point { casts_shadows, .. }
            | Light::Spot { casts_shadows, .. } => *casts_shadows,
            Light::Ambient { .. } => false,
        }
    }
}

/// The maximum lights a single `LitMaterial` shader evaluates per draw. Beyond
/// this the extra lights are dropped (the multi-pass additive path is a later
/// step). Must match `MAX_LIGHTS` in the lit shader.
pub const MAX_LIGHTS: usize = 8;

// Light type tags, shared with the lit shader's `lightType[]`.
const TYPE_AMBIENT: i32 = 0;
const TYPE_DIRECTIONAL: i32 = 1;
const TYPE_POINT: i32 = 2;
const TYPE_SPOT: i32 = 3;

/// Lights packed into fixed-length, flattened arrays for upload to the lit
/// shader's bounded uniform arrays. `color` is premultiplied by intensity.
pub struct LightUniforms {
    pub count: i32,
    pub types: [i32; MAX_LIGHTS],
    pub colors: [f32; 3 * MAX_LIGHTS],
    pub positions: [f32; 3 * MAX_LIGHTS],
    pub directions: [f32; 3 * MAX_LIGHTS],
    pub ranges: [f32; MAX_LIGHTS],
    pub cone_cos: [f32; MAX_LIGHTS],
}

/// Pack up to `MAX_LIGHTS` lights into flattened arrays for the lit shader.
pub fn pack_lights(lights: &[Light]) -> LightUniforms {
    let mut u = LightUniforms {
        count: 0,
        types: [TYPE_AMBIENT; MAX_LIGHTS],
        colors: [0.0; 3 * MAX_LIGHTS],
        positions: [0.0; 3 * MAX_LIGHTS],
        directions: [0.0; 3 * MAX_LIGHTS],
        ranges: [1.0; MAX_LIGHTS],
        cone_cos: [-1.0; MAX_LIGHTS],
    };

    for light in lights.iter().take(MAX_LIGHTS) {
        let i = u.count as usize;
        let c3 = i * 3;
        let mut set_color = |col: &[f32; 3], intensity: f32| {
            u.colors[c3] = col[0] * intensity;
            u.colors[c3 + 1] = col[1] * intensity;
            u.colors[c3 + 2] = col[2] * intensity;
        };
        match light {
            Light::Ambient { color } => {
                u.types[i] = TYPE_AMBIENT;
                set_color(color, 1.0);
            }
            Light::Directional {
                direction,
                color,
                intensity,
                ..
            } => {
                u.types[i] = TYPE_DIRECTIONAL;
                set_color(color, *intensity);
                u.directions[c3..c3 + 3].copy_from_slice(direction);
            }
            Light::Point {
                position,
                color,
                intensity,
                range,
                ..
            } => {
                u.types[i] = TYPE_POINT;
                set_color(color, *intensity);
                u.positions[c3..c3 + 3].copy_from_slice(position);
                u.ranges[i] = *range;
            }
            Light::Spot {
                position,
                direction,
                color,
                intensity,
                range,
                cone_angle,
                ..
            } => {
                u.types[i] = TYPE_SPOT;
                set_color(color, *intensity);
                u.positions[c3..c3 + 3].copy_from_slice(position);
                u.directions[c3..c3 + 3].copy_from_slice(direction);
                u.ranges[i] = *range;
                u.cone_cos[i] = cone_angle.cos();
            }
        }
        u.count += 1;
    }

    u
}

/// The shared lit-shading GLSL, prepended (after `FOG_GLSL`) to every lit
/// forward fragment shader — the static `LitMaterial` and the skinned
/// `SkinnedMaterial` concat it so they shade identically. Declares the packed
/// light uniforms, the shadow-map uniforms, and `accumulateLights`, which sums
/// the frame's diffuse + specular light at a surface point (shadowing the
/// casting light's contribution only). `__MAX_LIGHTS__` is substituted by
/// [`lighting_glsl`] so the GLSL array size matches the Rust cap.
const LIGHTING_GLSL_TEMPLATE: &str = r#"
        #define MAX_LIGHTS __MAX_LIGHTS__

        uniform int numLights;
        uniform int lightType[MAX_LIGHTS];      // 0=ambient 1=directional 2=point 3=spot
        uniform vec3 lightColor[MAX_LIGHTS];     // already * intensity
        uniform vec3 lightPosition[MAX_LIGHTS];  // point / spot
        uniform vec3 lightDirection[MAX_LIGHTS]; // directional / spot (travel dir)
        uniform float lightRange[MAX_LIGHTS];    // point / spot falloff distance
        uniform float lightConeCos[MAX_LIGHTS];  // spot: cos(cone angle)

        // Camera world position, for the Blinn-Phong specular view direction.
        uniform vec3 viewPos;
        const float shininess = 32.0;
        const float specularStrength = 0.4;

        // Shadow map of the single casting light (directional or spot for now).
        // `shadowLightIndex` is which light it belongs to, so only that light's
        // contribution is shadowed.
        uniform sampler2D shadowMap;
        uniform mat4 lightSpaceMatrix;
        uniform int shadowEnabled;
        uniform int shadowLightIndex;

        // Inverse of the depth material's packDepth (RGBA8 -> [0,1] depth).
        float unpackDepth(vec4 rgba) {
            return dot(rgba, vec4(1.0, 1.0 / 255.0, 1.0 / 65025.0, 1.0 / 16581375.0));
        }

        // 0 = fully lit, 1 = fully shadowed. 3x3 PCF; out-of-frustum reads as lit.
        // Works for ortho (directional) and perspective (spot) light matrices —
        // the divide by w handles the perspective case.
        float sampleShadow(vec3 worldPos, float ndotl) {
            vec4 lightSpacePos = lightSpaceMatrix * vec4(worldPos, 1.0);
            vec3 proj = lightSpacePos.xyz / lightSpacePos.w;
            proj = proj * 0.5 + 0.5;
            if (proj.z > 1.0 || proj.x < 0.0 || proj.x > 1.0 || proj.y < 0.0 || proj.y > 1.0) {
                return 0.0;
            }
            // Slope-scaled bias to fight shadow acne on grazing surfaces.
            float bias = max(0.0015 * (1.0 - ndotl), 0.0008);
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

        // Sum the frame's lights at a surface point. Diffuse and specular are
        // kept separate so specular highlights stay the light's color, not
        // tinted by albedo (the caller multiplies only diffuse by albedo).
        void accumulateLights(vec3 n, vec3 worldPos, out vec3 diffuseLight, out vec3 specularLight) {
            vec3 viewDir = normalize(viewPos - worldPos);
            diffuseLight = vec3(0.0);
            specularLight = vec3(0.0);

            for (int i = 0; i < numLights; i++) {
                int t = lightType[i];
                if (t == 0) {
                    diffuseLight += lightColor[i]; // ambient: never shadowed
                    continue;
                }

                // Unit vector toward the light, and an attenuation factor.
                vec3 l;
                float atten = 1.0;
                if (t == 1) {
                    l = -normalize(lightDirection[i]);
                } else {
                    // Point (t == 2) or spot (t == 3): both attenuate by distance.
                    vec3 toLight = lightPosition[i] - worldPos;
                    float dist = length(toLight);
                    l = toLight / max(dist, 1e-4);

                    float range = max(lightRange[i], 1e-4);
                    float a = clamp(1.0 - (dist * dist) / (range * range), 0.0, 1.0);
                    atten = a * a;

                    if (t == 3) {
                        float cosAngle = dot(-l, normalize(lightDirection[i]));
                        // Soft edge over a small band inside the cone.
                        float outer = lightConeCos[i];
                        float inner = mix(1.0, outer, 0.85);
                        atten *= clamp((cosAngle - outer) / max(inner - outer, 1e-4), 0.0, 1.0);
                    }
                }

                float ndotl = max(dot(n, l), 0.0);
                vec3 diffuse = lightColor[i] * ndotl * atten;
                // Blinn-Phong specular, only where the surface faces the light.
                float spec = (ndotl > 0.0)
                    ? pow(max(dot(n, normalize(l + viewDir)), 0.0), shininess)
                    : 0.0;
                vec3 specular = lightColor[i] * spec * specularStrength * atten;

                // Shadow only the casting light's contribution (diffuse + spec).
                if (shadowEnabled == 1 && i == shadowLightIndex) {
                    float lit = 1.0 - sampleShadow(worldPos, ndotl);
                    diffuse *= lit;
                    specular *= lit;
                }
                diffuseLight += diffuse;
                specularLight += specular;
            }
        }
"#;

/// [`LIGHTING_GLSL_TEMPLATE`] with the `MAX_LIGHTS` cap substituted in.
pub fn lighting_glsl() -> String {
    LIGHTING_GLSL_TEMPLATE.replace("__MAX_LIGHTS__", &MAX_LIGHTS.to_string())
}

/// The lighting + shadow uniform locations of one lit forward shader program
/// (the `FogUniforms` pattern). Every material whose fragment shader includes
/// [`lighting_glsl`] looks these up in `initialize` and uploads via
/// [`LightingUniforms::set`] each draw.
pub struct LightingUniforms {
    num_lights_loc: UniformLocation,
    light_type_loc: UniformLocation,
    light_color_loc: UniformLocation,
    light_position_loc: UniformLocation,
    light_direction_loc: UniformLocation,
    light_range_loc: UniformLocation,
    light_cone_cos_loc: UniformLocation,
    view_pos_loc: UniformLocation,
    shadow_map_loc: UniformLocation,
    light_space_matrix_loc: UniformLocation,
    shadow_enabled_loc: UniformLocation,
    shadow_light_index_loc: UniformLocation,
}

impl LightingUniforms {
    pub fn get(shader: &ShaderProgram, gl: &glow::Context) -> LightingUniforms {
        LightingUniforms {
            num_lights_loc: shader.get_uniform_location(gl, "numLights"),
            light_type_loc: shader.get_uniform_location(gl, "lightType"),
            light_color_loc: shader.get_uniform_location(gl, "lightColor"),
            light_position_loc: shader.get_uniform_location(gl, "lightPosition"),
            light_direction_loc: shader.get_uniform_location(gl, "lightDirection"),
            light_range_loc: shader.get_uniform_location(gl, "lightRange"),
            light_cone_cos_loc: shader.get_uniform_location(gl, "lightConeCos"),
            view_pos_loc: shader.get_uniform_location(gl, "viewPos"),
            shadow_map_loc: shader.get_uniform_location(gl, "shadowMap"),
            light_space_matrix_loc: shader.get_uniform_location(gl, "lightSpaceMatrix"),
            shadow_enabled_loc: shader.get_uniform_location(gl, "shadowEnabled"),
            shadow_light_index_loc: shader.get_uniform_location(gl, "shadowLightIndex"),
        }
    }

    /// Upload this draw's lights (packed from `ctx.lights`), the specular view
    /// position (the inverse-view translation), and the shadow map — bound to
    /// texture unit 1 (unit 0 is albedo, 2 the normal map), leaving unit 0
    /// active.
    pub fn set(&self, p: &ShaderProgram, ctx: &RenderContext, view_matrix: &Matrix4<f32>) {
        let gl = ctx.gl;
        let lights = pack_lights(ctx.lights);

        p.set_uniform_1i(gl, &self.num_lights_loc, lights.count);
        p.set_uniform_1iv(gl, &self.light_type_loc, &lights.types);
        p.set_uniform_vec3v(gl, &self.light_color_loc, &lights.colors);
        p.set_uniform_vec3v(gl, &self.light_position_loc, &lights.positions);
        p.set_uniform_vec3v(gl, &self.light_direction_loc, &lights.directions);
        p.set_uniform_1fv(gl, &self.light_range_loc, &lights.ranges);
        p.set_uniform_1fv(gl, &self.light_cone_cos_loc, &lights.cone_cos);

        // Camera world position (inverse-view translation) for the specular
        // view direction.
        let view_pos = view_matrix
            .invert()
            .map(|inv| inv.w.truncate())
            .unwrap_or(Vector3::new(0.0, 0.0, 0.0));
        p.set_uniform_vec3(gl, &self.view_pos_loc, &view_pos);

        p.set_uniform_1i(gl, &self.shadow_map_loc, 1);
        match ctx.shadow {
            Some(shadow) => {
                p.set_uniform_1i(gl, &self.shadow_enabled_loc, 1);
                p.set_uniform_1i(gl, &self.shadow_light_index_loc, shadow.light_index);
                p.set_uniform_matrix4(gl, &self.light_space_matrix_loc, &shadow.light_space_matrix);
                unsafe {
                    gl.active_texture(glow::TEXTURE0 + 1);
                    gl.bind_texture(glow::TEXTURE_2D, Some(shadow.depth_texture));
                    gl.active_texture(glow::TEXTURE0);
                }
            }
            None => {
                p.set_uniform_1i(gl, &self.shadow_enabled_loc, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{lighting_glsl, pack_lights, Light, MAX_LIGHTS};

    #[test]
    fn lighting_glsl_substitutes_the_cap() {
        let glsl = lighting_glsl();
        assert!(glsl.contains(&format!("#define MAX_LIGHTS {MAX_LIGHTS}")));
        assert!(!glsl.contains("__MAX_LIGHTS__"));
    }

    #[test]
    fn packs_each_type_with_premultiplied_color() {
        let lights = [
            Light::ambient(0.1, 0.2, 0.3),
            Light::directional(0.0, -1.0, 0.0, 1.0, 1.0, 1.0, 2.0),
            Light::point(1.0, 2.0, 3.0, 1.0, 0.0, 0.0, 4.0, 10.0),
            Light::spot(0.0, 5.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 20.0, 0.0),
        ];
        let u = pack_lights(&lights);
        assert_eq!(u.count, 4);
        assert_eq!(u.types[0..4], [0, 1, 2, 3]);
        // Directional color premultiplied by intensity 2.
        assert_eq!(&u.colors[3..6], &[2.0, 2.0, 2.0]);
        // Point: position + range; color * intensity 4.
        assert_eq!(&u.positions[6..9], &[1.0, 2.0, 3.0]);
        assert_eq!(u.ranges[2], 10.0);
        assert_eq!(&u.colors[6..9], &[4.0, 0.0, 0.0]);
        // Spot: cone_cos = cos(0) = 1.
        assert!((u.cone_cos[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_is_zero_count() {
        assert_eq!(pack_lights(&[]).count, 0);
    }

    #[test]
    fn caps_at_max_lights() {
        let many = vec![Light::ambient(0.1, 0.1, 0.1); MAX_LIGHTS + 4];
        assert_eq!(pack_lights(&many).count, MAX_LIGHTS as i32);
    }
}
