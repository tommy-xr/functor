//! `Job.run` end to end, through the embedded producer: results, phase
//! rules, refusals, cancel, reload and replay. Lifecycle tests hold jobs on
//! [`FakeJobs`] and complete them on the test thread.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::*;
use crate::functor_lang_game_embedded::{FunctorLangEmbeddedGame, NativePlatform};
use crate::protocol::GameProducer;
use crate::FrameTime;

/// Asks for `work(input)` under key "k" on frame 2 (so a replay from frame 0
/// holds the delivery), and keeps the result or the error's message.
fn game_source(work: &str, input: &str) -> String {
    format!(
        r#"
type Msg = | Done(r: Result.t<float, Job.error>)
let scale = 2.0
let unrelated = 1.0
{work}
let init = {{ n: 0.0, got: -1.0, err: "" }}
let update = (m, msg) =>
  match msg with
  | Done(r) =>
    (match r with
     | Result.Ok(v) => {{ m with got: v }}
     | Result.Error(e) => {{ m with err: Job.describe(e) }})
let tick = (m, dt, tts) =>
  let next = {{ m with n: m.n + 1.0 }} in
  if m.n == 2.0 then (next, Job.run("k", work, {input}, Done)) else next
let draw = (m, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(m.got, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
"#
    )
}

const DOUBLE: &str = "let work = (x) => x * scale";

struct Rig {
    game: FunctorLangEmbeddedGame,
    fake: Rc<FakeJobs>,
    frames: u32,
    dir: tempfile::TempDir,
}

impl Rig {
    fn new(source: &str) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("data")).unwrap();
        std::fs::write(dir.path().join("data/t.bin"), [1u8; 3]).unwrap();
        let mut game = FunctorLangEmbeddedGame::create(
            vec![("game.fun".to_string(), source.to_string())],
            Box::new(NativePlatform),
        )
        .expect("loads");
        let fake = Rc::new(FakeJobs::default());
        game.job_port().set_dispatch(fake.clone());
        let reader = crate::storage::ProjectDirReader(dir.path().to_path_buf());
        game.job_port().set_reader(Some(Arc::new(reader)));
        Rig { game, fake, frames: 0, dir }
    }

    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            let t = self.frames as f32 / 60.0;
            self.game.tick(FrameTime { tts: t, dts: 1.0 / 60.0 });
            self.frames += 1;
        }
    }

    /// Complete every held job, then play a frame to deliver.
    fn finish(&mut self) {
        for ticket in self.fake.held() {
            self.fake.complete(ticket);
        }
        self.tick(1);
    }

    fn model(&self) -> String {
        self.game.state_debug()
    }

    fn reload(&mut self, source: &str) {
        self.game.reload_project(&[("game.fun".to_string(), source.to_string())]).expect("reloads");
    }
}

#[test]
fn a_pure_jobs_result_is_delivered() {
    let mut rig = Rig::new(&game_source(DOUBLE, "21.0"));
    rig.tick(4);
    assert_eq!(rig.fake.dispatched().len(), 1);
    rig.finish();
    assert!(rig.model().contains("got: 42"), "{}", rig.model());
}

#[test]
fn a_job_reads_files_through_its_producers_reader_dependencies_included() {
    // `size` is a dependency: on main (the process's reader, no such file)
    // it is -1; on the worker it reads the producer's project.
    let work = r#"let size = if File.exists(Path.of("data/t.bin")) then Bytes.length(File.bytes(Path.of("data/t.bin"))) else -1.0
let work = (x) => x + size * 10.0 + Bytes.length(File.bytes(Path.of("data/t.bin")))"#;
    let mut rig = Rig::new(&game_source(work, "0.5"));
    rig.tick(4);
    rig.finish();
    assert!(rig.model().contains("got: 33.5"), "{}", rig.model());
}

