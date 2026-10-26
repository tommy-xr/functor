//! `Frame.withSurfaceGrid`: a world-aligned grid shaded onto the room's
//! surfaces, recovered per pixel from the environment depth
//! (`crate::environment_depth`) — the headset's depth sensing under a
//! composited feed, the stand-in room's rendered depth elsewhere.
//!
//! A fullscreen pass after the room and before the scene: each pixel walks
//! its eye ray onto the depth surface (project a guess into the depth view,
//! read the depth there, unproject it, move the guess onto it), then draws
//! thin triplanar lines at that world point. On desktop the depth view IS the
//! eye view, so one step lands exactly; the headset's depth cameras sit a few
//! centimetres off the eyes, so a few steps converge.

use serde::{Deserialize, Serialize};

/// The grid a frame shades onto the room (`Frame.withSurfaceGrid`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceGrid {
    /// Cell size in world units (metres in passthrough).
    pub cell: f32,
    /// Line colour, sRGB-authored.
    pub color: [f32; 3],
}

// Reuses the compositor's fullscreen quad; `vUv` spans the viewport.
pub const GRID_VERTEX_SHADER_SOURCE: &str = crate::composite::COMPOSITE_VERTEX_SHADER_SOURCE;

pub const GRID_FRAGMENT_SHADER_SOURCE: &str = r#"
        in vec2 vUv;
        out vec4 fragColor;

        uniform highp sampler2DArray depthTexture;
        uniform float depthLayer;
        // World -> depth clip, and back.
        uniform highp mat4 depthViewProjection;
        uniform highp mat4 depthInverse;
        // Eye clip -> world.
        uniform highp mat4 eyeInverse;
        uniform highp vec3 eye;
        uniform highp float cell;
        uniform vec3 color;

        highp vec3 unproject(highp mat4 m, highp vec3 ndc) {
            highp vec4 p = m * vec4(ndc, 1.0);
            return p.xyz / p.w;
        }

        void main() {
            highp vec2 ndc = vUv * 2.0 - 1.0;
            highp vec3 dir = normalize(unproject(eyeInverse, vec3(ndc, 1.0)) - eye);
            // Walk the eye ray onto the depth surface.
            highp float t = 1.5;
            bool hit = true;
            for (int i = 0; i < 3; i++) {
                highp vec4 clip = depthViewProjection * vec4(eye + dir * t, 1.0);
                highp vec2 uv = clip.xy / clip.w * 0.5 + 0.5;
                highp float depth = textureLod(depthTexture, vec3(uv, depthLayer), 0.0).r;
                // Behind the depth camera, off its image, or no surface.
                if (clip.w <= 0.0 || uv != clamp(uv, 0.0, 1.0) || depth >= 1.0) {
                    hit = false;
                }
                highp vec3 surface = unproject(depthInverse, vec3(uv, depth) * 2.0 - 1.0);
                t = dot(surface - eye, dir);
            }
            highp vec3 p = eye + dir * t;

            // Triplanar: each axis plane draws its two in-plane line sets,
            // weighted by how squarely the surface faces it.
            highp vec3 n = normalize(cross(dFdx(p), dFdy(p)));
            highp vec3 w = pow(abs(n), vec3(4.0));
            w /= w.x + w.y + w.z;
            highp vec3 q = p / cell;
            // Distance to the nearest line, in pixels, per axis.
            highp vec3 d = abs(fract(q - 0.5) - 0.5) / max(fwidth(q), vec3(1e-5));
            vec3 lines = 1.0 - clamp(min(d.yxx, d.zzy) - 0.5, 0.0, 1.0);
            float alpha = dot(w, lines) * (1.0 - smoothstep(4.0, 8.0, t));
            if (!hit || t <= 0.0 || alpha <= 0.0) {
                discard;
            }
            fragColor = functorOutput(vec4(functorSrgbToLinear(color), alpha));
        }
"#;
