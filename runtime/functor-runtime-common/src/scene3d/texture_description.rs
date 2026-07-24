use serde::{Deserialize, Serialize};

use crate::render_target::RenderTargetDescriptor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TextureDescription {
    File(String),
    /// A file texture sampled with clamp-to-edge wrapping. Sprite images use
    /// this so linear filtering at UV 0/1 cannot bleed the opposite edge into
    /// transparent borders; ordinary model textures retain repeat wrapping.
    FileClamped(String),
    /// The texture of a named render target (the id only — its size lives on
    /// the frame's `RenderTargetPass` descriptor). Sampling an id no frame
    /// declares binds a magenta fallback and warns once. Vertical convention
    /// differs from `File`: FBO row 0 is the image *bottom* (right-side-up on
    /// a front-facing quad), while file textures upload top-row-first — the
    /// same UVs show a file texture and a render target flipped relative to
    /// each other.
    RenderTarget(String),
    /// A file texture with placeholder fallbacks tried WHILE IT LOADS
    /// (`Asset.whilePending`): the first loaded chain entry binds until
    /// `file` itself is ready; a FAILED `file` falls back like a plain
    /// `File` (failure is not pending — it must stay visible). Emitted only
    /// when a chain exists, so plain textures keep the `File` wire shape.
    FileWhilePending {
        file: String,
        while_pending: Vec<String>,
    },
    /// The clamp-to-edge sibling of `FileWhilePending`, used by sprites.
    FileClampedWhilePending {
        file: String,
        while_pending: Vec<String>,
    },
}

impl TextureDescription {
    pub fn render_target(rt: RenderTargetDescriptor) -> TextureDescription {
        TextureDescription::RenderTarget(rt.id)
    }
}
