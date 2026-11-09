//! The shell's hot-reload indicator: a small Functor mark in a corner of the
//! view that spins while a pushed reload (`/reload-source`,
//! `/reload-project`) is in flight, then holds green (swapped in) or red
//! (rejected) for [`HOLD`]. Shell-owned: it never reaches the game's model,
//! frames, or history, and draws over the finished frame.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use cgmath::{Matrix4, Quaternion, Rad, SquareMatrix};
use glow::HasContext;
use serde::{Deserialize, Serialize};

use crate::TrackingPose;

/// How long the outcome shows after a reload settles.
pub const HOLD: Duration = Duration::from_millis(500);

/// Turns per second while loading.
const SPIN_HZ: f32 = 1.0;

type Reply = Result<String, String>;

/// What the indicator draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ReloadShown {
    Loading,
    Ok,
    Failed,
}

impl ReloadShown {
    /// Authored sRGB: the site's cyan mark while loading, then green or red.
    pub fn color(self) -> [f32; 3] {
        match self {
            ReloadShown::Loading => [0.255, 0.847, 0.902],
            ReloadShown::Ok => [0.24, 0.86, 0.42],
            ReloadShown::Failed => [0.95, 0.25, 0.25],
        }
    }
}

/// How the last pushed reload settled.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReloadOutcome {
    /// False when the reload was rejected (the old program keeps running) or
    /// superseded by a newer push.
    pub ok: bool,
    /// From the push reaching the shell to its answer.
    pub ms: f64,
    /// `/state.rendered_frames` when it settled.
    pub rendered_frame: u64,
}

/// `/state.reload`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReloadState {
    /// A pushed reload has not answered yet.
    pub in_progress: bool,
    /// What the indicator draws now; omitted when hidden.
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<ReloadShown>,
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<ReloadOutcome>,
}

struct InFlight {
    started: Instant,
    answer: Receiver<Reply>,
    reply: Sender<Reply>,
}

/// Pushed reloads in flight and the last outcome.
#[derive(Default)]
pub struct ReloadIndicator {
    in_flight: Vec<InFlight>,
    last: Option<(ReloadOutcome, Instant)>,
}

impl ReloadIndicator {
    /// Track a push answered on `reply`: the producer answers on the returned
    /// sender instead, and [`Self::poll`] forwards it.
    pub fn track(&mut self, reply: Sender<Reply>, now: Instant) -> Sender<Reply> {
        let (tx, answer) = mpsc::channel();
        self.in_flight.push(InFlight { started: now, answer, reply });
        tx
    }

    /// Forward answered pushes to their pushers, recording each outcome. Call
    /// once per shell loop, before reading [`Self::state`].
    pub fn poll(&mut self, now: Instant, rendered_frames: u64) {
        let mut last = self.last;
        self.in_flight.retain(|push| {
            let result = match push.answer.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => Err("the reload was dropped unanswered".to_string()),
            };
            let outcome = ReloadOutcome {
                ok: result.is_ok(),
                ms: (now - push.started).as_secs_f64() * 1000.0,
                rendered_frame: rendered_frames,
            };
            last = Some((outcome, now));
            let _ = push.reply.send(result);
            false
        });
        self.last = last;
    }

    /// What to draw at `now`, and the spin in radians.
    pub fn look(&self, now: Instant) -> Option<(ReloadShown, f32)> {
        if let Some(oldest) = self.in_flight.first() {
            let turns = (now - oldest.started).as_secs_f32() * SPIN_HZ;
            return Some((ReloadShown::Loading, turns.fract() * std::f32::consts::TAU));
        }
        let (outcome, at) = self.last?;
        (now - at < HOLD).then_some((if outcome.ok { ReloadShown::Ok } else { ReloadShown::Failed }, 0.0))
    }

    pub fn state(&self, now: Instant) -> ReloadState {
        ReloadState {
            in_progress: !self.in_flight.is_empty(),
            shown: self.look(now).map(|(shown, _)| shown),
            last: self.last.map(|(outcome, _)| outcome),
        }
    }
}

/// The mark from `docs/media/functor-icon.svg` (viewBox 0..1024, y down) as
/// triangles: crown, stem, crossbar.
const MARK_SVG: [[f32; 2]; 24] = [
    // Crown (concave hexagon, four triangles).
    [382.0, 341.0], [545.0, 179.0], [548.0, 300.0],
    [545.0, 179.0], [735.0, 179.0], [615.0, 300.0],
    [545.0, 179.0], [615.0, 300.0], [548.0, 300.0],
    [382.0, 341.0], [548.0, 300.0], [382.0, 466.0],
    // Stem.
    [382.0, 493.0], [497.0, 380.0], [497.0, 748.0],
    [382.0, 493.0], [497.0, 748.0], [382.0, 837.0],
    // Crossbar.
    [518.0, 426.0], [693.0, 426.0], [597.0, 522.0],
    [518.0, 426.0], [597.0, 522.0], [518.0, 522.0],
];

