use std::cell::RefCell;

use glow::HasContext;

use crate::RenderContext;

use super::{PixelFormat, RuntimeTexture, TextureData};

pub struct Texture2D {
    state: RefCell<Option<TextureState>>,
}

#[derive(Default)]
pub struct TextureOptions {
    pub wrap: bool,
    pub linear: bool,
}

impl Texture2D {
    pub fn init_from_data(data: TextureData, opts: TextureOptions) -> Texture2D {
        Texture2D {
            state: RefCell::new(Some(TextureState::Unloaded(data, opts))),
        }
    }
}

impl RuntimeTexture for Texture2D {
    fn bind(&self, index: u32, render_context: &RenderContext) {
        let mut state = self.state.borrow_mut();
        if let Some(texture_state) = state.take() {
            let new_state = texture_state.ensure_loaded(render_context);

            match new_state {
                TextureState::Loaded(tex) => unsafe {
                    let gl = render_context.gl;
                    gl.active_texture(glow::TEXTURE0 + index);
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                },
                TextureState::Unloaded(_, _) => {
                    panic!("Unable to load texture; should never happen")
                }
            }

            *state = Some(new_state);
        }
    }
}

pub enum TextureState {
    Unloaded(TextureData, TextureOptions),
    Loaded(glow::Texture),
}

impl TextureState {
    pub fn ensure_loaded(self, render_context: &RenderContext) -> Self {
        match self {
            TextureState::Loaded(_) => self,
            TextureState::Unloaded(texture_data, texture_opts) => unsafe {
                let gl = render_context.gl;
                let texture = gl.create_texture().expect("Texture to be created");
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));

                // Set texture parameters
                let wrap_val = if texture_opts.wrap {
                    glow::REPEAT
                } else {
                    glow::CLAMP_TO_EDGE
                };

                let filter = if texture_opts.linear {
                    glow::LINEAR
                } else {
                    glow::NEAREST
                };

                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap_val as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap_val as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);

                let format = match texture_data.format {
                    PixelFormat::RGB => glow::RGB,
                    PixelFormat::RGBA => glow::RGBA,
                };

                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    texture_data.width as i32,
                    texture_data.height as i32,
                    0,
                    format,
                    glow::UNSIGNED_BYTE,
                    Some(&texture_data.bytes),
                );

                Self::Loaded(texture)
            },
        }
    }
}
