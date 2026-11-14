//! The headset overlay's painter on the desktop, the way a Quest eye sees
//! it: an sRGB eye buffer (hardware encode, linear blending) at Quest-like
//! pixels per degree, the pairing card, the voice toast and the panel at
//! several distances and angles. Pins the pairing card's pixels to a golden
//! (`LINK_OVERLAY_UPDATE=1` rewrites it).
//!
//! `LINK_OVERLAY_OUT=<dir>` writes each view as a PNG and prints the
//! painter's CPU (shapes + mesh) and GPU (draw) cost. Self-skipping like
//! `reload_indicator` (needs a GL display; GLFW on the main thread):
//!
//! ```sh
//! LINK_OVERLAY_OUT=/tmp/overlay cargo test --release -p functor-runtime-desktop \
//!   --test link_overlay -- --ignored --nocapture
//! ```

use std::time::Instant;

use cgmath::{Deg, Matrix4};
use functor_runtime_common::debug_protocol::{PairingState, TaskStance, VoiceStage, VoiceToast};
use functor_runtime_common::XrInputSnapshot;
use functor_runtime_common::link_overlay::{card, lines, mesh, toast_lines, LinkOverlayPainter, Shape};
use functor_runtime_common::link_panel::{
    Connection, ConversationData, Destination, Panel, PanelData, PanelDecision, PanelRoute,
    PanelTask, Role, SaidLine,
};
use functor_runtime_common::reload_indicator::head_to_clip;
use functor_runtime_common::TrackingPose;
use glow::HasContext;

const SIZE: (u32, u32) = (1200, 800);
/// Pixels per degree at the view's centre: about a Quest 3 eye buffer's.
const PPD: f32 = 20.0;
/// The panel's background, for the GL check.
const PANEL_BG: [u8; 3] = [21, 26, 33];

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("link_overlay: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let (gl, _window, _glfw) = gl_context();
    framebuffer(&gl);
    let mut painter = LinkOverlayPainter::new(&gl, "#version 330 core");
    the_panel_paints_its_background_and_text(&gl, &mut painter);
    println!("test the_panel_paints_its_background_and_text ... ok");
    the_pairing_card_matches_its_golden(&gl, &mut painter);
    println!("test the_pairing_card_matches_its_golden ... ok");
    if let Ok(dir) = std::env::var("LINK_OVERLAY_OUT") {
        write_views(&gl, &mut painter, &dir);
    }
}

