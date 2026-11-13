//! The headset's link overlay: a head-locked panel the shell draws over any
//! game while the device link pairs, the push-to-talk [`Toast`], and the
//! controller gestures that drive them. Shell-owned: the game sees neither
//! the panel nor the presses.
//!
//! Hold the left menu button [`HOLD`] to start pairing; while the code
//! shows, A says it matches the desktop's and B cancels. Once paired, hold
//! the right thumbstick click [`TALK_HOLD`] to talk, release to send; B
//! while talking cancels.

use std::time::{Duration, Instant};

use cgmath::Matrix4;
use glow::HasContext;

use crate::color_space::{OutputAlpha, OutputEncodeUniform};
use crate::debug_protocol::{PairingState, VoiceStage, VoiceToast};
use crate::input::{XrControllerSnapshot, XrInputSnapshot};
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::sprite_font;
use crate::TrackingPose;

/// How long the menu button is held to start pairing.
pub const HOLD: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Pair,
    Matches,
    Cancel,
}

/// Turns controller samples into [`Gesture`]s, on press edges.
#[derive(Default)]
pub struct Gestures {
    menu_since: Option<Instant>,
    menu_fired: bool,
    a: bool,
    b: bool,
}

impl Gestures {
    /// One sample; `open` while the overlay shows.
    pub fn step(
        &mut self,
        xr: Option<&XrInputSnapshot>,
        open: bool,
        now: Instant,
    ) -> Option<Gesture> {
        let (menu, a, b) = xr.map_or((false, false, false), |xr| {
            (
                xr.left.menu_pressed,
                xr.right.primary_pressed,
                xr.right.secondary_pressed,
            )
        });
        let pressed = |was: &mut bool, is: bool| !std::mem::replace(was, is) && is;
        let (a, b) = (pressed(&mut self.a, a), pressed(&mut self.b, b));
        let held = if menu {
            now.duration_since(*self.menu_since.get_or_insert(now))
        } else {
            self.menu_since = None;
            self.menu_fired = false;
            Duration::ZERO
        };
        if open {
            // A hold that outlasts the overlay must be released first.
            self.menu_fired |= menu;
            return match (a, b) {
                (_, true) => Some(Gesture::Cancel),
                (true, _) => Some(Gesture::Matches),
                _ => None,
            };
        }
        if menu && !self.menu_fired && held >= HOLD {
            self.menu_fired = true;
            return Some(Gesture::Pair);
        }
        None
    }
}

/// What the game samples while the overlay shows: poses, no presses.
pub fn without_presses(xr: &mut XrInputSnapshot) {
    for hand in [&mut xr.left, &mut xr.right] {
        *hand = XrControllerSnapshot {
            active: hand.active,
            grip: hand.grip,
            aim: hand.aim,
            ..XrControllerSnapshot::default()
        };
    }
}

/// How long the right thumbstick click is held before it talks; a shorter
/// click is the game's.
pub const TALK_HOLD: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Talk {
    Start,
    /// Released: send what was said.
    Send,
    Cancel,
}

/// Push-to-talk on the right thumbstick click. While it is held the game
/// does not see the click; a click shorter than [`TALK_HOLD`] reaches it
/// on release. While talking, B cancels and is not the game's either.
#[derive(Default)]
pub struct PushToTalk {
    since: Option<Instant>,
    talking: bool,
    /// Ended while still held (cancelled, or by the shell): wait for release.
    done: bool,
    /// A short click to hand the game.
    tap: bool,
    b: bool,
}

impl PushToTalk {
    /// One sample; `armed` while paired. Unarmed, the click is the game's.
    pub fn step(&mut self, xr: Option<&XrInputSnapshot>, armed: bool, now: Instant) -> Option<Talk> {
        let (click, b) = xr.map_or((false, false), |xr| {
            (xr.right.thumbstick_pressed, xr.right.secondary_pressed)
        });
        let b = !std::mem::replace(&mut self.b, b) && b;
        if !click {
            let held = self.since.take().is_some();
            let done = std::mem::take(&mut self.done);
            if std::mem::take(&mut self.talking) {
                return Some(Talk::Send);
            }
            self.tap |= held && !done;
            return None;
        }
        if !armed && self.since.is_none() {
            return None;
        }
        let since = *self.since.get_or_insert(now);
        if self.talking && b {
            self.stop();
            return Some(Talk::Cancel);
        }
        if !self.talking && !self.done && now.duration_since(since) >= TALK_HOLD {
            self.talking = true;
            return Some(Talk::Start);
        }
        None
    }