/// The mark's vertices: unit height, centered, y up.
fn mark_vertices() -> Vec<f32> {
    let (cx, cy, height) = ((382.0 + 735.0) / 2.0, (179.0 + 837.0) / 2.0, 837.0 - 179.0);
    MARK_SVG
        .iter()
        .flat_map(|[x, y]| [(x - cx) / height, (cy - y) / height])
        .collect()
}

/// Lower-left corner of a `width`×`height` framebuffer: 8% of the shorter
/// side tall, half that from the edges.
pub fn corner_transform(width: u32, height: u32, spin: f32) -> Matrix4<f32> {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    let size = 0.08 * w.min(h);
    // Half its height plus a half-height margin.
    let center = size;
    // Flattened (z scale 0): the spin only narrows the mark, like a coin.
    Matrix4::from_translation(cgmath::vec3(2.0 * center / w - 1.0, 2.0 * center / h - 1.0, 0.0))
        * Matrix4::from_nonuniform_scale(2.0 * size / w, 2.0 * size / h, 0.0)
        * Matrix4::from_angle_y(Rad(spin))
}

/// Head-locked for one eye: 5 cm tall, 60 cm ahead, low and to the left of
/// the head's gaze. `fov` is the eye's `[left, right, down, up]` angles in
/// radians; `eye_in_head` its pose relative to the head center, so both eyes
/// agree on where the mark is (correct stereo depth).
pub fn head_locked_transform(fov: [f32; 4], eye_in_head: TrackingPose, spin: f32) -> Matrix4<f32> {
    // Its own near plane: the game camera's may sit beyond 60 cm.
    let [left, right, down, up] = fov;
    let projection = crate::Camera { near: 0.05, far: 10.0, ..Default::default() }
        .projection_matrix_from_fov_angles(left, right, down, up);
    let [x, y, z, w] = eye_in_head.orientation;
    let eye = Matrix4::from_translation(eye_in_head.position.into()) * Matrix4::from(Quaternion::new(w, x, y, z));
    let head_from_eye = eye.invert().unwrap_or(Matrix4::identity());
    projection
        * head_from_eye
        * Matrix4::from_translation(cgmath::vec3(-0.15, -0.12, -0.6))
        * Matrix4::from_scale(0.05)
        * Matrix4::from_angle_y(Rad(spin))
}

/// The indicator's GL program and mark mesh, built once per context.
pub struct ReloadIndicatorPainter {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    vertices: i32,
}

impl ReloadIndicatorPainter {
    pub fn new(gl: &glow::Context, shader_version: &str) -> Self {
        let vertex = format!(
            "{shader_version}\n\
             layout(location = 0) in vec2 a_pos;\n\
             uniform mat4 u_transform;\n\
             void main() {{ gl_Position = u_transform * vec4(a_pos, 0.0, 1.0); }}"
        );
        let fragment = format!(
            "{shader_version}\n\
             precision mediump float;\n\
             {encode}\n\
             uniform vec3 u_color;\n\
             out vec4 frag_color;\n\
             void main() {{ frag_color = functorOutput(vec4(u_color, 1.0)); }}",
            encode = crate::color_space::OUTPUT_ENCODE_GLSL,
        );
        let vertices = mark_vertices();
        unsafe {
            let program = gl.create_program().expect("create reload indicator program");
            let mut shaders = Vec::new();
            for (kind, source) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
                let shader = gl.create_shader(kind).expect("create reload indicator shader");
                gl.shader_source(shader, &source);
                gl.compile_shader(shader);
                assert!(
                    gl.get_shader_compile_status(shader),
                    "reload indicator shader failed to compile: {}",
                    gl.get_shader_info_log(shader)
                );
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.link_program(program);
            assert!(
                gl.get_program_link_status(program),
                "reload indicator program failed to link: {}",
                gl.get_program_info_log(program)
            );
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }
            let vao = gl.create_vertex_array().expect("create reload indicator vao");
            gl.bind_vertex_array(Some(vao));
            let vbo = gl.create_buffer().expect("create reload indicator vbo");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let bytes = core::slice::from_raw_parts(
                vertices.as_ptr() as *const u8,
                std::mem::size_of_val(vertices.as_slice()),
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            ReloadIndicatorPainter { program, vao, vbo, vertices: (vertices.len() / 2) as i32 }
        }
    }

