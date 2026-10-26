//! The stand-in "room" behind `Frame.withPassthrough` frames wherever no real
//! camera feed is composited (desktop, web, a paused or missing feed), so
//! scale and floor height read without a headset.
//!
//! A procedural backdrop drawn like the skybox (a camera-centred cube at the
//! far plane, no depth write), shaded per pixel from the view ray: below the
//! horizon the ray hits the floor plane `y = 0` — the stage floor, 1 unit =
//! 1 m — which carries a faint 1 m grid fading with distance; above it, a
//! wall tone brightening toward the ceiling. No textures or assets, so
//! captures are deterministic.

// Reuses the skybox's camera-glued cube (see `skybox.rs`).
pub const ROOM_VERTEX_SHADER_SOURCE: &str = crate::skybox::SKYBOX_VERTEX_SHADER_SOURCE;

pub const ROOM_FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec3 texDir;

        uniform highp vec3 eye;

        // sRGB-authored tones.
        const vec3 FLOOR = vec3(0.30, 0.29, 0.28);
        const vec3 GRID = vec3(0.44, 0.43, 0.42);
        const vec3 WALL_LOW = vec3(0.58, 0.58, 0.60);
        const vec3 WALL_HIGH = vec3(0.74, 0.75, 0.78);

        void main() {
            highp vec3 d = normalize(texDir);
            vec3 c;
            if (d.y < 0.0 && eye.y > 0.0) {
                // Where the view ray meets the floor plane.
                highp float t = eye.y / -d.y;
                highp vec2 p = eye.xz + d.xz * t;
                // Distance to the nearest 1 m line, in pixels (antialiased).
                highp vec2 g = abs(fract(p - 0.5) - 0.5) / fwidth(p);
                float line = 1.0 - min(min(g.x, g.y), 1.0);
                float fade = 1.0 - smoothstep(2.0, 10.0, length(p - eye.xz));
                c = mix(FLOOR, GRID, line * fade);
            } else {
                c = mix(WALL_LOW, WALL_HIGH, clamp(d.y, 0.0, 1.0));
            }
            fragColor = functorOutput(vec4(functorSrgbToLinear(c), 1.0));
        }
"#;