#[test]
fn physics_and_ui_reads_inside_a_job_are_refused() {
    for (work, class) in [
        (r#"let work = (x) => if Physics.grounded("a") then 1.0 else 0.0"#, "physics"),
        (r#"let probe = (x) => Ui.button("go", x)
let work = (x) => let v = probe(x) in 1.0"#, "ui"),
    ] {
        let mut rig = Rig::new(&game_source(work, "1.0"));
        rig.tick(4);
        rig.finish();
        let model = rig.model();
        assert!(model.contains(&format!("reads {class} state")), "{model}");
        assert!(model.contains("got: -1"), "{model}");
    }
}

#[test]
fn an_input_or_result_holding_a_function_is_not_shareable() {
    let mut rig = Rig::new(&game_source(DOUBLE, "{ f: (y) => y }"));
    rig.tick(4);
    assert_eq!(rig.fake.dispatched().len(), 1, "answered through the pool, next frame");
    rig.finish();
    assert!(rig.model().contains("Job.run input: `.f` is a function"), "{}", rig.model());

    let returns_fn = "let work = (x) => { v: x, f: (y) => y }";
    let mut rig = Rig::new(&game_source(returns_fn, "1.0"));
    rig.tick(4);
    rig.finish();
    assert!(rig.model().contains("Job.run result: `.f` is a function"), "{}", rig.model());
}

#[test]
fn a_function_built_at_run_time_is_refused_at_issue() {
    let made = "let make = (k) => (x) => x * k\nlet work = make(3.0)";
    let mut rig = Rig::new(&game_source(made, "1.0"));
    rig.tick(4);
    rig.finish();
    assert!(rig.model().contains("needs a top-level function"), "{}", rig.model());
}

#[test]
fn a_panic_in_a_job_is_crashed() {
    let mut rig = Rig::new(&game_source(DOUBLE, "1.0"));
    rig.tick(4);
    crate::functor_lang_prelude::job::PANIC_NEXT.with(|p| p.set(true));
    rig.finish();
    assert!(rig.model().contains("the job crashed: a test's panic"), "{}", rig.model());
}

#[test]
fn cancel_stops_a_job_mid_evaluation() {
    // Far more calls than a test can wait for, through `List.fold`'s
    // closure calls: only the `Interp::call` probe ends it.
    let work = "let spin = (n) => List.fold((acc, x) => acc + x, 0.0, List.range(n))
let work = (x) => List.fold((acc, i) => acc + spin(1000000.0), 0.0, List.range(1000000.0))";
    let source = game_source(work, "1.0");
    let project = functor_lang::project::load_sources_with_bundled_modules(
        vec![("game.fun".into(), source)],
        &functor_prelude::bundled_modules(),
    )
    .ok()
    .unwrap();
    let program = JobProgram::new(&project.module, &project.sources);
    let spec = RunSpec {
        key: "k".into(),
        call: Ok(RunCall { program: program.source, f: "work".into(), input: Shareable::Number(1.0) }),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        flag.store(true, Ordering::Relaxed);
    });
    let started = std::time::Instant::now();
    let value = crate::functor_lang_prelude::job::run(&spec, None, &JobCtx { stop }, &ReadSlot::default());
    stopper.join().unwrap();
    assert!(started.elapsed().as_secs() < 5, "stopped promptly");
    assert!(format!("{value:?}").contains(functor_lang::eval::CANCELLED), "{value:?}");
}

#[test]
fn a_cancelled_job_delivers_nothing_and_the_key_is_free_again() {
    let source = game_source(DOUBLE, "21.0").replace(
        "if m.n == 2.0 then (next, Job.run(\"k\", work, 21.0, Done)) else next",
        "if m.n == 2.0 then (next, Job.run(\"k\", work, 21.0, Done)) \
else if m.n == 3.0 then (next, Job.cancel(\"k\")) \
else if m.n == 4.0 then (next, Job.run(\"k\", work, 1.0, Done)) else next",
    );
    let mut rig = Rig::new(&source);
    rig.tick(6);
    let dispatched = rig.fake.dispatched();
    assert_eq!(dispatched.len(), 2);
    assert_eq!(rig.fake.stopped(dispatched[0]), Some(true));
    rig.finish();
    assert!(rig.model().contains("got: 2"), "{}", rig.model());
}

#[test]
fn a_reload_that_leaves_the_function_alone_keeps_its_job() {
    let source = game_source(DOUBLE, "21.0");
    let mut rig = Rig::new(&source);
    rig.tick(4);
    rig.reload(&source.replace("let unrelated = 1.0", "let unrelated = 7.0"));
    rig.tick(1);
    assert_eq!(rig.fake.dispatched().len(), 1, "not run again");
    rig.finish();
    assert!(rig.model().contains("got: 42"), "{}", rig.model());
}

#[test]
fn a_reload_that_changes_the_function_runs_it_again_as_a_new_attempt() {
    let source = game_source(DOUBLE, "21.0");
    let mut rig = Rig::new(&source);
    rig.tick(4);
    let first = rig.fake.held()[0];
    // `scale` is a dependency: the code `work` reaches changed.
    rig.reload(&source.replace("let scale = 2.0", "let scale = 3.0"));
    let dispatched = rig.fake.dispatched();
    assert_eq!(dispatched.len(), 2);
    assert_eq!(dispatched[1], Ticket { id: first.id, attempt: 2 });
    assert_eq!(rig.fake.stopped(first), Some(true));
    rig.finish();
    assert!(rig.model().contains("got: 63"), "the new code's result: {}", rig.model());
}

#[test]
fn a_result_that_read_a_changed_file_runs_again_after_a_reload() {
    let work = r#"let work = (x) => x + Bytes.length(File.bytes(Path.of("data/t.bin")))"#;
    let source = game_source(work, "0.0");
    let mut rig = Rig::new(&source);
    rig.tick(4);
    // Finished, not yet delivered, when the file changes and a reload runs.
    rig.fake.complete(rig.fake.held()[0]);
    std::fs::write(rig.dir.path().join("data/t.bin"), [1u8; 5]).unwrap();
    rig.reload(&source.replace("let unrelated = 1.0", "let unrelated = 2.0"));
    rig.tick(1);
    assert!(rig.model().contains("got: -1"), "the stale result dropped: {}", rig.model());
    assert_eq!(rig.fake.dispatched().len(), 2);
    rig.finish();
    assert!(rig.model().contains("got: 5"), "{}", rig.model());
}

#[test]
fn a_job_on_the_pool_replays_exactly_without_running_again() {
    let mut rig = Rig::new(&game_source(DOUBLE, "21.0"));
    let counted = Rc::new(Counted(std::cell::Cell::new(0)));
    rig.game.job_port().set_dispatch(counted.clone());
    rig.tick(1);
    let mut models = Vec::new();
    let started = std::time::Instant::now();
    while !rig.model().contains("got: 42") {
        assert!(started.elapsed().as_secs() < 30, "the job never landed");
        std::thread::sleep(std::time::Duration::from_millis(1));
        rig.tick(1);
        models.push(rig.model());
    }
    for _ in 0..3 {
        rig.tick(1);
        models.push(rig.model());
    }
    assert_eq!(counted.0.get(), 1);
    rig.game.seek_scene_to(0).expect("seek");
    let replayed = rig.game.replay_scene(models.len()).expect("replays");
    let replayed: Vec<String> = replayed.into_iter().map(|(model, _)| model).collect();
    assert_eq!(replayed, models);
    assert_eq!(counted.0.get(), 1, "a replay never runs the job");
}

/// Spin-up and per-job cost, for the PR body: `FUNCTOR_SPINUP` lists
/// `dir:entry:f` (comma-separated). Run in release with `--ignored
/// --nocapture`.
#[test]
#[ignore]
fn measure_job_costs() {
    let median = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    for spec in std::env::var("FUNCTOR_SPINUP").expect("FUNCTOR_SPINUP").split(',') {
        let mut parts = spec.split(':');
        let (dir, entry, f) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        let dir = std::path::Path::new(dir);
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".fun") && n != entry)
            .collect();
        names.sort();
        names.insert(0, entry.to_string());
        let files: Vec<(std::path::PathBuf, String)> = names
            .iter()
            .map(|n| (dir.join(n), std::fs::read_to_string(dir.join(n)).unwrap()))
            .collect();
        let lines: usize = files.iter().map(|(_, s)| s.lines().count()).sum();
        let project = functor_lang::project::load_sources_with_bundled_modules(
            files,
            &functor_prelude::bundled_modules(),
        )
        .unwrap_or_else(|e| panic!("{}", e.render()));
        let started = std::time::Instant::now();
        let program = JobProgram::new(&project.module, &project.sources);
        let admit_ms = started.elapsed().as_secs_f64() * 1e3;
        let deps = functor_lang::reuse::CodeIndex::new(&project.module).deps_of(f).unwrap_or_else(|| {
            let near: Vec<&str> = project.module.defs.iter().map(|d| d.name.as_str()).filter(|n| n.contains("load")).collect();
            panic!("no `{f}`: {near:?}")
        }).len();
        let call = RunCall { program: program.source.clone(), f: f.to_string(), input: Shareable::Number(0.0) };
        let runs: Vec<(f64, f64)> = (0..5).map(|_| crate::functor_lang_prelude::job::spin_up(&call)).collect();
        let compile = median(runs.iter().map(|r| r.0).collect());
        let deps_ms = median(runs.iter().map(|r| r.1).collect());
        println!(
            "spin-up {} ({} files, {lines} lines, {} defs linked; `{f}` reaches {deps}): \
compile {compile:.1} ms, deps {deps_ms:.1} ms; JobProgram::new {admit_ms:.2} ms",
            dir.display(),
            names.len(),
            project.module.defs.len(),
        );
    }

    // Per-job overhead: a trivial job's round trip on a warm pool worker.
    let source = game_source("let work = (x) => x", "0.0");
    let project = functor_lang::project::load_sources_with_bundled_modules(
        vec![("game.fun".into(), source)],
        &functor_prelude::bundled_modules(),
    )
    .ok()
    .unwrap();
    let program = JobProgram::new(&project.module, &project.sources);
    let pool = Pool::start(1);
    let mailbox = Mailbox::default();
    for (label, input) in [
        ("a number", Shareable::Number(1.0)),
        ("a 10k-number list", Shareable::from_value(&functor_lang::Value::List(Rc::new(
            (0..10_000).map(|i| functor_lang::Value::Number(i as f64)).collect(),
        ))).unwrap()),
    ] {
        let mut times = Vec::new();
        for id in 0..201u64 {
            let spec = Arc::new(RunSpec {
                key: "k".into(),
                call: Ok(RunCall { program: program.source.clone(), f: "work".into(), input: input.clone() }),
            });
            let (task, fail, _) = JobSpec::Run(spec).task(None);
            let stop = Arc::new(AtomicBool::new(false));
            let started = std::time::Instant::now();
            pool.dispatch(Job { ticket: Ticket { id, attempt: 1 }, task, fail, stop, progress: Default::default(), mailbox: mailbox.clone() });
            let got = loop {
                if let Some((_, value)) = mailbox.take().pop() {
                    break value;
                }
                std::hint::spin_loop();
            };
            let us = started.elapsed().as_secs_f64() * 1e6;
            // Main-side adoption is part of the cost.
            let started = std::time::Instant::now();
            let _ = got.to_value();
            let adopt = started.elapsed().as_secs_f64() * 1e6;
            if id > 0 {
                times.push(us + adopt);
            }
        }
        println!("per-job round trip, warm worker, {label}: median {:.0} µs", median(times));
    }
}

