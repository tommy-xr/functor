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
//! centimetres off the eyes, so a second, filtered step settles it.

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

        // Bilinear depth by hand (depth textures are not filterable): the
        // headset's 320x320 image read nearest would terrace every surface.
        // A texel with no surface (1.0) poisons the sample, so edges stay
        // gaps rather than skins.
        highp float sampleDepth(highp vec2 uv) {
            highp vec2 size = vec2(textureSize(depthTexture, 0).xy);
            highp vec2 st = uv * size - 0.5;
            highp vec2 f = fract(st);
            highp vec2 base = (floor(st) + 0.5) / size;
            highp vec2 texel = 1.0 / size;
            highp float a = textureLod(depthTexture, vec3(base, depthLayer), 0.0).r;
            highp float b = textureLod(depthTexture, vec3(base + vec2(texel.x, 0.0), depthLayer), 0.0).r;
            highp float c = textureLod(depthTexture, vec3(base + vec2(0.0, texel.y), depthLayer), 0.0).r;
            highp float d = textureLod(depthTexture, vec3(base + texel, depthLayer), 0.0).r;
            if (max(max(a, b), max(c, d)) >= 1.0) {
                return 1.0;
            }
            return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
        }

        highp vec3 unproject(highp mat4 m, highp vec3 ndc) {
            highp vec4 p = m * vec4(ndc, 1.0);
            return p.xyz / p.w;
        }

        // Move the point `t` along the eye ray onto the depth surface: project
        // it into the depth view, read the depth there, unproject it, and
        // return its distance along the ray. Clears `hit` behind the depth
        // camera, off its image, or where it saw no surface.
        highp float onSurface(highp vec3 dir, highp float t, bool filtered, inout bool hit) {
            highp vec4 clip = depthViewProjection * vec4(eye + dir * t, 1.0);
            highp vec2 uv = clip.xy / clip.w * 0.5 + 0.5;
            highp float depth = filtered
                ? sampleDepth(uv)
                : textureLod(depthTexture, vec3(uv, depthLayer), 0.0).r;
            if (clip.w <= 0.0 || uv != clamp(uv, 0.0, 1.0) || depth >= 1.0) {
                hit = false;
            }
            highp vec3 surface = unproject(depthInverse, vec3(uv, depth) * 2.0 - 1.0);
            return dot(surface - eye, dir);
        }

        void main() {
            highp vec2 ndc = vUv * 2.0 - 1.0;
            highp vec3 dir = normalize(unproject(eyeInverse, vec3(ndc, 1.0)) - eye);
            // Walk the eye ray onto the depth surface: a coarse step from a
            // 1.5 m guess, then a filtered one from where it landed.
            bool hit = true;
            highp float t = onSurface(dir, 1.5, false, hit);
            t = onSurface(dir, t, true, hit);
            highp vec3 p = eye + dir * t;

            // Triplanar: each axis plane draws its two in-plane line sets,
            // weighted by how squarely the surface faces it.
            highp vec3 n = normalize(cross(dFdx(p), dFdy(p)));
            highp vec3 w = pow(abs(n), vec3(4.0));
            w /= w.x + w.y + w.z;
            highp vec3 q = p / cell;
            // Distance to the nearest line, in pixels, per axis.
            highp vec3 dq = fwidth(q);
            highp vec3 d = abs(fract(q - 0.5) - 0.5) / max(dq, vec3(1e-5));
            vec3 lines = 1.0 - clamp(min(d.yxx, d.zzy) - 0.5, 0.0, 1.0);
            // Fade out where a pixel spans a large share of a cell: depth
            // edges (the point jumps between neighbours) and grazing angles.
            float alpha = dot(w, lines) * (1.0 - smoothstep(4.0, 8.0, t))
                * (1.0 - smoothstep(0.2, 0.4, max(max(dq.x, dq.y), dq.z)));
            if (!hit || t <= 0.0 || alpha <= 0.0) {
                discard;
            }
            fragColor = functorOutput(vec4(functorSrgbToLinear(color), alpha));
        }
"#;
