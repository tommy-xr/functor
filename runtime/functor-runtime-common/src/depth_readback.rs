//! The environment depth read back to the CPU every [`READBACK_EVERY`]
//! frames, for the fused room and the depth stream.
//!
//! The readback never stalls the render thread: a pass packs the depth layer
//! into an RGBA8 target, `glReadPixels` copies it into a pixel-pack buffer,
//! and a later frame maps the buffer once its fence has signalled. What the
//! readback read (its view, its time) waits beside the fence, so pixels never
//! pair with a newer pose.

use glow::HasContext;

use crate::environment_depth::EnvironmentDepthView;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};

/// Frames between readbacks: 30 a second at 120 Hz.
pub const READBACK_EVERY: u64 = 4;
/// The readback's size: the headset's depth image, and desktop's stand-in
/// depth downsampled to it.
pub const READBACK_SIZE: u32 = 320;

/// Packs one depth layer into RGBA8: 24-bit window depth in RGB, most
/// significant byte first ([`crate::fused_mesh::DepthFrame::from_rgba8`]
/// unpacks it).
const PACK_FRAGMENT_SHADER_SOURCE: &str = r#"
        in vec2 vUv;
        out vec4 fragColor;

        uniform highp sampler2DArray depthTexture;
        uniform float depthLayer;

        void main() {
            highp float d = textureLod(depthTexture, vec3(vUv, depthLayer), 0.0).r;
            // At d = 1 the sum rounds to 2^24 in float32: clamp, or "no
            // surface" would pack as FF0000.
            highp float v = min(floor(clamp(d, 0.0, 1.0) * 16777215.0 + 0.5), 16777215.0);
            highp float hi = floor(v / 65536.0);
            highp float mid = floor((v - hi * 65536.0) / 256.0);
            highp float lo = v - hi * 65536.0 - mid * 256.0;
            fragColor = vec4(hi, mid, lo, 255.0) / 255.0;
        }
"#;

/// GL objects for packing and reading back one depth layer, and the readback
/// in flight with its metadata `M`.
pub(crate) struct DepthReadback<M> {
    fbo: glow::Framebuffer,
    color: glow::Renderbuffer,
    pack: PixelPack<M>,
    program: ShaderProgram,
    depth_texture_loc: UniformLocation,
    depth_layer_loc: UniformLocation,
    /// The frame of the last pass asked; only a frame's first pass starts.
    last_pass: Option<u64>,
}

/// A `width`×`height` RGBA8 framebuffer read into a pixel-pack buffer without
/// stalling: [`Self::start`] queues the copy behind a fence, and a later
/// [`Self::poll`] maps the buffer once the fence has signalled. The pixels'
/// metadata `M` waits beside the fence.
pub(crate) struct PixelPack<M> {
    pbo: glow::Buffer,
    width: u32,
    height: u32,
    pending: Option<(glow::Fence, M)>,
}

impl<M> PixelPack<M> {
    pub(crate) fn new(gl: &glow::Context, width: u32, height: u32) -> PixelPack<M> {
        unsafe {
            let pbo = gl.create_buffer().expect("pixel pack buffer");
            crate::gpu_counters::gpu_counters().buffer_created();
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(pbo));
            gl.buffer_data_size(
                glow::PIXEL_PACK_BUFFER,
                (width * height * 4) as i32,
                glow::STREAM_READ,
            );
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            PixelPack {
                pbo,
                width,
                height,
                pending: None,
            }
        }
    }

    pub(crate) fn delete(self, gl: &glow::Context) {
        unsafe {
            if let Some((fence, _)) = self.pending {
                gl.delete_sync(fence);
            }
            gl.delete_buffer(self.pbo);
        }
        crate::gpu_counters::gpu_counters().buffer_deleted();
    }

    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Copy the bound read framebuffer's bottom-left `width`×`height` into
    /// the buffer, behind a fence; `meta` comes back with the pixels.
    pub(crate) fn start(&mut self, gl: &glow::Context, meta: M) {
        let fence = unsafe {
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(self.pbo));
            gl.read_pixels(
                0,
                0,
                self.width as i32,
                self.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::BufferOffset(0),
            );
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0)
                .expect("pixel pack fence")
        };
        self.pending = Some((fence, meta));
    }

    /// The pixels in flight (rows bottom-up) and their metadata once the
    /// fence has signalled — at once when `wait` (mapping then waits for the
    /// copy). A failed wait or map drops them.
    pub(crate) fn poll(&mut self, gl: &glow::Context, wait: bool) -> Option<(Vec<u8>, M)> {
        let (fence, meta) = self.pending.take()?;
        let status = if wait {
            glow::ALREADY_SIGNALED
        } else {
            unsafe { gl.client_wait_sync(fence, 0, 0) }
        };
        if status == glow::TIMEOUT_EXPIRED {
            self.pending = Some((fence, meta));
            return None;
        }
        let rgba = (status != glow::WAIT_FAILED).then(|| self.read(gl)).flatten();
        unsafe { gl.delete_sync(fence) };
        rgba.map(|rgba| (rgba, meta))
    }

    /// The pixel-pack buffer's bytes; `None` if it could not be mapped.
    fn read(&self, gl: &glow::Context) -> Option<Vec<u8>> {
        let len = (self.width * self.height * 4) as usize;
        let mut rgba = Some(vec![0; len]);
        unsafe {
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(self.pbo));
            // WebGL 2 has no mapping; its read is a copy.
            #[cfg(target_arch = "wasm32")]
            if let Some(rgba) = &mut rgba {
                gl.get_buffer_sub_data(glow::PIXEL_PACK_BUFFER, 0, rgba);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mapped =
                    gl.map_buffer_range(glow::PIXEL_PACK_BUFFER, 0, len as i32, glow::MAP_READ_BIT);
                match rgba.as_mut().filter(|_| !mapped.is_null()) {
                    Some(rgba) => {
                        rgba.copy_from_slice(std::slice::from_raw_parts(mapped, len));
                        gl.unmap_buffer(glow::PIXEL_PACK_BUFFER);
                    }
                    None => rgba = None,
                }
            }
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
        }
        rgba
    }
}