    /// The shell ended the utterance itself (its time limit, a lost link):
    /// the button must be released before it talks again.
    pub fn stop(&mut self) {
        self.done |= std::mem::take(&mut self.talking);
    }

    /// What the game sees of the right hand this frame; `stepping` when it
    /// is sampled (a short click is handed over then).
    pub fn game_view(&mut self, xr: &mut XrInputSnapshot, stepping: bool) {
        if self.since.is_some() {
            xr.right.thumbstick_pressed = false;
            if self.talking || self.done {
                xr.right.secondary_pressed = false;
            }
        }
        if self.tap && stepping {
            self.tap = false;
            xr.right.thumbstick_pressed = true;
        }
    }
}

/// One line of the panel: text, cell height in metres, authored sRGB color.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub size: f32,
    pub color: [f32; 3],
}

const CYAN: [f32; 3] = [65.0 / 255.0, 216.0 / 255.0, 230.0 / 255.0];
const PINK: [f32; 3] = [1.0, 0.36, 0.72];
const WHITE: [f32; 3] = [0.95, 0.95, 0.98];
const DIM: [f32; 3] = [0.65, 0.65, 0.75];
const BACKGROUND: [f32; 4] = [0.06, 0.04, 0.12, 0.92];
/// Text sizes: a heading, the code, the rest.
const HEADING: f32 = 0.022;
const CODE: f32 = 0.07;
const BODY: f32 = 0.016;

/// `482913` as `482 913`.
pub fn spaced(code: &str) -> String {
    match code.len() {
        6 if code.is_ascii() => format!("{} {}", &code[..3], &code[3..]),
        _ => code.to_string(),
    }
}

/// The panel's lines for `pairing`.
pub fn lines(pairing: &PairingState) -> Vec<Line> {
    let line = |text: &str, size, color| Line {
        text: text.to_string(),
        size,
        color,
    };
    match pairing {
        PairingState::Searching => vec![
            line("PAIR WITH A DESKTOP", HEADING, CYAN),
            line("Looking for Functor on this network...", BODY, WHITE),
            line("B  Cancel", BODY, DIM),
        ],
        PairingState::Code {
            desktop,
            code,
            confirmed,
        } => vec![
            line(
                &format!("PAIR WITH {}", desktop.to_uppercase()),
                HEADING,
                CYAN,
            ),
            line(&spaced(code), CODE, WHITE),
            if *confirmed {
                line("Now click Allow on the desktop", BODY, WHITE)
            } else {
                line("Does the desktop show this code?", BODY, WHITE)
            },
            if *confirmed {
                line("B  Cancel", BODY, DIM)
            } else {
                line("A  Matches      B  Cancel", BODY, PINK)
            },
        ],
        PairingState::Paired { desktop } => vec![
            line("PAIRED", HEADING, CYAN),
            line(&format!("with {desktop}"), BODY, WHITE),
        ],
        PairingState::Failed { reason } => {
            vec![line("NOT PAIRED", HEADING, PINK), line(reason, BODY, WHITE)]
        }
    }
}

/// Steps on the toast's level meter.
pub const METER: u8 = 12;
/// How long an outcome shows, and the fade after.
const OUTCOME: Duration = Duration::from_secs(4);
const FADE: Duration = Duration::from_millis(500);
/// How long the transcript shows.
const TRANSCRIPT: Duration = Duration::from_secs(5);
/// A stage waiting on the desktop stops showing after this.
const STALE: Duration = Duration::from_secs(120);
/// Transcript characters per line, and its most lines.
const WRAP: usize = 34;
const WRAP_LINES: usize = 3;

/// The push-to-talk toast: a stage, the transcript for a while, and a fade
/// once done. Hidden, it holds nothing and costs one check a frame.
#[derive(Default)]
pub struct Toast {
    shown: Option<VoiceToast>,
    /// When it starts fading; `None` stays until changed.
    fade_at: Option<Instant>,
    transcript_until: Option<Instant>,
}

