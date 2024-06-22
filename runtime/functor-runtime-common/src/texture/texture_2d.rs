use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

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

    pub fn init_from_future<F>(future: F, opts: TextureOptions) -> Texture2D
    where
        F: Future<Output = Result<TextureData, String>> + 'static,
    {
        Texture2D {
            state: RefCell::new(Some(TextureState::Loading(Box::pin(future), opts))),
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
                TextureState::Loading(_, _) => {
                    // TODO: Have a default texture to use
                    // While loading, we can't actually do anything
                    println!("Still waiting for texture loading...")
                }
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
    Loading(
        Pin<Box<dyn Future<Output = Result<TextureData, String>>>>,
        TextureOptions,
    ),
    Loaded(glow::Texture),
}

impl TextureState {
    pub fn ensure_loaded(self, render_context: &RenderContext) -> Self {
        match self {
            TextureState::Loaded(_) => self,
            TextureState::Loading(..) => self.poll_load(render_context),
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

                gl.bind_texture(glow::TEXTURE_2D, None);
                Self::Loaded(texture)
            },
        }
    }

    fn poll_load(self, render_context: &RenderContext) -> Self {
        if let Self::Loading(mut future, opts) = self {
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            match Future::poll(Pin::new(&mut future), &mut cx) {
                Poll::Ready(Ok(texture_data)) => {
                    TextureState::Unloaded(texture_data, opts).ensure_loaded(render_context)
                }
                Poll::Ready(Err(e)) => {
                    // TODO: More robust error handling...
                    panic!("Failed to load texture: {}", e);
                }
                Poll::Pending => {
                    println!("Waiting for texture to load...");
                    TextureState::Loading(future, opts)
                }
            }
        } else {
            self
        }
    }
}
