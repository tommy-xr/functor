//! The load job through the embedded producer: frames keep coming while a
//! slow top-level value loads off the frame thread.

use std::time::{Duration, Instant};

use crate::functor_lang_game_embedded::{FunctorLangEmbeddedGame, NativePlatform};
use crate::protocol::GameProducer;
use crate::FrameTime;

const BOOT: &str = r#"
let init = { spin: 0.0 }
let tick = (m, dt, tts) => { spin: m.spin + dt }
let draw = (m, tts) => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
"#;

/// `slow` is ~`TOTAL` interpreted steps, reporting its share done.
const SLOW: &str = r#"
let total = TOTAL
let slow = List.fold((acc, i) => Job.report(i / total, acc + i), 0.0, List.range(total))
let init = { sum: slow, frames: 0.0 }
let tick = (m, dt, tts) => { m with frames: m.frames + 1.0 }
let draw = (m, tts) => Frame.create(Camera3D.lookAt(Vec3.make(m.sum, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
"#;

fn boot() -> FunctorLangEmbeddedGame {
    FunctorLangEmbeddedGame::create(vec![("boot.fun".to_string(), BOOT.to_string())], Box::new(NativePlatform))
        .expect("boot loads")
}

/// One 60 Hz frame budget.
const BUDGET: Duration = Duration::from_micros(16_667);

/// What a paced 60 Hz loop saw: frames while loading, the longest gap
/// between frames, the progress shown each loading frame, and the status.
struct Run {
    loading_frames: usize,
    longest_gap: Duration,
    progress: Vec<f64>,
    status: Result<String, String>,
}

/// Drive `game` at 60 Hz until the load in flight lands, plus a few frames.
fn run(game: &mut FunctorLangEmbeddedGame) -> Run {
    let started = Instant::now();
    let mut last = Instant::now();
    let (mut loading_frames, mut longest_gap, mut progress) = (0, Duration::ZERO, Vec::new());
    let (done, landed) = std::sync::mpsc::channel();
    game.when_loaded(Box::new(move |status| {
        let _ = done.send(status);
    }));
    let mut status = None;
    let mut after = 0;
    for frame in 1.. {
        let time = FrameTime { tts: frame as f32 / 60.0, dts: 1.0 / 60.0 };
        game.check_hot_reload(time.clone());
        game.tick(time.clone());
        let _ = game.render_at(time, 0.0);
        if let Some(screen) = game.load_screen() {
            loading_frames += 1;
            progress.push(screen.shown());
        }
        status = status.or_else(|| landed.try_recv().ok());
        let now = Instant::now();
        longest_gap = longest_gap.max(now - last);
        last = now;
        if status.is_some() {
            after += 1;
            if after == 5 {
                break;
            }
        }
        assert!(started.elapsed() < Duration::from_secs(120), "the load never finished");
        if let Some(rest) = (started + BUDGET * frame).checked_duration_since(Instant::now()) {
            std::thread::sleep(rest);
        }
    }
    Run { loading_frames, longest_gap, progress, status: status.expect("ended above") }
}

#[test]
fn a_slow_load_keeps_frames_coming_and_its_progress_only_rises() {
    let mut game = boot();
    let source = SLOW.replace("TOTAL", "200000.0");
    let status = game.begin_load_project(&[("game.fun".to_string(), source)]).expect("links");
    assert!(status.contains("off the frame thread"), "{status}");
    assert!(game.load_pending());
    let run = run(&mut game);
    let status = run.status.expect("loads");
    eprintln!(
        "load: {status}; {} loading frames, longest gap {:.1} ms",
        run.loading_frames,
        run.longest_gap.as_secs_f64() * 1000.0
    );
    assert!(run.loading_frames >= 5, "{} frames while loading", run.loading_frames);
    assert!(
        run.longest_gap <= BUDGET * 2,
        "a {:.1} ms gap between frames",
        run.longest_gap.as_secs_f64() * 1000.0
    );
    assert!(run.progress.windows(2).all(|w| w[0] <= w[1]), "{:?}", run.progress);
    assert!(run.progress.last().is_some_and(|p| *p > 0.5), "{:?}", run.progress);
    // The loaded game runs, with the value the loader computed.
    let model = game.state_debug();
    assert!(model.contains("sum: 19999900000"), "{model}");
    assert!(!model.contains("frames: 0"), "it ticks: {model}");
}

#[test]
fn a_failed_load_reports_and_keeps_the_running_program() {
    let mut game = boot();
    let broken = SLOW.replace("TOTAL", "20000.0").replace("acc + i", "acc + i + Text.length(1.0)");
    game.begin_load_project(&[("game.fun".to_string(), broken)]).expect("links");
    let run = run(&mut game);
    let error = run.status.expect_err("the load fails");
    assert!(error.contains("game.fun:"), "located: {error}");
    assert!(!game.load_pending());
    assert!(game.state_debug().contains("spin"), "the boot program runs on");
}

#[test]
fn a_link_error_returns_at_once() {
    let mut game = boot();
    let error = game
        .begin_load_project(&[("game.fun".to_string(), "let init = (".to_string())])
        .expect_err("doesn't parse");
    assert!(error.contains("game.fun"), "{error}");
    assert!(!game.load_pending());
}

#[test]
fn the_games_loading_hook_draws_the_screen() {
    let mut game = boot();
    let source = format!(
        "{}\nlet loading = (p, tts) => Frame.create(Camera3D.lookAt(Vec3.make(42.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n",
        SLOW.replace("TOTAL", "200000.0")
    );
    game.begin_load_project(&[("game.fun".to_string(), source)]).expect("links");
    let time = FrameTime { tts: 0.0, dts: 1.0 / 60.0 };
    let frame = game.render_at(time, 0.0);
    assert!(game.load_pending(), "still loading");
    assert!(format!("{:?}", frame.camera).contains("42.0"), "the hook's camera: {:?}", frame.camera);
}