// ── Progress ────────────────────────────────────────────────────────────

/// Watches `Job.progress` for keys "k" (into `seen`) and "o" (into
/// `other`); `ask` is `tick`'s body after `next`. `work` reports 1.0.
fn progress_source(ask: &str) -> String {
    format!(
        r#"
type Msg = | Done(r: Result.t<float, Job.error>) | Prog(p: float) | Other(p: float)
let scale = 2.0
let work = (x) => Job.report(1.0, x * scale)
let init = {{ n: 0.0, got: -1.0, seen: [], other: [] }}
let update = (m, msg) =>
  match msg with
  | Done(r) =>
    (match r with
     | Result.Ok(v) => {{ m with got: v }}
     | Result.Error(e) => m)
  | Prog(p) => {{ m with seen: List.append([p], m.seen) }}
  | Other(p) => {{ m with other: List.append([p], m.other) }}
let subscriptions = (m) => Sub.batch([Job.progress("k", Prog), Job.progress("o", Other)])
let tick = (m, dt, tts) =>
  let next = {{ m with n: m.n + 1.0 }} in
  {ask}
let draw = (m, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(m.got, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
"#
    )
}

const ASK_K: &str = r#"if m.n == 2.0 then (next, Job.run("k", work, 21.0, Done)) else next"#;

impl Rig {
    fn assert_seen(&self, seen: &str) {
        let model = self.model();
        assert!(model.contains(&format!("seen: {seen}")), "want seen {seen}: {model}");
    }
}

#[test]
fn progress_is_delivered_once_per_change_and_never_after_the_result() {
    let mut rig = Rig::new(&progress_source(ASK_K));
    rig.tick(4);
    let job = rig.fake.held()[0];
    rig.fake.report(job, 0.25);
    rig.tick(3);
    rig.assert_seen("[0.25]");
    // Two reports between frames: the frame delivers the latest.
    rig.fake.report(job, 0.5);
    rig.fake.report(job, 0.75);
    rig.tick(1);
    rig.assert_seen("[0.25, 0.75]");
    // A report and the result in one frame: only the result.
    rig.fake.report(job, 0.9);
    rig.finish();
    rig.tick(2);
    assert!(rig.model().contains("got: 42"), "{}", rig.model());
    rig.assert_seen("[0.25, 0.75]");
}

#[test]
fn a_cancelled_job_stops_reporting() {
    let ask = r#"if m.n == 2.0 then (next, Job.run("k", work, 21.0, Done)) else if m.n == 5.0 then (next, Job.cancel("k")) else next"#;
    let mut rig = Rig::new(&progress_source(ask));
    rig.tick(4);
    let job = rig.fake.held()[0];
    rig.fake.report(job, 0.3);
    rig.tick(2);
    assert_eq!(rig.fake.stopped(job), Some(true));
    rig.fake.report(job, 0.6);
    rig.tick(2);
    rig.assert_seen("[0.3]");
}

#[test]
fn a_reload_rerun_restarts_progress_and_drops_the_old_attempts() {
    let source = progress_source(ASK_K);
    let mut rig = Rig::new(&source);
    rig.tick(4);
    let first = rig.fake.held()[0];
    rig.fake.report(first, 0.5);
    rig.tick(1);
    rig.reload(&source.replace("let scale = 2.0", "let scale = 3.0"));
    let second = rig.fake.held()[1];
    assert_eq!(second, Ticket { id: first.id, attempt: 2 });
    rig.fake.report(first, 0.9);
    rig.tick(1);
    rig.assert_seen("[0.5]");
    rig.fake.report(second, 0.1);
    rig.tick(1);
    rig.assert_seen("[0.5, 0.1]");
}

#[test]
fn each_jobs_progress_reaches_its_own_keys_taggers() {
    let ask = r#"if m.n == 2.0 then (next, Effect.batch([Job.run("k", work, 21.0, Done), Job.run("o", work, 1.0, Done), Job.run("z", work, 1.0, Done)])) else next"#;
    let mut rig = Rig::new(&progress_source(ask));
    rig.tick(4);
    let held = rig.fake.held();
    rig.fake.report(held[0], 0.2);
    rig.fake.report(held[1], 0.7);
    rig.fake.report(held[2], 0.4);
    rig.tick(1);
    rig.assert_seen("[0.2]");
    assert!(rig.model().contains("other: [0.7]"), "{}", rig.model());
}

#[test]
fn a_replay_reproduces_progress_without_running_the_job() {
    let mut rig = Rig::new(&progress_source(ASK_K));
    rig.tick(1);
    let mut models = Vec::new();
    let mut play = |rig: &mut Rig, n: u32| {
        for _ in 0..n {
            rig.tick(1);
            models.push(rig.model());
        }
    };
    play(&mut rig, 3);
    let job = rig.fake.held()[0];
    rig.fake.report(job, 0.25);
    play(&mut rig, 2);
    rig.fake.report(job, 0.5);
    play(&mut rig, 2);
    rig.fake.complete(job);
    play(&mut rig, 2);
    rig.assert_seen("[0.25, 0.5]");
    rig.game.seek_scene_to(0).expect("seek");
    let replayed = rig.game.replay_scene(models.len()).expect("replays");
    let replayed: Vec<String> = replayed.into_iter().map(|(model, _)| model).collect();
    assert_eq!(replayed, models);
    assert_eq!(rig.fake.dispatched().len(), 1, "a replay never runs the job");
}

#[test]
fn a_branch_that_cuts_the_shown_progress_shows_the_latest_again() {
    let mut rig = Rig::new(&progress_source(ASK_K));
    rig.tick(4);
    let job = rig.fake.held()[0];
    rig.fake.report(job, 0.25);
    rig.tick(2); // frames 4–5: 0.25 shown on 4
    rig.fake.report(job, 0.5);
    rig.tick(3); // frames 6–8: 0.5 shown on 6
    // A branch after 6 keeps what was shown: nothing again.
    rig.game.seek_scene_to(7).expect("seek");
    rig.tick(1);
    rig.assert_seen("[0.25, 0.5]");
    // A branch from 5 cuts it: the kept model saw 0.25, so 0.5 comes again.
    rig.game.seek_scene_to(5).expect("seek");
    rig.assert_seen("[0.25]");
    rig.tick(1);
    rig.assert_seen("[0.25, 0.5]");
    assert_eq!(rig.fake.dispatched().len(), 1, "the job kept running");
}

#[test]
fn progress_that_cancels_another_job_stops_that_jobs_progress_the_same_frame() {
    let ask = r#"if m.n == 2.0 then (next, Effect.batch([Job.run("k", work, 21.0, Done), Job.run("o", work, 1.0, Done)])) else next"#;
    let source = progress_source(ask).replace(
        "| Prog(p) => { m with seen: List.append([p], m.seen) }",
        "| Prog(p) => ({ m with seen: List.append([p], m.seen) }, Job.cancel(\"o\"))",
    );
    let mut rig = Rig::new(&source);
    rig.tick(4);
    let held = rig.fake.held();
    rig.fake.report(held[0], 0.2);
    rig.fake.report(held[1], 0.7);
    rig.tick(1);
    rig.assert_seen("[0.2]");
    assert!(rig.model().contains("other: []"), "{}", rig.model());
}
