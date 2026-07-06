//! Screen-space compositor (docs/time-travel.md T5).
//!
//! The shared foundation for fork+overlay (K=2 at 0.5/0.5) and forward-ghosting
//! (K=N at 1/N): render K pure `Frame`s into K offscreen targets, then composite
//! them onto the backbuffer as a weighted average. The average is done
//! **in-shader** (sample the K textures, sum with per-texture weight) rather than
//! via GL blend state — the 3D path keeps blending off, and an exact in-shader
//! sum makes "static geometry averages to itself (solid), motion smears (faint)"
//! fall straight out of the math.
//!
//! The GL side lives in `SceneContext::draw_composite`; the per-frame
//! orchestration in `renderer::render_composited_frames`. This module owns the
//! shader source and the pure weight logic (unit-tested without a GL context).

/// Max frames a single composite pass can average. The fragment shader unrolls a
/// fixed-size sampler array, so this is a compile-time bound (it must match the
/// `MAX_COMPOSITE` `#define` in the fragment source below). It also stays within
/// the WebGL2-guaranteed 16 texture image units.
pub const MAX_COMPOSITE: usize = 8;

/// Normalize `weights` so they sum to 1.0 — the composite is then an *average*
/// regardless of the scale the caller passes (both `[0.5, 0.5]` and `[1.0, 1.0]`
/// average two frames; `[1/N; N]` averages N). Each weight is sanitized
/// per-element first: only finite, non-negative values contribute; anything else
/// (`NaN`, `Inf`, negative) is treated as 0 so a single bad entry can't poison
/// the average or leak a `NaN` into the shader. If nothing valid remains it falls
/// back to equal weights, so a degenerate input still renders something sensible
/// rather than a black (or garbage) frame.
pub fn normalize_weights(weights: &[f32]) -> Vec<f32> {
    if weights.is_empty() {
        return Vec::new();
    }
    let clean: Vec<f32> = weights
        .iter()
        .map(|w| if w.is_finite() && *w >= 0.0 { *w } else { 0.0 })
        .collect();
    let total: f32 = clean.iter().sum();
    if total <= 0.0 {
        let equal = 1.0 / weights.len() as f32;
        return vec![equal; weights.len()];
    }
    clean.iter().map(|w| w / total).collect()
}

// A passthrough fullscreen-quad vertex shader. The unit quad
// (`geometry::Quad`) spans [-0.5, 0.5] in XY; scaling by 2 fills NDC, and its
// [0, 1] UVs address the offscreen targets 1:1.
pub const COMPOSITE_VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inUv;

        out vec2 vUv;

        void main() {
            vUv = inUv;
            gl_Position = vec4(inPos.xy * 2.0, 0.0, 1.0);
        }
"#;

// Weighted-average of MAX_COMPOSITE textures, **manually unrolled** with literal
// sampler indices. GLSL ES 3.00 (WebGL2) only reliably supports indexing a
// `sampler2D[]` by constant *expressions*; a loop index is the weaker
// "constant-index-expression" some drivers reject, so we index `uTex[0]..uTex[7]`
// with literals to stay portable to the web path (which compiles this shader in
// T6). Unused slots carry weight 0 (padded by the caller). The unrolled body must
// have exactly `MAX_COMPOSITE` lines; the `#define` sizes the uniform arrays.
pub const COMPOSITE_FRAGMENT_SHADER_SOURCE: &str = r#"
        #define MAX_COMPOSITE 8

        in vec2 vUv;
        out vec4 fragColor;

        uniform sampler2D uTex[MAX_COMPOSITE];
        uniform float uWeight[MAX_COMPOSITE];

        void main() {
            vec3 acc = vec3(0.0);
            acc += uWeight[0] * texture(uTex[0], vUv).rgb;
            acc += uWeight[1] * texture(uTex[1], vUv).rgb;
            acc += uWeight[2] * texture(uTex[2], vUv).rgb;
            acc += uWeight[3] * texture(uTex[3], vUv).rgb;
            acc += uWeight[4] * texture(uTex[4], vUv).rgb;
            acc += uWeight[5] * texture(uTex[5], vUv).rgb;
            acc += uWeight[6] * texture(uTex[6], vUv).rgb;
            acc += uWeight[7] * texture(uTex[7], vUv).rgb;
            fragColor = vec4(acc, 1.0);
        }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_normalized_is_unchanged() {
        assert_eq!(normalize_weights(&[0.5, 0.5]), vec![0.5, 0.5]);
        let n = normalize_weights(&[0.1; 10]);
        assert!(n.iter().all(|w| (w - 0.1).abs() < 1e-6));
    }

    #[test]
    fn scales_to_sum_one() {
        assert_eq!(normalize_weights(&[1.0, 1.0]), vec![0.5, 0.5]);
        let n = normalize_weights(&[3.0, 1.0]);
        assert_eq!(n, vec![0.75, 0.25]);
        assert!((n.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn zero_total_falls_back_to_equal() {
        assert_eq!(normalize_weights(&[0.0, 0.0]), vec![0.5, 0.5]);
        assert_eq!(normalize_weights(&[0.0, 0.0, 0.0, 0.0]), vec![0.25; 4]);
    }

    #[test]
    fn empty_is_empty() {
        assert!(normalize_weights(&[]).is_empty());
    }

    #[test]
    fn non_finite_and_negative_weights_are_dropped() {
        // A single bad entry must not poison the average or leak NaN downstream.
        assert_eq!(normalize_weights(&[1.0, f32::NAN]), vec![1.0, 0.0]);
        assert_eq!(normalize_weights(&[1.0, f32::INFINITY]), vec![1.0, 0.0]);
        assert_eq!(normalize_weights(&[3.0, -1.0]), vec![1.0, 0.0]);
        // Nothing valid remains -> equal-weight fallback (never a black frame).
        assert_eq!(normalize_weights(&[f32::NAN, -2.0]), vec![0.5, 0.5]);
    }

    #[test]
    fn matches_declared_shader_bound() {
        // The fragment shader hard-codes the array size; keep it in lockstep
        // with the Rust bound the padding logic relies on.
        assert!(COMPOSITE_FRAGMENT_SHADER_SOURCE.contains(&format!("#define MAX_COMPOSITE {MAX_COMPOSITE}")));
    }
}
