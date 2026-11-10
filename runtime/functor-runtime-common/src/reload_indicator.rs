//! The shell's hot-reload indicator: a small Functor mark in a corner of the
//! view that fades in and spins while a pushed reload (`/reload-source`,
//! `/reload-project`) is in flight, then shows the outcome for [`HOLD`] —
//! still spinning in cyan (swapped in), or red and swinging to rest (rejected)
//! — and fades out over [`FADE`]. Shell-owned: it never reaches the game's
//! model, frames, or history, and draws over the finished frame.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use cgmath::{Matrix4, Quaternion, Rad, SquareMatrix};
use glow::HasContext;
use serde::{Deserialize, Serialize};

use crate::color_space::{OutputAlpha, OutputEncodeUniform};
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::TrackingPose;

/// How long the outcome shows at full strength after a reload settles.
pub const HOLD: Duration = Duration::from_millis(500);

/// The fade in when a reload starts, and out after [`HOLD`].
pub const FADE: Duration = Duration::from_millis(200);

/// Turns per second.
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
    /// Authored sRGB: the logo's cyan (`#41d8e6`, the mark on dark grounds),
    /// or red for a rejected reload.
    pub fn color(self) -> [f32; 3] {
        match self {
            ReloadShown::Loading | ReloadShown::Ok => [65.0 / 255.0, 216.0 / 255.0, 230.0 / 255.0],
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
    /// What the indicator draws now, fades included; omitted when hidden.
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

/// The indicator at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReloadLook {
    pub shown: ReloadShown,
    /// Radians about the mark's vertical axis.
    pub spin: f32,
    /// Opacity, `0..=1`.
    pub alpha: f32,
}

/// Pushed reloads in flight and the last outcome.
#[derive(Default)]
pub struct ReloadIndicator {
    in_flight: Vec<InFlight>,
    last: Option<(ReloadOutcome, Instant)>,
    /// When the current showing began fading in.
    fade_in_from: Option<Instant>,
    /// The first push: the spin runs from here and never restarts, so a push
    /// during a fade-out doesn't jump the angle.
    spin_from: Option<Instant>,
}

impl ReloadIndicator {
    /// Track a push answered on `reply`: the producer answers on the returned
    /// sender instead, and [`Self::poll`] forwards it.
    pub fn track(&mut self, reply: Sender<Reply>, now: Instant) -> Sender<Reply> {
        // Fade in from however visible it is now.
        let alpha = self.look(now).map_or(0.0, |look| look.alpha);
        self.fade_in_from = Some(now - FADE.mul_f32(alpha));
        self.spin_from.get_or_insert(now);
        let (tx, answer) = mpsc::channel();
        self.in_flight.push(InFlight { started: now, answer, reply });
        tx
    }

    /// Forward answered pushes to their pushers, recording each outcome. Call
    /// once per shell loop, before reading [`Self::state`].
    pub fn poll(&mut self, now: Instant, rendered_frames: u64) {
        let mut last = self.last;
        self.in_flight.retain(|push| {
            let answer = match push.answer.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => return true,
                // Unanswered: a failure, and `reply` drops too, so the pusher
                // sees a runtime fault (500), not a rejected program.
                Err(TryRecvError::Disconnected) => None,
            };
            let outcome = ReloadOutcome {
                ok: matches!(answer, Some(Ok(_))),
                ms: (now - push.started).as_secs_f64() * 1000.0,
                rendered_frame: rendered_frames,
            };
            last = Some((outcome, now));
            if let Some(result) = answer {
                let _ = push.reply.send(result);
            }
            false
        });
        self.last = last;
    }

    /// What to draw at `now`; `None` when hidden.
    pub fn look(&self, now: Instant) -> Option<ReloadLook> {
        let ramp = |d: Duration| (d.as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        let spin_from = self.spin_from?;
        let turns_at = |t: Instant| t.saturating_duration_since(spin_from).as_secs_f32() * SPIN_HZ;
        let fade_in = ramp(now.saturating_duration_since(self.fade_in_from?));
        let (shown, turns, alpha) = if self.in_flight.is_empty() {
            let (outcome, at) = self.last?;
            let since = now.saturating_duration_since(at);
            let left = (HOLD + FADE).checked_sub(since)?;
            if left.is_zero() {
                return None;
            }
            let alpha = fade_in.min(ramp(left));
            if outcome.ok {
                (ReloadShown::Ok, turns_at(now), alpha)
            } else {
                // Swing the short way round to rest facing the viewer.
                let stopped = turns_at(at);
                (ReloadShown::Failed, stopped + (stopped.round() - stopped) * ramp(since), alpha)
            }
        } else {
            (ReloadShown::Loading, turns_at(now), fade_in)
        };
        Some(ReloadLook { shown, spin: turns.rem_euclid(1.0) * std::f32::consts::TAU, alpha })
    }

    pub fn state(&self, now: Instant) -> ReloadState {
        ReloadState {
            in_progress: !self.in_flight.is_empty(),
            shown: self.look(now).map(|look| look.shown),
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
    let eye_from_head = eye.invert().unwrap_or(Matrix4::identity());
    projection
        * eye_from_head
        * Matrix4::from_translation(cgmath::vec3(-0.15, -0.12, -0.6))
        * Matrix4::from_scale(0.05)
        * Matrix4::from_angle_y(Rad(spin))
}

/// The indicator's GL program and mark mesh, built once per context.
pub struct ReloadIndicatorPainter {
    program: ShaderProgram,
    transform: UniformLocation,
    color: UniformLocation,
    output: OutputEncodeUniform,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    vertices: i32,
}

const VERTEX_SHADER: &str = r#"
    layout(location = 0) in vec2 a_pos;
    uniform mat4 u_transform;
    void main() { gl_Position = u_transform * vec4(a_pos, 0.0, 1.0); }
"#;

const FRAGMENT_SHADER: &str = r#"
    uniform vec4 u_color;
    out vec4 frag_color;
    void main() { frag_color = functorOutput(u_color); }
"#;

impl ReloadIndicatorPainter {
    pub fn new(gl: &glow::Context, shader_version: &str) -> Self {
        let vertex = Shader::build(gl, ShaderType::Vertex, VERTEX_SHADER, shader_version);
        let fragment = Shader::build(gl, ShaderType::Fragment, FRAGMENT_SHADER, shader_version);
        let program = ShaderProgram::link(gl, &vertex, &fragment);
        let vertices = mark_vertices();
        let counters = crate::gpu_counters::gpu_counters();
        unsafe {
            let vao = gl.create_vertex_array().expect("create reload indicator vao");
            counters.vao_created();
            gl.bind_vertex_array(Some(vao));
            let vbo = gl.create_buffer().expect("create reload indicator vbo");
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let bytes = crate::terrain_renderer::slice_bytes(&vertices);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            counters.uploaded(bytes.len());
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            ReloadIndicatorPainter {
                transform: program.get_uniform_location(gl, "u_transform"),
                color: program.get_uniform_location(gl, "u_color"),
                output: OutputEncodeUniform::get(&program, gl),
                program,
                vao,
                vbo,
                vertices: (vertices.len() / 2) as i32,
            }
        }
    }

    /// Draw the mark over the bound framebuffer, unoccluded, in `shown`'s
    /// color at `alpha`. `transform` maps it to clip space
    /// ([`corner_transform`], [`head_locked_transform`]); `viewport` is the
    /// target's size. Restores the capabilities it toggles, but leaves the
    /// viewport and blend function set.
    pub fn draw(
        &self,
        gl: &glow::Context,
        transform: Matrix4<f32>,
        shown: ReloadShown,
        alpha: f32,
        viewport: (u32, u32),
        // The shell's `SceneContext::output_srgb_encode()`.
        output_srgb_encode: bool,
    ) {
        let [r, g, b] = crate::color_space::srgb_to_linear3(shown.color());
        let caps = [glow::CULL_FACE, glow::DEPTH_TEST, glow::SCISSOR_TEST, glow::BLEND];
        unsafe {
            let was = caps.map(|cap| gl.is_enabled(cap));
            for cap in caps {
                gl.disable(cap);
            }
            // Source-over, for the target's alpha too: a faded mark over a
            // passthrough eye buffer lets the room through by the same share.
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func_separate(
                glow::SRC_ALPHA,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
            );
            gl.viewport(0, 0, viewport.0 as i32, viewport.1 as i32);
            self.program.use_program(gl);
            self.program.set_uniform_matrix4(gl, &self.transform, &transform);
            self.program.set_uniform_vec4(gl, &self.color, &cgmath::vec4(r, g, b, alpha));
            self.output.set(&self.program, gl, output_srgb_encode, OutputAlpha::Blend);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, self.vertices);
            gl.bind_vertex_array(None);
            gl.use_program(None);
            for (cap, on) in caps.into_iter().zip(was) {
                if on {
                    gl.enable(cap);
                } else {
                    gl.disable(cap);
                }
            }
        }
    }

    pub fn delete(self, gl: &glow::Context) {
        let counters = crate::gpu_counters::gpu_counters();
        unsafe {
            gl.delete_buffer(self.vbo);
            counters.buffer_deleted();
            gl.delete_vertex_array(self.vao);
            counters.vao_deleted();
            gl.delete_program(self.program.program_id);
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
    fn fades_in_spinning_then_shows_the_outcome_and_fades_out() {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        assert_eq!(indicator.state(t0), ReloadState::default());

        let (reply, pusher) = mpsc::channel();
        let producer = indicator.track(reply, t0);
        indicator.poll(t0 + Duration::from_millis(250), 10);
        let look = indicator.look(t0 + Duration::from_millis(250)).unwrap();
        assert_eq!((look.shown, look.alpha), (ReloadShown::Loading, 1.0));
        assert!((look.spin - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "a quarter turn: {}", look.spin);
        assert!(indicator.state(t0).in_progress);
        let fading_in = indicator.look(t0 + FADE / 2).unwrap();
        assert!((fading_in.alpha - 0.5).abs() < 1e-4, "{fading_in:?}");

        producer.send(Ok("reloaded".to_string())).unwrap();
        let settled = t0 + Duration::from_millis(400);
        indicator.poll(settled, 12);
        assert_eq!(pusher.try_recv().unwrap(), Ok("reloaded".to_string()), "forwarded to the pusher");
        let state = indicator.state(settled);
        assert!(!state.in_progress);
        assert_eq!(state.shown, Some(ReloadShown::Ok));
        let last = state.last.unwrap();
        assert!(last.ok && last.rendered_frame == 12 && (last.ms - 400.0).abs() < 1.0);

        // Success keeps turning at full strength through the hold...
        let held = indicator.look(settled + HOLD).unwrap();
        assert_eq!((held.shown, held.alpha), (ReloadShown::Ok, 1.0));
        assert!((held.spin - 0.9 * std::f32::consts::TAU).abs() < 1e-3, "{held:?}");
        // ...then fades out.
        let fading_out = indicator.look(settled + HOLD + FADE / 2).unwrap();
        assert!((fading_out.alpha - 0.5).abs() < 1e-4, "{fading_out:?}");
        assert_eq!(indicator.look(settled + HOLD + FADE), None, "hidden after the fade");
        assert!(indicator.state(settled + HOLD + FADE).last.is_some(), "the outcome stays in /state");
    }

    #[test]
    fn a_quick_reload_still_fades_in() {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        let _pusher = answered(&mut indicator, t0, Ok("reloaded".to_string()));
        indicator.poll(t0, 0);
        assert_eq!(indicator.look(t0).unwrap().alpha, 0.0);
        assert!((indicator.look(t0 + FADE / 4).unwrap().alpha - 0.25).abs() < 1e-4);
    }

    #[test]
    fn a_push_during_the_fade_out_fades_back_in_from_there() {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        let _first = answered(&mut indicator, t0, Ok("reloaded".to_string()));
        indicator.poll(t0, 0);
        let again = t0 + HOLD + FADE / 2;
        let before = indicator.look(again).unwrap();
        let (reply, _pusher) = mpsc::channel();
        let _producer = indicator.track(reply, again);
        let after = indicator.look(again).unwrap();
        assert_eq!(after.shown, ReloadShown::Loading);
        assert!((after.alpha - before.alpha).abs() < 1e-4 && (after.spin - before.spin).abs() < 1e-4);
        assert_eq!(indicator.look(again + FADE).unwrap().alpha, 1.0);
    }

    #[test]
    fn a_rejected_or_dropped_reload_shows_red() {
        let t0 = Instant::now();
        let mut indicator = ReloadIndicator::default();
        let pusher = answered(&mut indicator, t0, Err("type error".to_string()));
        indicator.poll(t0, 0);
        assert_eq!(pusher.try_recv().unwrap(), Err("type error".to_string()));
        let look = indicator.look(t0 + FADE).unwrap();
        assert_eq!(look, ReloadLook { shown: ReloadShown::Failed, spin: 0.0, alpha: 1.0 });

        // Rejected 0.4 of a turn in: it swings back to face the viewer.
        let (reply, _pusher) = mpsc::channel();
        let rejected = t0 + Duration::from_millis(1400);
        indicator.track(reply, t0 + Duration::from_secs(1)).send(Err("type error".to_string())).unwrap();
        indicator.poll(rejected, 0);
        let turning = indicator.look(rejected + FADE / 2).unwrap();
        assert!((turning.spin - 0.2 * std::f32::consts::TAU).abs() < 1e-3, "{turning:?}");
        let resting = indicator.look(rejected + FADE).unwrap();
        assert!(resting.spin.abs() < 1e-3 && resting.shown == ReloadShown::Failed, "{resting:?}");

        let (reply, pusher) = mpsc::channel();
        drop(indicator.track(reply, t0));
        indicator.poll(t0, 0);
        assert_eq!(indicator.state(t0).shown, Some(ReloadShown::Failed));
        assert_eq!(pusher.try_recv(), Err(TryRecvError::Disconnected), "a fault, not a rejection");
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
