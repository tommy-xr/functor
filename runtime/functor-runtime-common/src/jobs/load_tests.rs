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
        game.poll_load();
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
    let source = SLOW.replace("TOTAL", "500000.0");
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
    assert!(model.contains("sum: 124999750000"), "{model}");
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

#[test]
fn a_push_during_a_load_restarts_it_with_the_new_files() {
    let mut game = boot();
    let first = SLOW.replace("TOTAL", "300000.0");
    game.begin_load_project(&[("game.fun".to_string(), first)]).expect("links");
    // Whoever waits on the load (the shell's `/load-project`) hears when the
    // restarted one lands.
    let (done, landed) = std::sync::mpsc::channel();
    game.when_loaded(Box::new(move |status| {
        let _ = done.send(status);
    }));
    let second = SLOW.replace("TOTAL", "1000.0");
    let status = game.reload_project(&[("game.fun".to_string(), second)]).expect("links");
    assert!(status.contains("off the frame thread"), "{status}");
    let run = run(&mut game);
    run.status.expect("loads");
    assert!(game.state_debug().contains("sum: 499500"), "the pushed files: {}", game.state_debug());
    assert!(landed.try_recv().expect("told").is_ok());
}

/// `slow` again each frame, so a reload's new value shows; `frames` counts
/// ticks, so a kept model keeps counting.
const PLAYING: &str = r#"
let total = TOTAL
let slow = List.fold((acc, i) => Job.report(i / total, acc + i), 0.0, List.range(total))
let init = { sum: 0.0, frames: 0.0 }
let tick = (m, dt, tts) => { sum: slow, frames: m.frames + 1.0 }
let draw = (m, tts) => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
"#;

fn frames(game: &FunctorLangEmbeddedGame) -> f64 {
    let model = game.state_debug();
    let at = model.find("frames: ").expect("frames") + "frames: ".len();
    model[at..].split([',', ' ', '}']).next().unwrap().parse().expect("a number")
}

/// What a paced 60 Hz loop saw while a reload ran on the loader: frames
/// the old program ticked, the longest gap (the push's frame included),
/// and how the reload ended.
fn reload_playing(game: &mut FunctorLangEmbeddedGame, push: String) -> (f64, Duration, Result<String, String>) {
    let started = Instant::now();
    let mut last = Instant::now();
    let mut longest_gap = Duration::ZERO;
    let (done, landed) = std::sync::mpsc::channel();
    let before = frames(game);
    let mut ticked_while_reloading = 0.0;
    let mut status = None;
    for frame in 1.. {
        let time = FrameTime { tts: frame as f32 / 60.0, dts: 1.0 / 60.0 };
        game.check_hot_reload(time.clone());
        if frame == 3 {
            let begun = game.reload_project(&[("game.fun".to_string(), push.clone())]).expect("links");
            assert!(begun.contains("off the frame thread"), "{begun}");
            let done = done.clone();
            game.when_loaded(Box::new(move |status| {
                let _ = done.send(status);
            }));
        }
        game.tick(time.clone());
        let _ = game.render_at(time, 0.0);
        if frame >= 3 && status.is_none() {
            ticked_while_reloading = frames(game) - before;
        }
        status = status.or_else(|| landed.try_recv().ok());
        let now = Instant::now();
        longest_gap = longest_gap.max(now - last);
        last = now;
        if status.is_some() {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(120), "the reload never landed");
        if let Some(rest) = (started + BUDGET * frame).checked_duration_since(Instant::now()) {
            std::thread::sleep(rest);
        }
    }
    (ticked_while_reloading, longest_gap, status.expect("ended above"))
}