impl Toast {
    pub fn show(&mut self, stage: VoiceStage, now: Instant) {
        self.fade_at = match stage {
            VoiceStage::Listening { .. } => None,
            VoiceStage::Sending | VoiceStage::Heard | VoiceStage::Working => Some(now + STALE),
            _ => Some(now + OUTCOME),
        };
        // A new utterance starts clean.
        let transcript = self
            .shown
            .take()
            .and_then(|t| t.transcript)
            .filter(|_| !matches!(stage, VoiceStage::Listening { .. }));
        self.shown = Some(VoiceToast { stage, transcript });
    }

    pub fn transcript(&mut self, text: String, now: Instant) {
        if let Some(shown) = &mut self.shown {
            shown.transcript = Some(text);
            self.transcript_until = Some(now + TRANSCRIPT);
        }
    }

    /// Fades out from `now`.
    pub fn hide(&mut self, now: Instant) {
        if self.shown.is_some() {
            self.fade_at = Some(self.fade_at.map_or(now, |at| at.min(now)));
        }
    }

    pub fn stage(&self) -> Option<&VoiceStage> {
        self.shown.as_ref().map(|t| &t.stage)
    }

    /// What shows at `now`, and its opacity.
    pub fn look(&mut self, now: Instant) -> Option<(&VoiceToast, f32)> {
        if self.transcript_until.is_some_and(|until| now >= until) {
            self.transcript_until = None;
            if let Some(shown) = &mut self.shown {
                shown.transcript = None;
            }
        }
        let alpha = match self.fade_at {
            Some(at) if now >= at + FADE => {
                self.shown = None;
                self.transcript_until = None;
                return None;
            }
            Some(at) if now > at => {
                1.0 - now.duration_since(at).as_secs_f32() / FADE.as_secs_f32()
            }
            _ => 1.0,
        };
        self.shown.as_ref().map(|t| (t, alpha))
    }
}

/// The toast's lines.
pub fn toast_lines(toast: &VoiceToast) -> Vec<Line> {
    let line = |text: &str, size, color| Line {
        text: text.to_string(),
        size,
        color,
    };
    let mut out = match &toast.stage {
        VoiceStage::NotConnected => vec![
            line("NOT CONNECTED", HEADING, PINK),
            line("Not connected to a desktop", BODY, WHITE),
        ],
        VoiceStage::AllowMicrophone => vec![
            line("MICROPHONE", HEADING, PINK),
            line("Allow the microphone, then hold again", BODY, WHITE),
        ],
        VoiceStage::Listening {
            level,
            seconds_left,
        } => {
            let on = usize::from((*level).min(METER));
            let meter = format!("{}{}", "|".repeat(on), ".".repeat(usize::from(METER) - on));
            vec![
                line("LISTENING...", HEADING, CYAN),
                line(&meter, HEADING, WHITE),
                match seconds_left {
                    Some(s) => line(&format!("Sending in {s}"), BODY, PINK),
                    None => line("Release to send      B  Cancel", BODY, DIM),
                },
            ]
        }
        VoiceStage::Sending => vec![line("SENDING...", HEADING, CYAN)],
        VoiceStage::Heard => vec![line("HEARD", HEADING, CYAN)],
        VoiceStage::Working => vec![line("WORKING...", HEADING, CYAN)],
        VoiceStage::Reloaded => vec![line("RELOADED", HEADING, CYAN)],
        VoiceStage::Failed { reason } => {
            vec![line("FAILED", HEADING, PINK), line(reason, BODY, WHITE)]
        }
        VoiceStage::NotSent { reason } => {
            vec![line("NOT SENT", HEADING, PINK), line(reason, BODY, WHITE)]
        }
    };
    if let Some(text) = &toast.transcript {
        let quoted = format!("\"{text}\"");
        out.extend(
            wrap(&quoted, WRAP, WRAP_LINES)
                .iter()
                .map(|l| line(l, BODY, WHITE)),
        );
    }
    out
}