impl<M> DepthReadback<M> {
    pub(crate) fn new(gl: &glow::Context, shader_version: &str) -> DepthReadback<M> {
        let vertex = Shader::build(
            gl,
            ShaderType::Vertex,
            crate::composite::COMPOSITE_VERTEX_SHADER_SOURCE,
            shader_version,
        );
        let fragment = Shader::build(
            gl,
            ShaderType::Fragment,
            PACK_FRAGMENT_SHADER_SOURCE,
            shader_version,
        );
        let program = ShaderProgram::link(gl, &vertex, &fragment);
        let size = READBACK_SIZE as i32;
        unsafe {
            let color = gl.create_renderbuffer().expect("depth readback color");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, size, size);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            let previous = crate::renderer::ambient_framebuffer(gl);
            let fbo = gl.create_framebuffer().expect("depth readback fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(color),
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
            DepthReadback {
                fbo,
                color,
                pack: PixelPack::new(gl, READBACK_SIZE, READBACK_SIZE),
                depth_texture_loc: program.get_uniform_location(gl, "depthTexture"),
                depth_layer_loc: program.get_uniform_location(gl, "depthLayer"),
                program,
                last_pass: None,
            }
        }
    }

    pub(crate) fn delete(self, gl: &glow::Context) {
        self.pack.delete(gl);
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_renderbuffer(self.color);
            gl.delete_program(self.program.program_id);
        }
    }

    /// Whether this pass may start a readback: the first pass of a readback
    /// frame (the left eye of a stereo pair), with none in flight. Call once
    /// per pass.
    pub(crate) fn can_start(&mut self, frame: u64) -> bool {
        let first_pass = self.last_pass != Some(frame);
        self.last_pass = Some(frame);
        first_pass && frame % READBACK_EVERY == 0 && !self.pack.busy()
    }

    /// Pack `view`'s layer and start copying it into the pixel-pack buffer;
    /// `meta` comes back with its pixels from [`Self::poll`]. Restores the
    /// framebuffer; the caller resets the viewport.
    pub(crate) fn start(
        &mut self,
        gl: &glow::Context,
        quad: &mut dyn crate::geometry::Geometry,
        view: EnvironmentDepthView,
        meta: M,
    ) {
        let size = READBACK_SIZE as i32;
        unsafe {
            let previous = crate::renderer::ambient_framebuffer(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, size, size);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            self.program.use_program(gl);
            self.program.set_uniform_1i(gl, &self.depth_texture_loc, 0);
            self.program
                .set_uniform_1f(gl, &self.depth_layer_loc, view.layer as f32);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(view.texture));
            quad.draw(gl);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            self.pack.start(gl, meta);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
        }
    }

    /// The readback in flight's pixels and metadata ([`PixelPack::poll`]).
    pub(crate) fn poll(&mut self, gl: &glow::Context, wait: bool) -> Option<(Vec<u8>, M)> {
        self.pack.poll(gl, wait)
    }
}
