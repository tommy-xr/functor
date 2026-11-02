//! `Frame.withRoomOcclusion`: real surfaces hide the virtual geometry behind
//! them. A depth-only fullscreen pass before the scene walks each pixel's eye
//! ray onto the environment depth (`crate::surface_grid::DEPTH_WALK_GLSL`)
//! and writes that point's depth, so the scene's own depth test drops what
//! lies behind it. Over a composited feed those pixels keep the transparent
//! clear, so the room shows there — a hand or a tablet held in front of a
//! virtual part.

// Reuses the compositor's fullscreen quad; `vUv` spans the viewport.
pub const OCCLUSION_VERTEX_SHADER_SOURCE: &str = crate::composite::COMPOSITE_VERTEX_SHADER_SOURCE;

/// The depth surface is pushed back by `t * RELATIVE + ABSOLUTE` metres, so
/// sensor noise doesn't cut into geometry resting on a real surface (a part
/// on a desk), at the cost of thin geometry that close behind it showing.
const PUSH_BACK_RELATIVE: f32 = 0.03;
const PUSH_BACK_ABSOLUTE: f32 = 0.01;

/// The fragment shader, after `DEPTH_WALK_GLSL`.
pub fn fragment_shader_source() -> String {
    format!(
        "{}\nconst highp float PUSH_BACK_RELATIVE = {PUSH_BACK_RELATIVE:?};\n\
         const highp float PUSH_BACK_ABSOLUTE = {PUSH_BACK_ABSOLUTE:?};\n{OCCLUSION_FRAGMENT_SHADER_SOURCE}",
        crate::surface_grid::DEPTH_WALK_GLSL
    )
}

const OCCLUSION_FRAGMENT_SHADER_SOURCE: &str = r#"
        in vec2 vUv;

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