/// `text` in lines of at most `width` characters, broken at spaces where
/// it can be; at most `lines` of them, the last cut with `...`.
fn wrap(text: &str, width: usize, lines: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        while !word.is_empty() {
            let used = current.chars().count();
            let gap = usize::from(used > 0);
            if used + gap + word.len() <= width {
                if gap == 1 {
                    current.push(' ');
                }
                current.extend(word.drain(..));
            } else if used > 0 {
                out.push(std::mem::take(&mut current));
            } else {
                // A word longer than a line: split it.
                out.push(word.drain(..width).collect());
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.len() > lines {
        out.truncate(lines);
        let last = &mut out[lines - 1];
        let keep: String = last.chars().take(width - 3).collect();
        *last = keep + "...";
    }
    out
}

/// Where the toast sits: a metre out and low in the view, following the
/// head's turns lazily — still within [`FOLLOW_SLACK`], then easing back in
/// front.
#[derive(Default)]
pub struct Follow {
    yaw: Option<f32>,
    catching: bool,
}

const FOLLOW_SLACK: f32 = 20.0 * std::f32::consts::PI / 180.0;

impl Follow {
    /// Head space (metres) from the toast's own, for `head` (the head's
    /// orientation `[x, y, z, w]` in tracking space) after `dt` seconds.
    pub fn transform(&mut self, head: [f32; 4], dt: f32) -> Matrix4<f32> {
        let [x, y, z, w] = head;
        let forward = cgmath::Quaternion::new(w, x, y, z) * cgmath::vec3(0.0, 0.0, -1.0);
        let facing = (-forward.x).atan2(-forward.z);
        let yaw = self.yaw.get_or_insert(facing);
        let behind = wrap_angle(facing - *yaw);
        self.catching |= behind.abs() > FOLLOW_SLACK;
        if self.catching {
            *yaw = wrap_angle(*yaw + behind * (dt * 4.0).min(1.0));
            self.catching = wrap_angle(facing - *yaw).abs() > 2f32.to_radians();
        }
        Matrix4::from_angle_y(cgmath::Rad(wrap_angle(*yaw - facing)))
            * Matrix4::from_translation(cgmath::vec3(0.0, -0.3, -1.0))
    }

    /// Hidden: it appears in front next time.
    pub fn reset(&mut self) {
        *self = Follow::default();
    }
}

fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a + PI).rem_euclid(TAU) - PI
}

/// Floats per vertex: position (2), atlas uv (2, negative for a solid
/// fill), linear RGBA (4).
const STRIDE: usize = 8;
const PAD: f32 = 0.03;
const GAP: f32 = 0.018;

/// The panel's triangles in metres, centred on the origin, y up: a
/// background, then one quad per visible glyph.
pub fn mesh(lines: &[Line]) -> Vec<f32> {
    let width = |l: &Line| sprite_font::advance_count(&l.text) as f32 * l.size;
    let inner_w = lines.iter().map(width).fold(0.0, f32::max);
    let inner_h =
        lines.iter().map(|l| l.size).sum::<f32>() + GAP * lines.len().saturating_sub(1) as f32;
    let (w, h) = (inner_w + 2.0 * PAD, inner_h + 2.0 * PAD);
    let mut out = Vec::new();
    let bg = crate::color_space::srgb_to_linear3([BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]]);
    quad(
        &mut out,
        [-w / 2.0, -h / 2.0, w, h],
        None,
        [bg[0], bg[1], bg[2], BACKGROUND[3]],
    );
    let mut top = h / 2.0 - PAD;
    let atlas = [
        sprite_font::ATLAS_WIDTH as f32,
        sprite_font::ATLAS_HEIGHT as f32,
    ];
    for line in lines {
        let [r, g, b] = crate::color_space::srgb_to_linear3(line.color);
        let mut x = -width(line) / 2.0;
        for character in line.text.chars() {
            if let Some((cx, cy, cw, ch)) = sprite_font::glyph_cell(character) {
                let uv = [cx / atlas[0], cy / atlas[1], cw / atlas[0], ch / atlas[1]];
                quad(
                    &mut out,
                    [x, top - line.size, line.size, line.size],
                    Some(uv),
                    [r, g, b, 1.0],
                );
            }
            x += line.size;
        }
        top -= line.size + GAP;
    }
    out
}

/// `rect` is `[x, y, w, h]` (y up); `uv` is the atlas cell `[u, v, w, h]`
/// with v down, as the atlas is stored.
fn quad(out: &mut Vec<f32>, rect: [f32; 4], uv: Option<[f32; 4]>, color: [f32; 4]) {
    let [x, y, w, h] = rect;
    let corner = |dx: f32, dy: f32| {
        let (u, v) = uv.map_or((-1.0, -1.0), |[u, v, uw, vh]| {
            (u + dx * uw, v + (1.0 - dy) * vh)
        });
        [
            x + dx * w,
            y + dy * h,
            u,
            v,
            color[0],
            color[1],
            color[2],
            color[3],
        ]
    };
    for (dx, dy) in [
        (0.0, 0.0),
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
    ] {
        out.extend(corner(dx, dy));
    }
}

