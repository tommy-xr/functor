//! `Frame.withRoomOcclusion`: real surfaces hide the virtual geometry behind
//! them. A depth-only fullscreen pass before the scene walks each pixel's eye
//! ray onto the environment depth (`crate::surface_grid::DEPTH_WALK_GLSL`)
//! and writes that point's depth, so the scene's own depth test drops what
//! lies behind it. Over a composited feed those pixels keep the transparent
//! clear, so the room shows there — a hand or a tablet held in front of a
//! virtual part.

// Reuses the compositor's fullscreen quad; `vUv` spans the viewport.
pub const OCCLUSION_VERTEX_SHADER_SOURCE: &str = crate::composite::COMPOSITE_VERTEX_SHADER_SOURCE;

// Compiled after `crate::surface_grid::DEPTH_WALK_GLSL`.
pub const OCCLUSION_FRAGMENT_SHADER_SOURCE: &str = r#"
        in vec2 vUv;

        // The surface is pushed back by 3% of its distance + 1 cm, so sensor
        // noise doesn't cut into a part resting on a real desk (at the cost
        // of thin geometry that close behind a surface showing).
        const highp float PUSH_BACK_RELATIVE = 0.03;
        const highp float PUSH_BACK_ABSOLUTE = 0.01;

        // World -> this eye's clip.
        uniform highp mat4 eyeViewProjection;

        void main() {
            highp vec3 dir;
            bool hit;
            highp float t = walk(vUv * 2.0 - 1.0, dir, hit);
            if (!hit || t <= 0.0) {
                discard;
            }
            highp vec4 clip = eyeViewProjection
                * vec4(eye + dir * (t * (1.0 + PUSH_BACK_RELATIVE) + PUSH_BACK_ABSOLUTE), 1.0);
            gl_FragDepth = clamp(clip.z / clip.w * 0.5 + 0.5, 0.0, 1.0);
        }
"#;
