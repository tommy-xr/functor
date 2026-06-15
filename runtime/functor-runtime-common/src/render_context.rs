use std::sync::Arc;

use crate::{asset::AssetCache, FrameTime};

/// Global override for how the scene is shaded — a debug aid, not a per-material
/// choice. `Default` uses each node's own material; the others replace it with a
/// diagnostic shader across the whole frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugRenderMode {
    #[default]
    Default,
    /// Visualize world-space surface normals as RGB (`normal * 0.5 + 0.5`).
    Normals,
}

pub struct RenderContext<'a> {
    pub gl: &'a glow::Context,
    pub shader_version: &'a str,
    pub asset_cache: Arc<AssetCache>,
    pub frame_time: FrameTime,
    pub debug_render_mode: DebugRenderMode,
}