/// Head-locked for one eye: a metre ahead, 10° below the gaze. `fov`
/// is the eye's `[left, right, down, up]` in radians; `eye_in_head` its pose
/// relative to the head centre, so both eyes agree on the panel's depth.
pub fn head_locked_transform(fov: [f32; 4], eye_in_head: TrackingPose) -> Matrix4<f32> {
    crate::reload_indicator::head_to_clip(fov, eye_in_head)
        * Matrix4::from_translation(cgmath::vec3(0.0, -0.18, -1.0))
}

/// The overlay's GL program, font atlas and current mesh.
pub struct LinkOverlayPainter {
    program: ShaderProgram,
    transform: UniformLocation,
    alpha: UniformLocation,
    atlas_uniform: UniformLocation,
    output: OutputEncodeUniform,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    atlas: glow::Texture,
    /// What the mesh shows, so it is rebuilt only on change.
    shown: Vec<Line>,
    vertices: i32,
}

const VERTEX_SHADER: &str = r#"
    layout(location = 0) in vec2 a_pos;
    layout(location = 1) in vec2 a_uv;
    layout(location = 2) in vec4 a_color;
    uniform mat4 u_transform;
    out vec2 v_uv;
    out vec4 v_color;
    void main() {
        v_uv = a_uv;
        v_color = a_color;
        gl_Position = u_transform * vec4(a_pos, 0.0, 1.0);
    }
"#;

const FRAGMENT_SHADER: &str = r#"
    uniform sampler2D u_atlas;
    uniform float u_alpha;
    in vec2 v_uv;
    in vec4 v_color;
    out vec4 frag_color;
    void main() {
        float coverage = v_uv.x < 0.0 ? 1.0 : texture(u_atlas, v_uv).a;
        frag_color = functorOutput(vec4(v_color.rgb, v_color.a * coverage * u_alpha));
    }
"#;

