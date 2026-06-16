use std::sync::Arc;

use crate::{asset::AssetCache, FrameTime, Light};

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

impl DebugRenderMode {
    /// The canonical lowercase label, used everywhere the mode crosses a text
    /// boundary: the `--debug-render` CLI flag, the `?debug-render=` URL query
    /// on wasm, the debug server's `/render-mode` body, and `/state`.
    pub fn label(&self) -> &'static str {
        match self {
            DebugRenderMode::Default => "default",
            DebugRenderMode::Normals => "normals",
        }
    }

    /// Parse a label back into a mode (case-insensitive). `None` for anything
    /// unrecognized, so callers can report it rather than silently defaulting.
    pub fn from_label(s: &str) -> Option<DebugRenderMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" => Some(DebugRenderMode::Default),
            "normals" => Some(DebugRenderMode::Normals),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DebugRenderMode;

    #[test]
    fn from_label_roundtrips_every_variant() {
        for mode in [DebugRenderMode::Default, DebugRenderMode::Normals] {
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
}
