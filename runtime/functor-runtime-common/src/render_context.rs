use std::sync::Arc;

use cgmath::{Matrix4, Vector3};

use crate::{asset::AssetCache, fog::Fog, FrameTime, Light};

/// Which rendering pass is in flight. `DepthOnly` (e.g. filling a shadow map)
/// draws geometry with a trivial depth material from the light's viewpoint;
/// `Forward` is the normal shaded pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderPass {
    #[default]
    Forward,
    DepthOnly,
}

/// Shadow data made available to the forward pass: the directional light's depth
/// map and the matrix that projects a world position into that light's clip
/// space (for sampling the map).
#[derive(Clone, Copy)]
pub struct ShadowUniforms {
    pub depth_texture: glow::Texture,
    pub light_space_matrix: Matrix4<f32>,
    /// Index (into the packed light array) of the light that cast this map, so
    /// the lit shader applies the shadow to that light's contribution only.
    pub light_index: i32,
}

/// Global override for how the scene is shaded — a debug aid, not a per-material
/// choice. `Default` uses each node's own material; the others replace it with a
/// diagnostic shader across the whole frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugRenderMode {
    #[default]
    Default,
    /// Visualize world-space surface normals as RGB (`normal * 0.5 + 0.5`).
    Normals,
    /// Visualize world-space surface tangents as RGB (`tangent * 0.5 + 0.5`) —
    /// the guard for the tangent vertex attribute (glTF import + analytic
    /// generation), as `Normals` is for normals.
    Tangents,
    /// Normal shading plus a physics wireframe overlay: the live world's
    /// colliders/contacts as colored lines (rapier's debug renderer via
    /// `physics::World::debug_lines`), making declared-vs-simulated divergence
    /// visible at a glance. The overlay pass is wired natively only (the web
    /// renderer doesn't call `render_debug_lines` yet), so on wasm this
    /// shades like `Default`.
    Physics,
}

impl DebugRenderMode {
    /// The canonical lowercase label, used everywhere the mode crosses a text
    /// boundary: the `--debug-render` CLI flag, the `?debug-render=` URL query
    /// on wasm, the debug server's `/render-mode` body, and `/state`.
    pub fn label(&self) -> &'static str {
        match self {
            DebugRenderMode::Default => "default",
            DebugRenderMode::Normals => "normals",
            DebugRenderMode::Tangents => "tangents",
            DebugRenderMode::Physics => "physics",
        }
    }

    /// Parse a label back into a mode (case-insensitive). `None` for anything
    /// unrecognized, so callers can report it rather than silently defaulting.
    pub fn from_label(s: &str) -> Option<DebugRenderMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" => Some(DebugRenderMode::Default),
            "normals" => Some(DebugRenderMode::Normals),
            "tangents" => Some(DebugRenderMode::Tangents),
            "physics" => Some(DebugRenderMode::Physics),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DebugRenderMode;

    #[test]
    fn from_label_roundtrips_every_variant() {
        for mode in [
            DebugRenderMode::Default,
            DebugRenderMode::Normals,
            DebugRenderMode::Tangents,
            DebugRenderMode::Physics,
        ] {
            assert_eq!(DebugRenderMode::from_label(mode.label()), Some(mode));
        }
    }

    #[test]
    fn from_label_is_case_insensitive_and_trims() {
        assert_eq!(
            DebugRenderMode::from_label("  NORMALS "),
            Some(DebugRenderMode::Normals)
        );
    }

    #[test]
    fn from_label_rejects_unknown() {
        assert_eq!(DebugRenderMode::from_label("bogus"), None);
        assert_eq!(DebugRenderMode::from_label(""), None);
    }
}

pub struct RenderContext<'a> {
    pub gl: &'a glow::Context,
    pub shader_version: &'a str,
    pub asset_cache: Arc<AssetCache>,
    pub frame_time: FrameTime,
    pub debug_render_mode: DebugRenderMode,
    /// Frame-level lights (from `Frame.lights`), read by `LitMaterial`.
    pub lights: &'a [Light],
    /// Which pass is rendering (forward vs. a depth-only shadow pass).
    pub render_pass: RenderPass,
    /// The directional shadow map + light matrix, when shadows are active.
    /// `None` during the depth pass and when no light casts shadows.
    pub shadow: Option<ShadowUniforms>,
    /// Frame-level distance fog (from `Frame.fog`), applied by every forward
    /// material. `None` during depth passes and fog-less frames. The
    /// normals/tangents debug materials ignore it (no fog block in their
    /// shaders); the physics overlay mode shades normally, so fog applies.
    pub fog: Option<&'a Fog>,
    /// The pass camera's world position — frame-constant, computed once per
    /// pass rather than per draw (the fog shader blends by distance from it).
    pub camera_pos: Vector3<f32>,
    /// Stable center-camera data used only for terrain LOD selection.
    ///
    /// Stereo shells render the same [`Frame`](crate::Frame) twice with
    /// per-eye view cameras. Keeping terrain selection tied to one live
    /// tracked center pose makes both eyes draw the exact same patch set,
    /// avoiding binocular shimmer while preserving per-eye rasterization.
    pub lod_camera_pos: Vector3<f32>,
    /// World-to-clip matrices whose frusta are unioned for culling. Stereo
    /// shells provide both tracked eyes; mono passes use only element zero.
    pub lod_view_projections: [Matrix4<f32>; 2],
    pub lod_frustum_count: usize,
    /// Vertical projection scale (`cot(fov_y / 2)`), used to turn terrain
    /// world-space vertex spacing into projected pixels.
    pub lod_projection_scale: f32,
    pub viewport_height: f32,
}