impl LinkOverlayPainter {
    pub fn new(gl: &glow::Context, shader_version: &str) -> Self {
        let vertex = Shader::build(gl, ShaderType::Vertex, VERTEX_SHADER, shader_version);
        let fragment = Shader::build(gl, ShaderType::Fragment, FRAGMENT_SHADER, shader_version);
        let program = ShaderProgram::link(gl, &vertex, &fragment);
        let counters = crate::gpu_counters::gpu_counters();
        let font = sprite_font::atlas_texture_data();
        unsafe {
            let atlas = gl.create_texture().expect("create link overlay atlas");
            counters.texture_created();
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                font.width as i32,
                font.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&font.bytes)),
            );
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            let vao = gl.create_vertex_array().expect("create link overlay vao");
            counters.vao_created();
            gl.bind_vertex_array(Some(vao));
            let vbo = gl.create_buffer().expect("create link overlay vbo");
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = (STRIDE * 4) as i32;
            for (index, (size, offset)) in [(2, 0), (2, 2), (4, 4)].into_iter().enumerate() {
                gl.enable_vertex_attrib_array(index as u32);
                gl.vertex_attrib_pointer_f32(
                    index as u32,
                    size,
                    glow::FLOAT,
                    false,
                    stride,
                    offset * 4,
                );
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            LinkOverlayPainter {
                transform: program.get_uniform_location(gl, "u_transform"),
                alpha: program.get_uniform_location(gl, "u_alpha"),
                atlas_uniform: program.get_uniform_location(gl, "u_atlas"),
                output: OutputEncodeUniform::get(&program, gl),
                program,
                vao,
                vbo,
                atlas,
                shown: Vec::new(),
                vertices: 0,
            }
        }
    }

    /// Draws `lines` ([`lines`], [`toast_lines`]) at `alpha` over the bound
    /// framebuffer, unoccluded; `transform` maps panel metres to clip space
    /// ([`head_locked_transform`]). Restores the capabilities it toggles, but
    /// leaves the viewport and blend function set.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        gl: &glow::Context,
        lines: &[Line],
        alpha: f32,
        transform: Matrix4<f32>,
        viewport: (u32, u32),
        // The shell's `SceneContext::output_srgb_encode()`.
        output_srgb_encode: bool,
    ) {
        let caps = [
            glow::CULL_FACE,
            glow::DEPTH_TEST,
            glow::SCISSOR_TEST,
            glow::BLEND,
        ];
        unsafe {
            if self.shown != lines {
                let mesh = mesh(lines);
                let bytes = crate::terrain_renderer::slice_bytes(&mesh);
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::DYNAMIC_DRAW);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                crate::gpu_counters::gpu_counters().uploaded(bytes.len());
                self.vertices = (mesh.len() / STRIDE) as i32;
                self.shown = lines.to_vec();
            }
            let was = caps.map(|cap| gl.is_enabled(cap));
            for cap in caps {
                gl.disable(cap);
            }
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
            self.program
                .set_uniform_matrix4(gl, &self.transform, &transform);
            self.program.set_uniform_1f(gl, &self.alpha, alpha);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            self.program.set_uniform_1i(gl, &self.atlas_uniform, 0);
            self.output
                .set(&self.program, gl, output_srgb_encode, OutputAlpha::Blend);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, self.vertices);
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xr(menu: bool, a: bool, b: bool) -> XrInputSnapshot {
        let mut xr = XrInputSnapshot::default();
        xr.left.menu_pressed = menu;
        xr.right.primary_pressed = a;
        xr.right.secondary_pressed = b;
        xr
    }

    #[test]
    fn holding_menu_starts_pairing_once_per_hold() {
        let t0 = Instant::now();
        let mut g = Gestures::default();
        assert_eq!(g.step(Some(&xr(true, false, false)), false, t0), None);
        assert_eq!(
            g.step(Some(&xr(true, false, false)), false, t0 + HOLD / 2),
            None
        );
        assert_eq!(
            g.step(Some(&xr(true, false, false)), false, t0 + HOLD),
            Some(Gesture::Pair)
        );
        assert_eq!(
            g.step(Some(&xr(true, false, false)), false, t0 + HOLD * 3),
            None
        );
        // A short press is the game's.
        let t1 = t0 + HOLD * 4;
        assert_eq!(g.step(Some(&xr(false, false, false)), false, t1), None);
        assert_eq!(g.step(Some(&xr(true, false, false)), false, t1), None);
        assert_eq!(
            g.step(Some(&xr(false, false, false)), false, t1 + HOLD / 2),
            None
        );
        assert_eq!(g.step(None, false, t1 + HOLD * 2), None);
    }

    #[test]
    fn a_and_b_answer_only_while_open_and_only_on_a_press() {
        let t = Instant::now();
        let mut g = Gestures::default();
        // Held before the overlay opened: not a press now.
        assert_eq!(g.step(Some(&xr(false, true, false)), false, t), None);
        assert_eq!(g.step(Some(&xr(false, true, false)), true, t), None);
        assert_eq!(g.step(Some(&xr(false, false, false)), true, t), None);
        assert_eq!(
            g.step(Some(&xr(false, true, false)), true, t),
            Some(Gesture::Matches)
        );
        assert_eq!(g.step(Some(&xr(false, true, false)), true, t), None);
        assert_eq!(
            g.step(Some(&xr(false, true, true)), true, t),
            Some(Gesture::Cancel)
        );
        // Closed, A is the game's.
        assert_eq!(g.step(Some(&xr(false, false, false)), false, t), None);
        assert_eq!(g.step(Some(&xr(false, true, false)), false, t), None);
    }

    #[test]
    fn the_game_keeps_poses_but_loses_presses_while_open() {
        let mut sample = xr(true, true, true);
        sample.right.active = true;
        sample.right.trigger = 1.0;
        sample.right.aim = Some(TrackingPose::default());
        without_presses(&mut sample);
        assert!(sample.right.active && sample.right.aim.is_some());
        assert_eq!(sample.right.trigger, 0.0);
        assert!(
            !sample.left.menu_pressed
                && !sample.right.primary_pressed
                && !sample.right.secondary_pressed
        );
    }

    #[test]
    fn the_code_reads_in_two_groups_and_the_panel_fits_its_lines() {
        assert_eq!(spaced("482913"), "482 913");
        let code = PairingState::Code {
            desktop: "Mac".into(),
            code: "482913".into(),
            confirmed: false,
        };
        let lines = lines(&code);
        assert_eq!(lines[0].text, "PAIR WITH MAC");
        assert_eq!(lines[1].text, "482 913");
        assert!(lines[3].text.contains("Matches"));
        let mesh = mesh(&lines);
        // Background plus one quad per visible glyph (spaces draw nothing).
        let glyphs: usize = lines
            .iter()
            .map(|l| l.text.chars().filter(|c| *c != ' ').count())
            .sum();
        assert_eq!(mesh.len(), (1 + glyphs) * 6 * STRIDE);
        // Every glyph lies inside the background.
        let xs: Vec<f32> = mesh.chunks(STRIDE).map(|v| v[0]).collect();
        let ys: Vec<f32> = mesh.chunks(STRIDE).map(|v| v[1]).collect();
        let (bw, bh) = (xs[1] - xs[0], ys[2] - ys[0]);
        assert!(
            xs.iter().all(|x| x.abs() <= bw / 2.0 + 1e-6)
                && ys.iter().all(|y| y.abs() <= bh / 2.0 + 1e-6)
        );
        let confirmed = PairingState::Code {
            desktop: "Mac".into(),
            code: "482913".into(),
            confirmed: true,
        };
        assert!(super::lines(&confirmed)[2].text.contains("Allow"));
    }

    fn stick(click: bool, b: bool) -> XrInputSnapshot {
        let mut xr = XrInputSnapshot::default();
        xr.right.thumbstick_pressed = click;
        xr.right.secondary_pressed = b;
        xr
    }

    #[test]
    fn holding_the_right_stick_talks_and_a_short_click_is_the_games() {
        let t = Instant::now();
        let mut ptt = PushToTalk::default();
        let seen = |ptt: &mut PushToTalk, click, b| {
            let mut xr = stick(click, b);
            ptt.game_view(&mut xr, true);
            (xr.right.thumbstick_pressed, xr.right.secondary_pressed)
        };
        // Held: the game sees nothing; past the hold it talks.
        assert_eq!(ptt.step(Some(&stick(true, false)), true, t), None);
        assert_eq!(seen(&mut ptt, true, false), (false, false));
        assert_eq!(
            ptt.step(Some(&stick(true, false)), true, t + TALK_HOLD),
            Some(Talk::Start)
        );
        assert_eq!(ptt.step(Some(&stick(true, false)), true, t + TALK_HOLD * 4), None);
        // B is ours while talking.
        assert_eq!(seen(&mut ptt, true, true), (false, false));
        assert_eq!(ptt.step(Some(&stick(false, false)), true, t + TALK_HOLD * 5), Some(Talk::Send));
        assert_eq!(seen(&mut ptt, false, true), (false, true), "released: no tap");

        // A short click reaches the game on release, once.
        assert_eq!(ptt.step(Some(&stick(true, false)), true, t), None);
        assert_eq!(ptt.step(Some(&stick(false, false)), true, t + TALK_HOLD / 2), None);
        let mut xr = stick(false, false);
        ptt.game_view(&mut xr, false);
        assert!(!xr.right.thumbstick_pressed, "kept for a frame that steps");
        assert_eq!(seen(&mut ptt, false, false), (true, false));
        assert_eq!(seen(&mut ptt, false, false), (false, false));

        // Unpaired, the click is the game's alone.
        assert_eq!(ptt.step(Some(&stick(true, false)), false, t), None);
        assert_eq!(ptt.step(Some(&stick(true, false)), false, t + TALK_HOLD * 2), None);
        assert_eq!(seen(&mut ptt, true, false), (true, false));
    }

    #[test]
    fn b_cancels_and_the_hold_must_end_before_talking_again() {
        let t = Instant::now();
        let mut ptt = PushToTalk::default();
        ptt.step(Some(&stick(true, false)), true, t);
        assert_eq!(ptt.step(Some(&stick(true, false)), true, t + TALK_HOLD), Some(Talk::Start));
        assert_eq!(
            ptt.step(Some(&stick(true, true)), true, t + TALK_HOLD * 2),
            Some(Talk::Cancel)
        );
        assert_eq!(ptt.step(Some(&stick(true, false)), true, t + TALK_HOLD * 9), None);
        // Released after a cancel: neither a send nor a tap.
        assert_eq!(ptt.step(Some(&stick(false, false)), true, t + TALK_HOLD * 10), None);
        let mut xr = stick(false, false);
        ptt.game_view(&mut xr, true);
        assert!(!xr.right.thumbstick_pressed);
        // The shell's own stop (the time limit) waits for release too.
        ptt.step(Some(&stick(true, false)), true, t);
        ptt.step(Some(&stick(true, false)), true, t + TALK_HOLD);
        ptt.stop();
        assert_eq!(ptt.step(Some(&stick(false, false)), true, t + TALK_HOLD * 2), None);
    }

    #[test]
    fn the_toast_walks_its_stages_shows_the_transcript_a_while_and_fades() {
        let t = Instant::now();
        let mut toast = Toast::default();
        assert!(toast.look(t).is_none());
        toast.show(
            VoiceStage::Listening {
                level: 5,
                seconds_left: None,
            },
            t,
        );
        let (shown, alpha) = toast.look(t + Duration::from_secs(50)).unwrap();
        assert_eq!(alpha, 1.0, "listening stays");
        let lines = toast_lines(shown);
        assert_eq!(lines[0].text, "LISTENING...");
        assert_eq!(lines[1].text, "|||||.......");
        assert!(lines[2].text.contains("Cancel"));
        let countdown = VoiceToast {
            stage: VoiceStage::Listening {
                level: 0,
                seconds_left: Some(3),
            },
            transcript: None,
        };
        assert_eq!(toast_lines(&countdown)[2].text, "Sending in 3");

        toast.show(VoiceStage::Heard, t);
        toast.transcript("make the ball red".into(), t);
        toast.show(VoiceStage::Working, t + Duration::from_secs(1));
        let lines = toast_lines(toast.look(t + Duration::from_secs(2)).unwrap().0);
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["WORKING...", "\"make the ball red\""]
        );
        let later = toast.look(t + TRANSCRIPT).unwrap().0;
        assert_eq!(later.transcript, None, "the transcript shows a while");
        assert_eq!(later.stage, VoiceStage::Working);

        let done = t + Duration::from_secs(30);
        toast.show(VoiceStage::Reloaded, done);
        assert_eq!(toast.look(done + OUTCOME).unwrap().1, 1.0);
        let fading = toast.look(done + OUTCOME + FADE / 2).unwrap().1;
        assert!((fading - 0.5).abs() < 0.01, "{fading}");
        assert!(toast.look(done + OUTCOME + FADE).is_none());
        assert!(toast.stage().is_none(), "hidden holds nothing");

        // Idle from the desktop fades it now.
        toast.show(VoiceStage::Working, t);
        toast.hide(t);
        assert!(toast.look(t + FADE).is_none());
    }

    #[test]
    fn a_long_transcript_wraps_into_a_few_lines() {
        assert_eq!(wrap("make the ball red", 34, 3), ["make the ball red"]);
        assert_eq!(
            wrap("one two three four", 9, 3),
            ["one two", "three", "four"]
        );
        assert_eq!(wrap("abcdefghijkl", 5, 3), ["abcde", "fghij", "kl"]);
        let long = wrap(&"word ".repeat(100), 34, 3);
        assert_eq!(long.len(), 3);
        assert!(long[2].ends_with("...") && long[2].chars().count() <= 34);
        let lines = toast_lines(&VoiceToast {
            stage: VoiceStage::Heard,
            transcript: Some("é".repeat(2000)),
        });
        assert!(lines.len() == 4 && lines.iter().all(|l| l.text.chars().count() <= 34));
    }

    #[test]
    fn the_toast_follows_the_head_lazily() {
        let yaw = |deg: f32| {
            let half = deg.to_radians() / 2.0;
            [0.0, half.sin(), 0.0, half.cos()]
        };
        // Where the toast's centre sits in head space.
        let centre = |m: Matrix4<f32>| {
            let p = m * cgmath::vec4(0.0, 0.0, 0.0, 1.0);
            p.x.atan2(-p.z).to_degrees()
        };
        let mut follow = Follow::default();
        assert!(centre(follow.transform(yaw(0.0), 0.01)).abs() < 1e-3);
        // A small turn: it stays put in the world (so moves in view).
        let offset = centre(follow.transform(yaw(15.0), 0.01));
        assert!((offset - 15.0).abs() < 0.1, "{offset}");
        // A big turn: it eases back in front.
        let first = centre(follow.transform(yaw(60.0), 0.01));
        assert!(first > 50.0, "{first}");
        let mut last = first;
        for _ in 0..200 {
            last = centre(follow.transform(yaw(60.0), 0.016));
        }
        assert!(last.abs() < 2.5, "{last}");
        follow.reset();
        assert!(centre(follow.transform(yaw(90.0), 0.01)).abs() < 1e-3);
    }
}
