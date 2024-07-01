use std::cell::RefCell;

use glow::HasContext;

use crate::{
    asset::{RenderableAsset, RuntimeRenderableAsset},
    RenderContext,
};

use super::{PixelFormat, RuntimeTexture, TextureData};

pub struct Texture2D {
    ora: RuntimeRenderableAsset<TextureData>,
}

#[derive(Default)]
pub struct TextureOptions {
    pub wrap: bool,
    pub linear: bool,
}

impl Texture2D {
    pub fn init_from_data(data: TextureData, opts: TextureOptions) -> Texture2D {
        Texture2D {
            ora: RuntimeRenderableAsset::new(data, opts),
        }
    }
}

impl RuntimeTexture for Texture2D {
    fn bind(&self, index: u32, render_context: &RenderContext) {
        let texture = self.ora.get(render_context.gl);
        let gl = render_context.gl;
        unsafe {
            gl.active_texture(glow::TEXTURE0 + index);
            gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
        }
    }
}
