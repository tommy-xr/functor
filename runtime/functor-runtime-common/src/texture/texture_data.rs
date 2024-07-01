use glow::HasContext;

use crate::asset::RenderableAsset;

use super::{PixelFormat, Texture2D, TextureOptions};

#[derive(Clone)]
pub struct TextureData {
    pub bytes: std::vec::Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

impl TextureData {
    pub fn checkerboard_pattern(width: u32, height: u32, color: [u8; 4]) -> TextureData {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);

        for y in 0..height {
            for x in 0..width {
                let is_white = (x + y) % 2 == 0;
                let color = if is_white {
                    color
                } else {
                    [0, 0, 0, 255] // Black with full opacity
                };

                bytes.extend_from_slice(&color);
            }
        }

        TextureData {
            bytes,
            width,
            height,
            format: PixelFormat::RGBA,
        }
    }
}

impl RenderableAsset for TextureData {
    type HydratedType = glow::Texture;
    type OptionsType = TextureOptions;

    fn load(&self, gl_context: &glow::Context, options: &Self::OptionsType) -> Self::HydratedType {
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
