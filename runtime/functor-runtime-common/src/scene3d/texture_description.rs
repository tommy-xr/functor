use fable_library_rust::String_::LrcStr;
use serde::{Deserialize, Serialize};

use crate::render_target::RenderTargetDescriptor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TextureDescription {
    File(String),
    /// The texture of a named render target (the id only — its size lives on
    /// the frame's `RenderTargetPass` descriptor). Sampling an id no frame
    /// declares binds a magenta fallback and warns once. Vertical convention
    /// differs from `File`: FBO row 0 is the image *bottom* (right-side-up on
    /// a front-facing quad), while file textures upload top-row-first — the
    /// same UVs show a file texture and a render target flipped relative to
    /// each other.
    RenderTarget(String),
}

impl TextureDescription {
    pub fn file(s: LrcStr) -> TextureDescription {
        TextureDescription::File(s.to_string())
    }

    pub fn render_target(rt: RenderTargetDescriptor) -> TextureDescription {
        TextureDescription::RenderTarget(rt.id)
    }
}