    /// Draw the mark over the bound framebuffer, unoccluded, in `shown`'s
    /// color. `transform` maps it to clip space ([`corner_transform`],
    /// [`head_locked_transform`]); `viewport` is the target's size.
    pub fn draw(
        &self,
        gl: &glow::Context,
        transform: Matrix4<f32>,
        shown: ReloadShown,
        viewport: (u32, u32),
        // The shell's `SceneContext::output_srgb_encode()`.
        output_srgb_encode: bool,
    ) {
        let color = crate::color_space::srgb_to_linear3(shown.color());
        let transform = cgmath::conv::array4x4(transform);
        unsafe {
            let cull = gl.is_enabled(glow::CULL_FACE);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.viewport(0, 0, viewport.0 as i32, viewport.1 as i32);
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(self.program, "u_transform").as_ref(),
                false,
                transform.as_flattened(),
            );
            gl.uniform_3_f32_slice(gl.get_uniform_location(self.program, "u_color").as_ref(), &color);
            gl.uniform_1_i32(
                gl.get_uniform_location(self.program, "uOutputFlags").as_ref(),
                crate::color_space::output_flags(output_srgb_encode, crate::color_space::OutputAlpha::Opaque),
            );
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, self.vertices);
            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.enable(glow::DEPTH_TEST);
            if cull {
                gl.enable(glow::CULL_FACE);
            }
        }
    }

    pub fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_buffer(self.vbo);
            gl.delete_vertex_array(self.vao);
            gl.delete_program(self.program);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered(indicator: &mut ReloadIndicator, now: Instant, result: Reply) -> Receiver<Reply> {
        let (reply, pusher) = mpsc::channel();
        indicator.track(reply, now).send(result).unwrap();
        pusher
    }

    #[test]
    fn spins_while_in_flight_then_holds_the_outcome() {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        assert_eq!(indicator.state(t0), ReloadState::default());

        let (reply, pusher) = mpsc::channel();
        let producer = indicator.track(reply, t0);
        indicator.poll(t0 + Duration::from_millis(250), 10);
        let (shown, spin) = indicator.look(t0 + Duration::from_millis(250)).unwrap();
        assert_eq!(shown, ReloadShown::Loading);
        assert!((spin - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "a quarter turn: {spin}");
        assert!(indicator.state(t0).in_progress);

        producer.send(Ok("reloaded".to_string())).unwrap();
        let settled = t0 + Duration::from_millis(400);
        indicator.poll(settled, 12);
        assert_eq!(pusher.try_recv().unwrap(), Ok("reloaded".to_string()), "forwarded to the pusher");
        let state = indicator.state(settled);
        assert!(!state.in_progress);
        assert_eq!(state.shown, Some(ReloadShown::Ok));
        let last = state.last.unwrap();
        assert!(last.ok && last.rendered_frame == 12 && (last.ms - 400.0).abs() < 1.0);

        assert_eq!(indicator.look(settled + HOLD), None, "hidden after the hold");
        assert!(indicator.state(settled + HOLD).last.is_some(), "the outcome stays in /state");
    }

    #[test]
    fn a_rejected_or_dropped_reload_shows_red() {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        let pusher = answered(&mut indicator, t0, Err("type error".to_string()));
        indicator.poll(t0, 0);
        assert_eq!(pusher.try_recv().unwrap(), Err("type error".to_string()));
        assert_eq!(indicator.look(t0), Some((ReloadShown::Failed, 0.0)));

        let (reply, _pusher) = mpsc::channel();
        drop(indicator.track(reply, t0));
        indicator.poll(t0, 0);
        assert_eq!(indicator.state(t0).shown, Some(ReloadShown::Failed));
    }

    #[test]
    fn a_newer_push_keeps_it_spinning() {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        let (reply, _pusher) = mpsc::channel();
        let _newer = indicator.track(reply, t0);
        let _old = answered(&mut indicator, t0, Err("superseded".to_string()));
        indicator.poll(t0, 0);
        assert_eq!(indicator.state(t0).shown, Some(ReloadShown::Loading));
    }

    #[test]
    fn the_mark_spans_unit_height_centered() {
        let v = mark_vertices();
        let ys = v.iter().skip(1).step_by(2);
        let (lo, hi) = ys.fold((f32::MAX, f32::MIN), |(lo, hi), &y| (lo.min(y), hi.max(y)));
        assert!((lo + 0.5).abs() < 1e-6 && (hi - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_corner_mark_sits_lower_left() {
        let t = corner_transform(800, 600, 0.0);
        let center = t * cgmath::vec4(0.0, 0.0, 0.0, 1.0);
        // 8% of 600 = 48 px tall, centered 48 px in from each edge.
        assert!((center.x - (2.0 * 48.0 / 800.0 - 1.0)).abs() < 1e-5);
        assert!((center.y - (2.0 * 48.0 / 600.0 - 1.0)).abs() < 1e-5);
    }

    #[test]
    fn head_locked_mark_is_ahead_and_lower_left_for_both_eyes() {
        for x in [-0.032, 0.032] {
            let eye = TrackingPose::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
            let clip = head_locked_transform([-0.8, 0.8, -0.8, 0.8], eye, 0.0) * cgmath::vec4(0.0, 0.0, 0.0, 1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(clip.w > 0.0, "in front of the eye");
            assert!(ndc.x < 0.0 && ndc.y < 0.0 && ndc.x > -1.0 && ndc.y > -1.0, "lower-left, in view: {ndc:?}");
        }
    }
}