#[test]
fn a_heavy_reload_runs_off_the_frame_while_the_old_program_plays() {
    let source = PLAYING.replace("TOTAL", "500000.0");
    let mut game = FunctorLangEmbeddedGame::create(vec![("game.fun".to_string(), source)], Box::new(NativePlatform))
        .expect("loads")
        .load_in_background();
    let (ticked, longest_gap, status) = reload_playing(&mut game, PLAYING.replace("TOTAL", "510000.0"));
    let status = status.expect("reloads");
    eprintln!(
        "reload: {status}; the old program ticked {ticked} frames meanwhile, longest gap {:.1} ms",
        longest_gap.as_secs_f64() * 1000.0
    );
    assert!(status.contains("model preserved"), "{status}");
    assert!(ticked >= 5.0, "the old program ticked {ticked} frames");
    // The push's own frame links and type-checks the project before the
    // loader takes over: ~30 ms in a debug build, a few in release.
    let limit = if cfg!(debug_assertions) { BUDGET * 3 } else { BUDGET * 2 };
    assert!(longest_gap <= limit, "a {:.1} ms gap between frames", longest_gap.as_secs_f64() * 1000.0);
    // The model kept counting, and the next tick runs the new `slow`.
    let ft = FrameTime { tts: 100.0, dts: 1.0 / 60.0 };
    game.tick(ft);
    let model = game.state_debug();
    assert!(model.contains("sum: 130049745000"), "{model}");
    assert!(frames(&game) > ticked, "{model}");
}

#[test]
fn a_heavy_reload_that_fails_keeps_the_old_program() {
    let source = PLAYING.replace("TOTAL", "500000.0");
    let mut game = FunctorLangEmbeddedGame::create(vec![("game.fun".to_string(), source)], Box::new(NativePlatform))
        .expect("loads")
        .load_in_background();
    let broken = PLAYING.replace("TOTAL", "510000.0").replace("acc + i", "acc + i + Text.length(1.0)");
    let (ticked, _, status) = reload_playing(&mut game, broken);
    let error = status.expect_err("fails");
    assert!(error.contains("game.fun:"), "{error}");
    assert!(ticked >= 1.0);
    let ft = FrameTime { tts: 100.0, dts: 1.0 / 60.0 };
    game.tick(ft);
    assert!(game.state_debug().contains("sum: 124999750000"), "the old program: {}", game.state_debug());
}

#[test]
fn a_light_reload_stays_on_the_frame() {
    let source = PLAYING.replace("TOTAL", "500000.0");
    let mut game = FunctorLangEmbeddedGame::create(vec![("game.fun".to_string(), source.clone())], Box::new(NativePlatform))
        .expect("loads")
        .load_in_background();
    let edited = source.replace("Vec3.make(0.0, 0.0, -5.0)", "Vec3.make(0.0, 1.0, -5.0)");
    let status = game.reload_project(&[("game.fun".to_string(), edited)]).expect("reloads");
    assert!(status.starts_with("reloaded"), "{status}");
    assert!(!game.load_pending());
}

#[test]
fn a_newer_push_that_doesnt_compile_still_supersedes_a_heavy_reload() {
    let source = PLAYING.replace("TOTAL", "500000.0");
    let mut game = FunctorLangEmbeddedGame::create(vec![("game.fun".to_string(), source)], Box::new(NativePlatform))
        .expect("loads")
        .load_in_background();
    let heavy = PLAYING.replace("TOTAL", "510000.0");
    let begun = game.reload_project(&[("game.fun".to_string(), heavy)]).expect("links");
    assert!(begun.contains("off the frame thread"), "{begun}");
    assert!(game.load_pending());
    assert!(game.reload_project(&[("game.fun".to_string(), "let init = (".to_string())]).is_err());
    assert!(!game.load_pending(), "the older reload stopped");
}

#[test]
fn a_reload_that_newly_reads_a_file_runs_off_the_frame() {
    let source = PLAYING.replace("TOTAL", "10.0");
    let mut game = FunctorLangEmbeddedGame::create(vec![("game.fun".to_string(), source.clone())], Box::new(NativePlatform))
        .expect("loads")
        .load_in_background();
    let reading = format!("{source}\nlet there = File.exists(Path.of(\"level.bin\"))\n");
    let status = game.reload_project(&[("game.fun".to_string(), reading)]).expect("links");
    assert!(status.contains("off the frame thread"), "{status}");
    let run = run(&mut game);
    assert!(run.status.expect("reloads").contains("model preserved"));
}
