use glow::HasContext;
use serde::{Deserialize, Serialize};

pub const DEFAULT_RENDER_TARGET_SIZE: u32 = 512;

/// A named offscreen render target, declared once by the game and used at both
/// sites: the writer (`Frame::with_render_target`, which renders an inner frame
/// into it each frame) and the reader (`TextureDescription::RenderTarget`,
/// sampling it from a material). The name is the cross-frame identity — the
/// runtime keys its GPU buffers by it, so it survives hot reloads and frame
/// rebuilds. RGBA8 only for now (the one unconditionally WebGL2-safe format).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderTargetDescriptor {
    pub id: String,
    pub width: u32,
    pub height: u32,
}

impl RenderTargetDescriptor {
    pub fn new(id: impl Into<String>) -> RenderTargetDescriptor {
        RenderTargetDescriptor {
            id: id.into(),
            width: DEFAULT_RENDER_TARGET_SIZE,
            height: DEFAULT_RENDER_TARGET_SIZE,
        }
    }

    /// Subject-first so it pipes (`rt |> RenderTarget.sized(w, h)`); sizes are
    /// f32 at the boundary (Functor Lang numbers are floats), clamped to at least 1.
    pub fn sized(self, width: f32, height: f32) -> RenderTargetDescriptor {
        RenderTargetDescriptor {
            width: width.max(1.0) as u32,
            height: height.max(1.0) as u32,
            ..self
        }
    }
}

/// The GPU side of a render target: a double-buffered FBO pair (own RGBA8 color
/// texture each, one shared depth renderbuffer). Readers sample the *read*
/// texture while the pass writes the other one, then the pair swaps — so a
/// scene that samples its own target (a monitor visible to its own camera) sees
/// last frame's image instead of a GL feedback loop. The texture recipe follows
/// `ShadowMap::new` (RGBA8 + single declared mip level for macOS completeness),
/// but with LINEAR filtering — this is an albedo, not a depth map.
pub struct RenderTargetBuffers {
    fbos: [glow::Framebuffer; 2],
    textures: [glow::Texture; 2],
    depth_rbo: glow::Renderbuffer,
    read_index: usize,
    pub width: u32,
    pub height: u32,
}

impl RenderTargetBuffers {
    pub fn new(
        gl: &glow::Context,
        width: u32,
        height: u32,
        clear: [f32; 3],
    ) -> RenderTargetBuffers {
        let width = width.max(1);
        let height = height.max(1);
        unsafe {
            let depth_rbo = gl.create_renderbuffer().expect("render target depth rbo");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_rbo));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                width as i32,
                height as i32,
            );

            let mut fbos = [None; 2];
            let mut textures = [None; 2];
            for i in 0..2 {
                let texture = gl.create_texture().expect("render target texture");
                crate::gpu_counters::gpu_counters().texture_created();
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    width as i32,
                    height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_BASE_LEVEL, 0);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, 0);
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );

                let fbo = gl.create_framebuffer().expect("render target fbo");
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(depth_rbo),
                );

                // Clear to the pass's background (the fog color when the
                // target frame declares fog) so a read before the first write
                // (frame 1 of a self-viewing target) shows the clear color,
                // never uninitialized memory.
                gl.disable(glow::SCISSOR_TEST);
                gl.viewport(0, 0, width as i32, height as i32);
                gl.clear_color(clear[0], clear[1], clear[2], 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

                fbos[i] = Some(fbo);
                textures[i] = Some(texture);
            }

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);

            RenderTargetBuffers {
                fbos: [fbos[0].unwrap(), fbos[1].unwrap()],
                textures: [textures[0].unwrap(), textures[1].unwrap()],
                depth_rbo,
                read_index: 0,
                width,
                height,
            }
        }
    }

    /// The texture materials sample this frame (last completed write).
    pub fn read_texture(&self) -> glow::Texture {
        self.textures[self.read_index]
    }

    /// The framebuffer the current target pass renders into.
    pub fn write_fbo(&self) -> glow::Framebuffer {
        self.fbos[1 - self.read_index]
    }

    /// Publish the just-written buffer: subsequent reads sample it.
    pub fn swap(&mut self) {
        self.read_index = 1 - self.read_index;
    }

    /// Free the GL objects (used when a target is recreated at a new size).
    pub fn delete(&self, gl: &glow::Context) {
        unsafe {
            for fbo in self.fbos {
                gl.delete_framebuffer(fbo);
            }
            for texture in self.textures {
                gl.delete_texture(texture);
                crate::gpu_counters::gpu_counters().texture_deleted();
            }
            gl.delete_renderbuffer(self.depth_rbo);
        }
    }
}

/// A warning line visible on every target (`eprintln!` has no console on wasm).
#[cfg(target_arch = "wasm32")]
pub(crate) fn warn_line(message: &str) {
    web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(message));
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn warn_line(message: &str) {
    eprintln!("{}", message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_to_512() {
        let rt = RenderTargetDescriptor::new("feed");
        assert_eq!(rt.id, "feed");
        assert_eq!((rt.width, rt.height), (512, 512));
    }

    #[test]
    fn sized_truncates_and_clamps_to_one() {
        let rt = RenderTargetDescriptor::new("feed").sized(256.9, 128.0);
        assert_eq!((rt.width, rt.height), (256, 128));
        let rt = RenderTargetDescriptor::new("feed").sized(-4.0, 0.0);
        assert_eq!((rt.width, rt.height), (1, 1));
    }
}
