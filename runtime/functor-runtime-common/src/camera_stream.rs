//! `Xr.streamCamera` on a shell without a passthrough camera (desktop): the
//! stand-in room rendered from the view through a known pinhole camera, read
//! back without stalling, and streamed as `emulated` camera frames.

use cgmath::{Matrix4, SquareMatrix, Vector4};
use glow::HasContext;

use crate::camera_frame::{
    self, CameraFrameHeader, CameraFrameSource, CameraFrameTime, Intrinsics,
};
use crate::depth_frame::columns;
use crate::depth_readback::PixelPack;
use crate::sensor_stream::{FrameIds, SensorStreamer, StreamCapture};

pub const EMULATED_WIDTH: u32 = 640;
pub const EMULATED_HEIGHT: u32 = 480;
/// About 65° across, principal point at the image centre.
pub const EMULATED_INTRINSICS: Intrinsics = Intrinsics {
    fx: 500.0,
    fy: 500.0,
    cx: 320.0,
    cy: 240.0,
};
const NEAR: f32 = 0.05;
const FAR: f32 = 100.0;

/// OpenCV camera axes (x right, y down, z forward) → GL's (y up, z back).
pub(crate) fn cv_to_gl() -> Matrix4<f32> {
    Matrix4::from_diagonal(Vector4::new(1.0, -1.0, -1.0, 1.0))
}

/// What one readback shows, waiting beside its fence.
struct Shot {
    capture: StreamCapture,
    camera_to_world: Matrix4<f32>,
    space_epoch: u64,
}

/// The offscreen target the stand-in view renders into, and its readback.
pub(crate) struct EmulatedCamera {
    fbo: glow::Framebuffer,
    color: glow::Renderbuffer,
    depth: glow::Renderbuffer,
    pack: PixelPack<Shot>,
}

impl EmulatedCamera {
    pub(crate) fn new(gl: &glow::Context) -> EmulatedCamera {
        let (w, h) = (EMULATED_WIDTH as i32, EMULATED_HEIGHT as i32);
        unsafe {
            let previous = crate::renderer::ambient_framebuffer(gl);
            let storage = |format| {
                let buffer = gl
                    .create_renderbuffer()
                    .expect("emulated camera renderbuffer");
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(buffer));
                gl.renderbuffer_storage(glow::RENDERBUFFER, format, w, h);
                buffer
            };
            let color = storage(glow::RGBA8);
            let depth = storage(glow::DEPTH_COMPONENT24);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            let fbo = gl.create_framebuffer().expect("emulated camera fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            for (attachment, buffer) in [
                (glow::COLOR_ATTACHMENT0, color),
                (glow::DEPTH_ATTACHMENT, depth),
            ] {
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    attachment,
                    glow::RENDERBUFFER,
                    Some(buffer),
                );
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
            EmulatedCamera {
                fbo,
                color,
                depth,
                pack: PixelPack::new(gl, EMULATED_WIDTH, EMULATED_HEIGHT),
            }
        }
    }

    pub(crate) fn delete(self, gl: &glow::Context) {
        self.pack.delete(gl);
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_renderbuffer(self.color);
            gl.delete_renderbuffer(self.depth);
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.pack.busy()
    }

    /// Send the shot that finished, if one has.
    pub(crate) fn poll(&mut self, gl: &glow::Context, stream: &mut SensorStreamer) {
        if let Some((rgba, shot)) = self.pack.poll(gl, false) {
            let payload =
                move || camera_frame::rgba_to_i420(&rgba, EMULATED_WIDTH, EMULATED_HEIGHT, true);
            stream.send(shot.capture, payload, |ids| {
                serde_json::to_vec(&header(&shot, ids)).expect("a camera frame header serializes")
            });
        }
    }

    /// Render `capture`'s shot from `view` (world → GL camera) with
    /// `render(projection)` into this target and start reading it back.
    /// Restores the framebuffer, viewport and scissor test.
    pub(crate) fn start(
        &mut self,
        gl: &glow::Context,
        capture: StreamCapture,
        view: Matrix4<f32>,
        space_epoch: u64,
        render: impl FnOnce(&Matrix4<f32>),
    ) {
        let projection =
            EMULATED_INTRINSICS.gl_projection(EMULATED_WIDTH, EMULATED_HEIGHT, NEAR, FAR);
        unsafe {
            let previous = crate::renderer::ambient_framebuffer(gl);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let scissor = gl.is_enabled(glow::SCISSOR_TEST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            render(&projection);
            self.pack.start(
                gl,
                Shot {
                    capture,
                    camera_to_world: view.invert().unwrap_or(Matrix4::identity()) * cv_to_gl(),
                    space_epoch,
                },
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
            gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            if scissor {
                gl.enable(glow::SCISSOR_TEST);
            }
        }
    }
}

fn header(shot: &Shot, ids: FrameIds) -> CameraFrameHeader {
    CameraFrameHeader {
        schema: camera_frame::SCHEMA.to_string(),
        stream: ids.stream,
        seq: ids.seq,
        source: CameraFrameSource::Emulated,
        camera: "left".to_string(),
        width: EMULATED_WIDTH,
        height: EMULATED_HEIGHT,
        encoding: camera_frame::ENCODING.to_string(),
        intrinsics: EMULATED_INTRINSICS,
        distortion: None,
        camera_to_world: columns(shot.camera_to_world),
        // The emulated camera sits at the view.
        camera_to_head: columns(cv_to_gl()),
        time: CameraFrameTime {
            clock: "steady".to_string(),
            exposure_ns: shot.capture.steady_ns,
            sensor_ns: None,
        },
        tracking: true,
        space_epoch: shot.space_epoch,
        payload_len: camera_frame::i420_len(EMULATED_WIDTH, EMULATED_HEIGHT),
    }
}
