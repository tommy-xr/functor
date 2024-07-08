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

impl RenderableAsset for TextureData {
    type HydratedType = glow::Texture;
    type OptionsType = TextureOptions;

    fn hydrate(
        &self,
        gl_context: &glow::Context,
        options: &Self::OptionsType,
    ) -> Self::HydratedType {
        unsafe {
            let gl = gl_context;
            let texture = gl.create_texture().expect("Texture to be created");
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));

            // Set texture parameters
            let wrap_val = if options.wrap {
                glow::REPEAT
            } else {
                glow::CLAMP_TO_EDGE
            };

            let filter = if options.linear {
                glow::LINEAR
            } else {
                glow::NEAREST
            };

            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap_val as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap_val as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);

            let format = match self.format {
                PixelFormat::RGB => glow::RGB,
                PixelFormat::RGBA => glow::RGBA,
            };

            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                self.width as i32,
                self.height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                Some(&self.bytes),
            );

            gl.bind_texture(glow::TEXTURE_2D, None);
            texture
        }
    }
}