/// Opaque background where the panel is, lighter text pixels inside it,
/// nothing outside it.
fn the_panel_paints_its_background_and_text(gl: &glow::Context, painter: &mut LinkOverlayPainter) {
    let shapes = panel_view("now");
    let pixels = draw(gl, painter, &shapes, at(0.9, 0.0), Background::Black);
    let px = |x: u32, y: u32| {
        let i = ((y * SIZE.0 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    assert_eq!(px(5, 5), [0, 0, 0], "outside the panel");
    // Just inside its bottom-left corner: the background, exactly.
    let (cx, cy) = (SIZE.0 / 2, SIZE.1 / 2);
    let half = |m: f32| (m.atan2(0.9).to_degrees() * PPD) as u32;
    let corner = px(cx - half(0.37), cy - half(0.26));
    assert!(
        corner.iter().zip(PANEL_BG).all(|(g, w)| g.abs_diff(w) <= 2),
        "background {corner:?}"
    );
    let lit = (0..SIZE.1)
        .flat_map(|y| (0..SIZE.0).map(move |x| (x, y)))
        .filter(|&(x, y)| px(x, y).iter().any(|&c| c > 200))
        .count();
    assert!(lit > 2000, "text pixels: {lit}");
}

/// The pairing card's pixels against `fixtures/link_overlay_pairing.png`
/// (renderer-specific, like the example goldens); `LINK_OVERLAY_UPDATE=1`
/// rewrites it.
fn the_pairing_card_matches_its_golden(gl: &glow::Context, painter: &mut LinkOverlayPainter) {
    let pixels = flip(&draw(gl, painter, &card(&lines(&pairing())), at(1.0, 0.0), Background::Black));
    // The card, with a margin.
    let (x0, y0, w, h) = (300, 240, 600, 320);
    let crop: Vec<u8> = (y0..y0 + h)
        .flat_map(|y| {
            let start = ((y * SIZE.0 + x0) * 4) as usize;
            pixels[start..start + (w * 4) as usize].to_vec()
        })
        .collect();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/link_overlay_pairing.png");
    if std::env::var_os("LINK_OVERLAY_UPDATE").is_some() {
        let png = functor_runtime_common::frame_capture::encode_rgba_png(w, h, &crop).unwrap();
        std::fs::write(path, png).unwrap();
    }
    let want = image::open(path).expect("the golden (LINK_OVERLAY_UPDATE=1 writes it)").to_rgba8();
    assert_eq!((want.width(), want.height()), (w, h));
    // Edge pixels may differ by a driver's rounding; the glyphs may not.
    let off = crop
        .chunks(4)
        .zip(want.as_raw().chunks(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 24))
        .count();
    assert!(off * 200 < (w * h) as usize, "{off} pixels differ from {path}");
}

fn pairing() -> PairingState {
    PairingState::Code {
        desktop: "MacBook-Air".into(),
        code: "482913".into(),
        confirmed: false,
    }
}

fn write_views(gl: &glow::Context, painter: &mut LinkOverlayPainter, dir: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let mut save = |name: &str, shapes: &[Shape], transform: Matrix4<f32>, bg: Background| {
        let pixels = draw(gl, painter, shapes, transform, bg);
        let png = functor_runtime_common::frame_capture::encode_rgba_png(SIZE.0, SIZE.1, &flip(&pixels))
            .expect("encode");
        std::fs::write(format!("{dir}/{name}.png"), png).unwrap();
    };
    save("pairing", &card(&lines(&pairing())), at(1.0, 0.0), Background::Black);
    let listening = VoiceToast {
        stage: VoiceStage::Listening { level: 7, seconds_left: None },
        transcript: None,
    };
    save("toast-listening", &card(&toast_lines(&listening)), at(1.0, 0.0), Background::Bright);
    let heard = VoiceToast {
        stage: VoiceStage::Working,
        transcript: Some(TRANSCRIPT.into()),
    };
    save("toast-transcript", &card(&toast_lines(&heard)), at(1.0, 0.0), Background::Bright);
    for view in ["now", "tasks", "decision", "talk", "conversation", "cannot-reach"] {
        save(&format!("panel-{view}"), &panel_view(view), at(0.9, 0.0), Background::Black);
    }
    let decision = panel_view("decision");
    for (name, distance, yaw) in [
        ("near-0.6m", 0.6, 0.0),
        ("far-1.5m", 1.5, 0.0),
        ("far-2.5m", 2.5, 0.0),
        ("angle-45", 0.9, 45.0),
        ("angle-65", 0.9, 65.0),
    ] {
        save(&format!("decision-{name}"), &decision, at(distance, yaw), Background::Black);
    }

    // Cost: the shell rebuilds shapes every frame and the mesh on change.
    let data = data();
    let panel = panel_at("decision", &data);
    let connection = mac();
    let toast = toast();
    let n = 2000;
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(panel.shapes(Some(&data), &connection, Some(&toast), Instant::now()));
    }
    let shapes_us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(mesh(&decision));
    }
    let mesh_us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    let gpu_us = gpu_time(gl, painter, &decision);
    println!(
        "panel (decision): shapes {shapes_us:.1} us/frame, mesh {mesh_us:.1} us/change, {} vertices, GPU+driver draw {gpu_us:.1} us",
        mesh(&decision).len() / 8
    );
    println!("wrote views to {dir}");
}

const TRANSCRIPT: &str = "Make the ball bounce a little higher off the left wall, and \
    give it a short trail so I can follow it when it gets fast.";

fn mac() -> Connection {
    Connection::Connected { desktop: "MacBook-Air".into() }
}

fn toast() -> VoiceToast {
    VoiceToast {
        stage: VoiceStage::Listening { level: 7, seconds_left: None },
        transcript: None,
    }
}

fn data() -> PanelData {
    let task = |id, title: &str, stance, activity: &str, decision| PanelTask {
        id,
        title: title.into(),
        stance,
        activity: activity.into(),
        decision,
    };
    PanelData {
        project: Some("vr-lab".into()),
        placed: true,
        now: Some(1),
        tasks: vec![
            task(
                1,
                "Make the ball bounce higher off the left wall",
                TaskStance::Waiting,
                "Edited physics.fun; ran 14 tests",
                Some(PanelDecision {
                    tree: "tree-1".into(),
                    size: "+12 -3, 2 files, game logic".into(),
                    glance: false,
                    reason: "It changes how every wall bounces, not only the left one.".into(),
                    summary: "Raised the wall restitution from 0.6 to 0.85 and added a short \
                              trail to the ball, faded over 0.4 seconds."
                        .into(),
                    notes: vec!["Not tried with two balls.".into(), "The trail costs one draw.".into()],
                }),
            ),
            task(2, "Add a score counter above the net", TaskStance::Working, "Reading ui.fun", None),
            task(3, "Quieter footsteps on the carpet", TaskStance::Queued, "", None),
        ],
        waiting: "Waiting for your decision on \u{201c}Make the ball bounce\u{2026}\u{201d}".into(),
        route: Some(PanelRoute {
            id: 9,
            line: "Queued behind \u{201c}Add a score counter\u{201d}".into(),
            moves: vec![
                ("Steer \u{201c}Add a score counter\u{201d} instead".into(), Destination::Steer { task: 2 }),
                ("Make it a new task".into(), Destination::New),
            ],
        }),
        conversation: Some(conversation()),
        progress: None,
    }
}

/// Task 1's conversation: a spoken request, a route, the agent's replies.
fn conversation() -> ConversationData {
    let said = |role, spoken, text: &str| SaidLine {
        role,
        spoken,
        cut: false,
        text: text.into(),
    };
    ConversationData {
        task: 1,
        title: "Make the ball bounce higher off the left wall".into(),
        stance: TaskStance::Waiting,
        activity: String::new(),
        question: false,
        first: 0,
        items: vec![
            said(Role::You, true, TRANSCRIPT),
            said(Role::App, false, "Steered \u{201c}Make the ball bounce higher\u{201d}"),
            said(
                Role::Agent,
                false,
                "I raised the wall restitution from 0.6 to 0.85 in physics.fun and added a \
                 trail to the ball that fades over 0.4 seconds. Every wall bounces higher now, \
                 not only the left one \u{2014} say if you want just the left.",
            ),
            said(Role::You, false, "Only the left wall, please."),
            said(Role::Agent, false, "Done: the left wall alone uses 0.85; the others keep 0.6."),
        ],
    }
}

/// A panel opened and driven to `view` with the right controller.
fn panel_at(view: &str, data: &PanelData) -> Panel {
    let mut panel = Panel::default();
    panel.toggle();
    let mut press = |set: fn(&mut XrInputSnapshot)| {
        let mut xr = XrInputSnapshot::default();
        set(&mut xr);
        panel.step(Some(&xr), Some(data), false);
        panel.step(Some(&XrInputSnapshot::default()), Some(data), false);
    };
    let down: fn(&mut XrInputSnapshot) = |xr| xr.right.thumbstick = [0.0, -1.0];
    let right: fn(&mut XrInputSnapshot) = |xr| xr.right.thumbstick = [1.0, 0.0];
    let a: fn(&mut XrInputSnapshot) = |xr| xr.right.primary_pressed = true;
    match view {
        "tasks" => {
            press(down);
            press(down);
            press(right);
        }
        "decision" => {
            press(down);
            press(down);
            press(right);
            press(a);
        }
        "talk" => press(down),
        "conversation" => {
            press(a);
            press(a);
        }
        _ => {}
    }
    panel
}

fn panel_view(view: &str) -> Vec<Shape> {
    let data = data();
    let connection = match view {
        "cannot-reach" => Connection::NotConnected { desktop: "MacBook-Air".into() },
        _ => mac(),
    };
    panel_at(view, &data).shapes(Some(&data), &connection, Some(&toast()), Instant::now())
}

/// Metres ahead of the eye, turned `yaw` degrees about its vertical axis.
fn at(distance: f32, yaw: f32) -> Matrix4<f32> {
    let half = |px: u32| ((px as f32 / 2.0) / (PPD * 180.0 / std::f32::consts::PI)).atan();
    let (h, v) = (half(SIZE.0), half(SIZE.1));
    head_to_clip([-h, h, -v, v], TrackingPose::default())
        * Matrix4::from_translation(cgmath::vec3(0.0, 0.0, -distance))
        * Matrix4::from_angle_y(Deg(yaw))
}

#[derive(Clone, Copy)]
enum Background {
    Black,
    /// A bright game behind: sky blue left, warm white right.
    Bright,
}

/// One frame into the sRGB eye buffer; its bytes, bottom row first.
fn draw(
    gl: &glow::Context,
    painter: &mut LinkOverlayPainter,
    shapes: &[Shape],
    transform: Matrix4<f32>,
    bg: Background,
) -> Vec<u8> {
    unsafe {
        // Encode-on-write and linear blending, like the Quest's swapchain.
        gl.enable(glow::FRAMEBUFFER_SRGB);
        match bg {
            Background::Black => {
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            }
            Background::Bright => {
                gl.clear_color(0.79, 0.71, 0.48, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(0, 0, SIZE.0 as i32 / 2, SIZE.1 as i32);
                gl.clear_color(0.23, 0.55, 0.9, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                gl.disable(glow::SCISSOR_TEST);
            }
        }
    }
    painter.draw(gl, shapes, 1.0, transform, SIZE, false);
    let mut pixels = vec![0u8; (SIZE.0 * SIZE.1 * 4) as usize];
    unsafe {
        gl.disable(glow::FRAMEBUFFER_SRGB);
        gl.read_pixels(
            0,
            0,
            SIZE.0 as i32,
            SIZE.1 as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
    }
    pixels
}

/// Wall time of one steady-state draw (mesh already uploaded) through
/// `glFinish`, averaged: GPU plus driver. (glow cannot read a timer query
/// on macOS's GL 4.1.)
fn gpu_time(gl: &glow::Context, painter: &mut LinkOverlayPainter, shapes: &[Shape]) -> f64 {
    let transform = at(0.9, 0.0);
    draw(gl, painter, shapes, transform, Background::Black);
    let n = 500;
    unsafe {
        gl.finish();
        let t = Instant::now();
        for _ in 0..n {
            painter.draw(gl, shapes, 1.0, transform, SIZE, false);
        }
        gl.finish();
        t.elapsed().as_secs_f64() * 1e6 / n as f64
    }
}

/// Bottom-row-first RGBA as a top-row-first image.
fn flip(pixels: &[u8]) -> Vec<u8> {
    functor_runtime_common::frame_capture::flip_rgba_rows(SIZE.0, SIZE.1, pixels).unwrap()
}

/// A hidden-window GL context.
fn gl_context() -> (glow::Context, glfw::PWindow, glfw::Glfw) {
    use glfw::Context as _;
    let mut glfw = glfw::init(glfw::fail_on_errors).expect("init GLFW");
    glfw.window_hint(glfw::WindowHint::ContextVersion(4, 1));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    glfw.window_hint(glfw::WindowHint::Visible(false));
    let (mut window, _events) = glfw
        .create_window(64, 64, "link-overlay-test", glfw::WindowMode::Windowed)
        .expect("create hidden GLFW window");
    window.make_current();
    let gl = unsafe { glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _) };
    (gl, window, glfw)
}

/// Bind an sRGB eye buffer (`SRGB8_ALPHA8` + depth), as the Quest's.
fn framebuffer(gl: &glow::Context) {
    unsafe {
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        for (format, attachment) in [
            (glow::SRGB8_ALPHA8, glow::COLOR_ATTACHMENT0),
            (glow::DEPTH_COMPONENT24, glow::DEPTH_ATTACHMENT),
        ] {
            let rb = gl.create_renderbuffer().unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            gl.renderbuffer_storage(glow::RENDERBUFFER, format, SIZE.0 as i32, SIZE.1 as i32);
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, attachment, glow::RENDERBUFFER, Some(rb));
        }
        assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);
    }
}
