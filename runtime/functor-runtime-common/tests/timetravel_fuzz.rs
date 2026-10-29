//! Time-travel fuzzer: seeded random sequences of play / pause / scrub /
//! branch / hot reload over the embedded (web + Quest) producer, driven the way
//! a shell drives it (a [`GameClock`], one fixed step per rendered frame). The
//! desktop producer shares its recorder and frame body. Input while paused
//! models debug `POST /input` (shells ignore window input while paused).
//!
//! After every operation it checks that the independent histories — the model
//! snapshot ring, the physics timeline, the clock — stay in lockstep, and that
//! seeking to a recorded frame restores exactly what that frame recorded.
//!
//! Short budget by default. A long run:
//!
//! ```sh
//! TIMETRAVEL_FUZZ_SEEDS=20 TIMETRAVEL_FUZZ_OPS=400 \
//!   cargo test -p functor_runtime_common --test timetravel_fuzz -- --nocapture
//! ```
//!
//! `TIMETRAVEL_FUZZ_GAME=<name>` limits the run to one game; a failure prints
//! its shrunk op sequence. `TIMETRAVEL_FUZZ_DUMP=<dir>` writes the recorded and
//! restored world JSON (`was.json` / `now.json`) on a world-bytes mismatch.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use functor_runtime_common::functor_lang_game_embedded::{FunctorLangEmbeddedGame, NativePlatform};
use functor_runtime_common::physics::{self, DEFAULT_WORLD, FIXED_DT};
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::{
    FrameTime, GameClock, InputEdges, InputSnapshot, Key, RecordedInput, XrControllerSnapshot,
    XrInputSnapshot,
};

// ── Games ───────────────────────────────────────────────────────────────

struct Spec {
    name: &'static str,
    sources: Vec<(String, String)>,
    keys: &'static [Key],
    /// The game defines `input`, so key events reach it (and its log).
    events: bool,
    /// Its model holds closures, so a reload restarts history (by design).
    closures: bool,
    /// It defines `physics`, so every played frame steps the world once.
    physics: bool,
    mouse: bool,
    xr: bool,
    /// A tunable constant: `(exact source text, template with {})`. A semantic
    /// reload rewrites it; a no-op reload only appends an unused binding.
    knob: (&'static str, &'static str),
}

fn example(dir: &str) -> Vec<(String, String)> {
    let root = format!("{}/../../examples/{dir}", env!("CARGO_MANIFEST_DIR"));
    let read = |name: &str| std::fs::read_to_string(format!("{root}/{name}")).unwrap();
    let entry = "game.fun";
    let mut sources = vec![(entry.to_string(), read(entry))];
    let mut siblings: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".fun") && n != entry)
        .collect();
    siblings.sort();
    for name in siblings {
        let src = read(&name);
        sources.push((name, src));
    }
    sources
}

/// Dynamic bodies beside a `Physics.memo` static group, a `Scene.memo`
/// draw, `Sub.every`, and `update`: the memo caches under time travel.
const MEMO_GAME: &str = r#"
let tileCount = 12.0
let gravity = Vec3.make(0.0, -9.81, 0.0)
let tile = (i) => Physics.fixed(Physics.tag(Text.concat("t", Text.fromFloat(i))), Physics.box(1.0, 0.2, 1.0))
  |> Physics.at(Vec3.make(Math.mod(i, 4.0) * 1.1 - 2.0, 0.0, Math.floor(i / 4.0) * 1.1 - 1.5))
let tiles = (n) => List.map(tile, List.range(n))
let ballTag = (i) => Physics.tag(Text.concat("b", Text.fromFloat(i)))
let ball = (drop, i) => Physics.dynamic(ballTag(i), Physics.sphere(0.25))
  |> Physics.at(Vec3.make(Math.sin(i * 3.1 + drop) * 1.5, 2.0 + i * 0.6, Math.cos(i * 1.7 + drop)))
  |> Physics.restitution(0.5)
type Msg = | Beat
let init = { drop: 0.0, layout: 0.0, beats: 0.0 }
let physics = (model) =>
  Physics.scene(gravity, [Physics.memo("tiles", model.layout + tileCount, tiles), ball(model.drop, 0.0), ball(model.drop, 1.0), ball(model.drop, 2.0)])
let input = (model, key, isDown) =>
  match key with
  | Key.Space => if isDown then { model with drop: model.drop + 1.0 } else model
  | Key.L => if isDown then { model with layout: Math.mod(model.layout + 1.0, 3.0) } else model
  | Key.K => if isDown then model else (model, Physics.applyImpulse(ballTag(0.0), Vec3.make(0.5, 4.0, 0.0)))
  | _ => model
let subscriptions = (model) => Sub.every(Time.seconds(0.25), Beat)
let update = (model, msg) =>
  match msg with
  | Beat => { model with beats: model.beats + 1.0 }
let tick = (model, dt, tts) => model
let floor = (n) => Scene.group(List.range(n) |> List.map((i) => Scene.cube() |> Scene.translate(Vec3.make(i, 0.0, 0.0))))
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 6.0, -8.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([
      Scene.memo("floor", model.layout + tileCount, floor),
      Scene.sphere() |> Physics.transformed(ballTag(0.0)),
      Scene.sphere() |> Physics.transformed(ballTag(1.0)),
      Scene.sphere() |> Physics.transformed(ballTag(2.0)),
      Scene.cube() |> Scene.translate(Vec3.make(model.beats * 0.01, 0.0, 0.0)),
    ]))
"#;

/// A closure stored in the model: a reload can't carry such history, so it
/// restarts history at the current frame (an unsafe reload). The two closures
/// differ in parameter name so the model's `Display` tells them apart.
const CLOSURE_GAME: &str = r#"
let ballTag = Physics.tag("ball")
let slow = (x) => x + 1.0
let fast = (y) => y + 2.0
let init = { step: slow, n: 0.0 }
let input = (model, key, isDown) =>
  match key with
  | Key.F => if isDown then { model with step: fast } else { model with step: slow }
  | Key.K => if isDown then model else (model, Physics.applyImpulse(ballTag, Vec3.make(0.0, 3.0, 0.0)))
  | _ => model
let tick = (model, dt, tts) => { model with n: model.step(model.n) }
let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0), [
    Physics.fixed(Physics.tag("ground"), Physics.box(8.0, 0.2, 8.0)),
    Physics.dynamic(ballTag, Physics.sphere(0.3)) |> Physics.at(Vec3.make(0.0, 2.0, 0.0)) |> Physics.restitution(0.4)])
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([Scene.sphere() |> Physics.transformed(ballTag),
                 Scene.cube() |> Scene.translate(Vec3.make(Math.mod(model.n, 7.0), 0.0, 0.0))]))
"#;

fn specs() -> Vec<Spec> {
    use Key::*;
    vec![
        Spec {
            name: "character",
            sources: example("character"),
            keys: &[W, A, S, D, Space, Left, Right],
            events: false,
            physics: true,
            closures: false,
            mouse: false,
            xr: true,
            knob: ("let walkSpeed = 3.0", "let walkSpeed = {}"),
        },
        Spec {
            name: "physics",
            sources: example("physics"),
            keys: &[Space, K, R],
            events: true,
            physics: true,
            closures: false,
            mouse: false,
            xr: false,
            knob: ("|> Physics.restitution(0.55))", "|> Physics.restitution({}))"),
        },
        Spec {
            name: "platformer",
            sources: example("platformer"),
            keys: &[Left, Right, A, D, Space, Up, W],
            events: true,
            physics: false,
            closures: false,
            mouse: false,
            xr: false,
            knob: ("let runSpeed = 8.0", "let runSpeed = {}"),
        },
        Spec {
            name: "crowd",
            sources: {
                // A small crowd keeps per-frame draws cheap; the skinned path is
                // the same at any count.
                let mut s = example("crowd");
                s[0].1 = s[0].1.replace("let count = 100.0", "let count = 4.0");
                s
            },
            keys: &[],
            events: false,
            physics: false,
            closures: false,
            mouse: false,
            xr: false,
            knob: ("let spacing = 1.2", "let spacing = {}"),
        },
        Spec {
            name: "night-terrain",
            sources: example("night-terrain"),
            keys: &[W, A, S, D, Up, Down],
            events: true,
            physics: true,
            closures: false,
            mouse: true,
            xr: false,
            knob: ("let walkSpeed = 6.0", "let walkSpeed = {}"),
        },
        Spec {
            name: "closure",
            sources: vec![("game.fun".to_string(), CLOSURE_GAME.to_string())],
            keys: &[F, K],
            events: true,
            physics: true,
            closures: true,
            mouse: false,
            xr: false,
            knob: ("|> Physics.restitution(0.4)", "|> Physics.restitution({})"),
        },
        Spec {
            name: "memo",
            sources: vec![("game.fun".to_string(), MEMO_GAME.to_string())],
            keys: &[Space, L, K],
            events: true,
            physics: true,
            closures: false,
            mouse: false,
            xr: false,
            knob: ("|> Physics.restitution(0.5)", "|> Physics.restitution({})"),
        },
    ]
}

// ── Operations ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
enum Target {
    Back(u16),
    Oldest,
    Newest,
    /// Past the newest frame: the recorder clamps.
    Beyond(u16),
}

#[derive(Clone, Debug)]
enum Op {
    /// Render `frames` frames with random input (no ticks while paused, so
    /// paused input then stays pending, as debug `POST /input` delivers it).
    Play { frames: u8, seed: u32 },
    Pause,
    /// The scrubber's play button: rebase the clock to the scene, unpause.
    Resume,
    /// The scrubber's single step (implies pause).
    Step,
    /// Non-destructive scrub (pauses).
    Seek(Target),
    /// Drag into the future: seek the newest frame and queue `n` catch-up
    /// frames, drained at most 8 per rendered frame by this and later ops.
    PastEnd(u8),
    /// The debug API's destructive `POST /rewind`.
    Rewind(Target),
    /// Hot reload; `semantic` rewrites the spec's knob, else an unused binding.
    Reload { semantic: bool, n: u32 },
    /// Branch `back` frames from the newest and replay the recorder's own
    /// input log over `frames` frames: must reproduce what was recorded.
    Replay { back: u16, frames: u8 },
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

fn gen_target(rng: &mut Rng) -> Target {
    match rng.below(10) {
        0 => Target::Oldest,
        1 | 2 => Target::Newest,
        3 => Target::Beyond(1 + rng.below(40) as u16),
        _ => Target::Back(1 + rng.below(120) as u16),
    }
}

fn gen_ops(seed: u64, count: usize) -> Vec<Op> {
    let mut rng = Rng::new(seed);
    // Start with real history so early scrubs have somewhere to go.
    let mut ops = vec![Op::Play {
        frames: 40,
        seed: rng.next() as u32,
    }];
    while ops.len() < count {
        let op = match rng.below(100) {
            0..=29 => Op::Play {
                frames: 1 + rng.below(45) as u8,
                seed: rng.next() as u32,
            },
            30..=37 => Op::Pause,
            38..=47 => Op::Resume,
            48..=51 => Op::Step,
            52..=67 => Op::Seek(gen_target(&mut rng)),
            68..=71 => Op::PastEnd(1 + rng.below(20) as u8),
            72..=81 => Op::Rewind(gen_target(&mut rng)),
            82..=89 => Op::Reload {
                semantic: rng.chance(0.5),
                n: rng.next() as u32,
            },
            _ => Op::Replay {
                back: 1 + rng.below(90) as u16,
                frames: 1 + rng.below(30) as u8,
            },
        };
        ops.push(op);
    }
    ops
}

// ── The shell ───────────────────────────────────────────────────────────

/// What a frame recorded, captured right after its tick. `None` = unknown
/// (a semantic reload invalidated it); the next observation is adopted.
#[derive(Clone, Debug)]
struct Rec {
    model: Option<String>,
    world: Option<u64>,
    fixed: Option<u64>,
    /// Physics commands in flight at the end of the frame.
    pending: Option<String>,
    tts: f64,
    frame: Option<u64>,
    /// The program revision it ran under (semantic reloads bump it).
    rev: u32,
    /// A reload ran while this frame's input was pending: the reload drops the
    /// queries that input issued but the log keeps the input, so a replay
    /// of this frame diverges (a known gap; not replayed).
    reload_dropped_queries: bool,
    /// The shell's held keys at the tick.
    held: Vec<Key>,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Panic,
    /// A frame recorded somewhere other than the selected frame + 1.
    FrameGap,
    /// Model / physics / clock disagree about the current frame.
    Lockstep,
    /// A seek restored different bytes than the frame recorded.
    Fidelity,
    /// Branch-and-replay of the recorder's input log diverged.
    ReplayDiverged,
    /// The recorder's input log differs from what the shell fed.
    InputLog,
    /// An exact seek inside the scrubber range was refused.
    Refused,
    /// A no-op reload changed the selected state or the range.
    Reload,
}

#[derive(Debug, Clone)]
struct Failure {
    op: usize,
    kind: Kind,
    detail: String,
}

/// FNV-1a: stable across toolchains, so reported seeds keep reproducing.
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The live world: `(hash of its bytes minus pending commands, frame,
/// pending commands)`. Pending commands are checked on their own: input
/// between frames legitimately queues them on the live world.
fn world() -> Option<(u64, u64, String)> {
    physics::with_world(DEFAULT_WORLD, |w| {
        let mut core = w.clone();
        let pending = core.take_pending_commands();
        (hash(&core.snapshot()), w.frame(), format!("{pending:?}"))
    })
}

struct Shell<'a> {
    spec: &'a Spec,
    game: FunctorLangEmbeddedGame,
    clock: GameClock,
    oracle: BTreeMap<u64, Rec>,
    held: Vec<Key>,
    /// Key transitions since the last sample, as the shells accumulate them.
    edges: InputEdges,
    /// A reload ran while input was pending (see [`Rec::reload_dropped_queries`]).
    reloaded_pending: bool,
    stick: [f32; 2],
    look: [f32; 2],
    jump: bool,
    /// Inputs fed since the last tick (a seek or rewind drops them, as the
    /// producer drops its input buffer).
    pending: Vec<RecordedInput>,
    knob: Option<String>,
    noop: u32,
    rev: u32,
    op: usize,
}

type Check = Result<(), Failure>;

thread_local! {
    /// Recorded world JSON per frame, kept only under `TIMETRAVEL_FUZZ_DUMP`.
    static DUMP: RefCell<BTreeMap<u64, Vec<u8>>> = Default::default();
    /// Set while a fuzz run's panics are caught and reported.
    static QUIET: Cell<bool> = const { Cell::new(false) };
    /// What the run on this thread verified (printed with `--nocapture`).
    static STATS: RefCell<Stats> = Default::default();
    /// Where the last caught panic happened.
    static PANIC_AT: RefCell<String> = Default::default();
}

fn count(what: &'static str) {
    STATS.with(|s| *s.borrow_mut().entry(what).or_default() += 1);
}

impl<'a> Shell<'a> {
    fn new(spec: &'a Spec) -> Shell<'a> {
        let game = FunctorLangEmbeddedGame::create(spec.sources.clone(), Box::new(NativePlatform))
            .unwrap_or_else(|e| panic!("{}: {e}", spec.name));
        let mut shell = Shell {
            spec,
            game,
            clock: GameClock::new(None),
            oracle: BTreeMap::new(),
            held: Vec::new(),
            edges: InputEdges::default(),
            reloaded_pending: false,
            stick: [0.0; 2],
            look: [0.0; 2],
            jump: false,
            pending: Vec::new(),
            knob: None,
            noop: 0,
            rev: 0,
            op: 0,
        };
        // The shell's bootstrap: a zero-delta first frame.
        shell.render_frame(0.0).expect("bootstrap");
        shell
    }

    fn fail(&self, kind: Kind, detail: String) -> Failure {
        Failure {
            op: self.op,
            kind,
            detail,
        }
    }

    fn sample(&mut self) -> InputSnapshot {
        let mut held = self.held.clone();
        held.sort_by_key(|k| *k as i32);
        let mut snap = InputSnapshot {
            held_keys: held,
            xr: self.spec.xr.then(|| XrInputSnapshot {
                head: None,
                left: XrControllerSnapshot {
                    active: true,
                    thumbstick: self.stick,
                    ..Default::default()
                },
                right: XrControllerSnapshot {
                    active: true,
                    thumbstick: self.look,
                    primary_pressed: self.jump,
                    ..Default::default()
                },
            }),
            ..InputSnapshot::default()
        };
        std::mem::take(&mut self.edges).apply_to(&mut snap);
        snap
    }

    /// One rendered frame: the clock's fixed steps, then a draw.
    fn render_frame(&mut self, real_delta: f32) -> Check {
        for ft in self.clock.fixed_frames(real_delta) {
            self.tick(ft)?;
        }
        let tts = self.clock.current_tts();
        self.game.render(FrameTime { dts: 0.0, tts });
        Ok(())
    }

    fn tick(&mut self, ft: FrameTime) -> Check {
        let selected = self.game.current_scene_frame();
        if self.game.samples_input() {
            let snap = self.sample();
            self.game.sampled_input(&snap);
            self.pending.push(RecordedInput::Snapshot(Box::new(snap)));
        }
        self.game.tick(ft);
        let fed = std::mem::take(&mut self.pending);
        if ft.dts <= 0.0 {
            return Ok(());
        }
        let frame = self.game.current_scene_frame().expect("a played frame records");
        let expect = selected.map_or(0, |s| s + 1);
        if frame != expect {
            return Err(self.fail(
                Kind::FrameGap,
                format!("recorded frame {frame}, expected {expect} (selected {selected:?})"),
            ));
        }
        count("frames recorded");
        if self.oracle.contains_key(&frame) {
            count("branches");
        }
        let w = world();
        // One fixed step per recorded frame, from where the selected frame ended.
        let from = selected.and_then(|s| self.oracle.get(&s)).and_then(|r| r.fixed);
        if let (true, Some(from), Some(now)) = (self.spec.physics, from, w.as_ref().map(|w| w.1)) {
            if now != from + 1 {
                return Err(self.fail(
                    Kind::Lockstep,
                    format!("frame {frame}: world stepped from fixed {from} to {now}"),
                ));
            }
        }
        let logged = self.game.recorded_inputs_at(frame);
        if format!("{logged:?}") != format!("{fed:?}") {
            return Err(self.fail(
                Kind::InputLog,
                format!("frame {frame}: logged {logged:?}\n  but fed {fed:?}"),
            ));
        }
        // The future past a branch is gone.
        self.oracle.split_off(&frame);
        if std::env::var("TIMETRAVEL_FUZZ_DUMP").is_ok() {
            let bytes = physics::with_world(DEFAULT_WORLD, |w| w.snapshot()).unwrap_or_default();
            DUMP.with(|d| d.borrow_mut().insert(frame, bytes));
        }
        // Drawn at the frame's own time: a catch-up batch advances the clock
        // past it before the batch's ticks run.
        let drawn = self.draw_hash(ft.tts);
        self.oracle.insert(
            frame,
            Rec {
                model: Some(self.game.state_debug()),
                world: w.as_ref().map(|w| w.0),
                fixed: w.as_ref().map(|w| w.1),
                pending: w.map(|w| w.2),
                tts: self.game.current_scene_tts().expect("tts"),
                frame: Some(drawn),
                rev: self.rev,
                reload_dropped_queries: std::mem::take(&mut self.reloaded_pending),
                held: self.held.clone(),
            },
        );
        Ok(())
    }

    fn draw_hash(&mut self, tts: f32) -> u64 {
        let frame = self.game.render(FrameTime { dts: 0.0, tts });
        hash(&serde_json::to_vec(&frame).expect("frame serializes"))
    }

    fn feed(&mut self, input: RecordedInput) {
        match &input {
            RecordedInput::Key { code, is_down } => self.game.key_event(*code, *is_down),
            RecordedInput::MouseMove { x, y } => self.game.mouse_move(*x, *y),
            other => panic!("the fuzzer never feeds {other:?}"),
        }
        // Without an `input` hook a key event is dropped unlogged.
        if self.spec.events || !matches!(input, RecordedInput::Key { .. }) {
            self.pending.push(input);
        }
    }

    fn random_input(&mut self, rng: &mut Rng) {
        let keys = self.spec.keys;
        if !keys.is_empty() && rng.chance(0.3) {
            let key = keys[rng.below(keys.len() as u64) as usize];
            let down = !self.held.contains(&key);
            self.edges.key_transition(key, !down, down);
            if down {
                self.held.push(key);
            } else {
                self.held.retain(|k| *k != key);
            }
            self.feed(RecordedInput::Key {
                code: key as i32,
                is_down: down,
            });
        }
        if self.spec.mouse && rng.chance(0.1) {
            let (x, y) = (rng.below(800) as i32, rng.below(600) as i32);
            self.feed(RecordedInput::MouseMove { x, y });
        }
        if self.spec.xr && rng.chance(0.2) {
            let axis = |rng: &mut Rng| (rng.below(5) as f32 - 2.0) * 0.5;
            self.stick = [axis(rng), axis(rng)];
            self.look = [axis(rng), 0.0];
            self.jump = rng.chance(0.2);
        }
    }

    /// The current program source: the spec's, with the knob and no-op
    /// bindings applied.
    fn source(&self) -> String {
        let mut src = self.spec.sources[0].1.clone();
        if let Some(knob) = &self.knob {
            assert!(src.contains(self.spec.knob.0), "{}: knob not found", self.spec.name);
            src = src.replace(self.spec.knob.0, knob);
        }
        for i in 0..self.noop {
            src.push_str(&format!("\nlet fuzzReload{i} = {i}.0\n"));
        }
        src
    }

    fn range(&self) -> Option<(u64, u64)> {
        self.game.scene_frame_range()
    }

    fn resolve(&self, target: Target) -> u64 {
        let (lo, hi) = self.range().unwrap_or((0, 0));
        match target {
            Target::Back(n) => hi.saturating_sub(n as u64).max(lo),
            Target::Oldest => lo,
            Target::Newest => hi,
            Target::Beyond(n) => hi + n as u64,
        }
    }

    /// Every recorder agrees on the selected frame.
    fn check_lockstep(&self) -> Check {
        let Some(frame) = self.game.current_scene_frame() else {
            return Ok(());
        };
        let (lo, hi) = self.range().expect("a selected frame has a range");
        if frame < lo || frame > hi {
            return Err(self.fail(Kind::Lockstep, format!("frame {frame} outside {lo}..={hi}")));
        }
        let tts = self.game.current_scene_tts().expect("tts");
        let clock = self.clock.current_tts() as f64;
        if tts != clock {
            return Err(self.fail(
                Kind::Lockstep,
                format!("frame {frame}: scene tts {tts} but clock {clock}"),
            ));
        }
        if let Some(rec) = self.oracle.get(&frame) {
            if rec.tts != tts {
                return Err(self.fail(
                    Kind::Lockstep,
                    format!("frame {frame}: tts {tts}, recorded {}", rec.tts),
                ));
            }
            let live = world().map(|w| w.1);
            if rec.fixed.is_some() && live != rec.fixed {
                return Err(self.fail(
                    Kind::Lockstep,
                    format!("frame {frame}: world at fixed {live:?}, recorded {:?}", rec.fixed),
                ));
            }
        }
        Ok(())
    }

    /// The live state equals what `frame` recorded (adopting unknowns).
    fn check_fidelity(&mut self, frame: u64, kind: Kind) -> Check {
        let model = self.game.state_debug();
        let w = world();
        let drawn = self.draw_hash(self.clock.current_tts());
        let op = self.op;
        let Some(rec) = self.oracle.get_mut(&frame) else {
            return Err(Failure {
                op,
                kind: Kind::Lockstep,
                detail: format!("frame {frame} is in range but was never recorded"),
            });
        };
        let mut diffs = Vec::new();
        match &rec.model {
            Some(m) if *m != model => diffs.push(format!("model\n  was {m}\n  now {model}")),
            Some(_) => {}
            None => rec.model = Some(model),
        }
        if rec.world.is_some() && rec.world != w.as_ref().map(|w| w.0) {
            if let Ok(dir) = std::env::var("TIMETRAVEL_FUZZ_DUMP") {
                let now = physics::with_world(DEFAULT_WORLD, |w| w.snapshot()).unwrap();
                std::fs::write(format!("{dir}/now.json"), now).unwrap();
                if let Some(was) = DUMP.with(|d| d.borrow().get(&frame).cloned()) {
                    std::fs::write(format!("{dir}/was.json"), was).unwrap();
                }
            }
            diffs.push(format!("world bytes (fixed {:?} vs {:?})", rec.fixed, w.as_ref().map(|w| w.1)));
        }
        if rec.pending.is_some() && rec.pending != w.as_ref().map(|w| w.2.clone()) {
            diffs.push(format!(
                "in-flight physics commands\n  was {:?}\n  now {:?}",
                rec.pending,
                w.as_ref().map(|w| &w.2)
            ));
        }
        match rec.frame {
            Some(f) if f != drawn => diffs.push("drawn frame".to_string()),
            Some(_) => {}
            None => rec.frame = Some(drawn),
        }
        if diffs.is_empty() {
            count("frames verified");
            Ok(())
        } else {
            Err(Failure {
                op,
                kind,
                detail: format!("frame {frame}: {}", diffs.join("; ")),
            })
        }
    }

    /// A seek or rewind discards input since the frame, as the producer does.
    fn drop_pending(&mut self) {
        self.pending.clear();
        self.reloaded_pending = false;
    }

    fn rebase(&mut self) {
        if let Some(tts) = self.game.current_scene_tts() {
            self.clock.rebase(tts as f32);
        }
    }

    fn seek(&mut self, target: u64) -> Check {
        let Some((lo, hi)) = self.range() else {
            return Ok(());
        };
        match self.game.seek_scene_to(target) {
            Ok(_) => {}
            Err(e) => return Err(self.fail(Kind::Refused, format!("seek {target}: {e}"))),
        }
        self.drop_pending();
        let want = target.clamp(lo, hi);
        let got = self.game.current_scene_frame();
        if got != Some(want) {
            return Err(self.fail(Kind::Lockstep, format!("seek {target} landed on {got:?}")));
        }
        self.rebase();
        self.check_fidelity(want, Kind::Fidelity)
    }

    fn run(&mut self, op: &Op) -> Check {
        match op {
            Op::Play { frames, seed } => {
                let mut rng = Rng::new(*seed as u64);
                for _ in 0..*frames {
                    self.random_input(&mut rng);
                    self.render_frame(FIXED_DT)?;
                    self.check_lockstep()?;
                }
            }
            Op::Pause => self.clock.pause(),
            Op::Resume => {
                if self.clock.is_paused() {
                    self.rebase();
                    self.clock.resume();
                }
            }
            Op::Step => {
                self.clock.step(1.0 / 60.0);
                self.render_frame(FIXED_DT)?;
            }
            Op::Seek(target) => {
                let target = self.resolve(*target);
                self.seek(target)?;
                self.clock.pause();
            }
            Op::PastEnd(n) => {
                let newest = self.resolve(Target::Newest);
                self.seek(newest)?;
                self.clock.step_frames(*n as u32);
                self.render_frame(FIXED_DT)?;
            }
            Op::Rewind(target) => {
                let target = self.resolve(*target);
                if self.range().is_some() {
                    if let Err(e) = self.game.rewind_scene_to(target) {
                        return Err(self.fail(Kind::Refused, format!("rewind {target}: {e}")));
                    }
                    self.drop_pending();
                    self.rebase();
                    let frame = self.game.current_scene_frame().expect("rewound");
                    self.oracle.split_off(&(frame + 1));
                    self.check_fidelity(frame, Kind::Fidelity)?;
                }
            }
            Op::Reload { semantic, n } => self.reload(*semantic, *n)?,
            Op::Replay { back, frames } => self.replay(*back, *frames)?,
        }
        self.check_lockstep()
    }

    fn reload(&mut self, semantic: bool, n: u32) -> Check {
        if semantic {
            let value = ["0.3", "0.6", "0.9"][n as usize % 3];
            self.knob = Some(self.spec.knob.1.replace("{}", value));
        } else {
            self.noop += 1;
        }
        let before = (
            self.game.state_debug(),
            self.range(),
            self.game.current_scene_frame(),
            world(),
        );
        count(if self.clock.is_paused() { "reloads while paused" } else { "reloads" });
        self.reloaded_pending |= !self.pending.is_empty();
        let status = match self.game.reload_source(&self.source()) {
            Ok(status) => status,
            Err(e) => return Err(self.fail(Kind::Reload, format!("reload rejected: {e}"))),
        };
        // An input-only game's reload while scrubbed rebuilds its history under
        // the new code, by design.
        let rebuilt = status.contains("history recomputed");
        if semantic {
            self.rev += 1;
        }
        for rec in self.oracle.values_mut() {
            if rebuilt {
                rec.model = None;
            }
            if semantic || rebuilt {
                // New code draws the retained frames anew.
                rec.frame = None;
            }
        }
        let after = (
            self.game.state_debug(),
            self.range(),
            self.game.current_scene_frame(),
            world(),
        );
        // History holding a closure can't cross a reload: it restarts at the
        // selected frame, by design.
        let restarted =
            self.spec.closures && after.1 != before.1 && after.2.map(|f| (f, f)) == after.1;
        if restarted {
            // The new generation is seeded from the live model, which may hold
            // input since the frame (a known gap): adopt it.
            let frame = after.2.expect("restarted at a frame");
            self.oracle.retain(|f, _| *f == frame);
            if let Some(rec) = self.oracle.get_mut(&frame) {
                rec.model = None;
                rec.frame = None;
            }
        }
        let same = (rebuilt || before.0 == after.0)
            && (restarted || before.1 == after.1)
            && (&before.2, &before.3) == (&after.2, &after.3);
        if !same {
            return Err(self.fail(
                Kind::Reload,
                format!("reload changed (model, range, frame, world): {before:?} -> {after:?}"),
            ));
        }
        // The live state may hold input since the frame; a later seek checks
        // the snapshot itself.
        Ok(())
    }

    fn replay(&mut self, back: u16, frames: u8) -> Check {
        let Some((lo, hi)) = self.range() else {
            return Ok(());
        };
        let k = hi.saturating_sub(back as u64).max(lo);
        let n = (frames as u64).min(hi - k);
        // Only frames recorded under the current code, fully known.
        let window: Vec<Rec> = (k..=k + n).filter_map(|f| self.oracle.get(&f).cloned()).collect();
        if n == 0
            || window.len() as u64 != n + 1
            || window
                .iter()
                .any(|r| {
                    r.rev != self.rev
                        || r.model.is_none()
                        || r.frame.is_none()
                        || r.reload_dropped_queries
                })
        {
            return Ok(());
        }
        count("replays");
        let logged: Vec<Vec<RecordedInput>> =
            (k + 1..=k + n).map(|f| self.game.recorded_inputs_at(f)).collect();
        if let Err(e) = self.game.rewind_scene_to(k) {
            return Err(self.fail(Kind::Refused, format!("replay rewind {k}: {e}")));
        }
        self.drop_pending();
        self.rebase();
        self.clock.resume();
        self.check_fidelity(k, Kind::Fidelity)?;
        for (i, inputs) in logged.into_iter().enumerate() {
            let frame = k + 1 + i as u64;
            let mut sampled = None;
            for input in inputs {
                match input {
                    RecordedInput::Snapshot(snap) => sampled = Some(*snap),
                    other => self.feed(other),
                }
            }
            let ft = self.clock.fixed_frames(FIXED_DT);
            assert_eq!(ft.len(), 1, "one fixed step per rendered frame");
            let selected = self.game.current_scene_frame();
            if let Some(snap) = sampled {
                self.game.sampled_input(&snap);
                self.pending.push(RecordedInput::Snapshot(Box::new(snap)));
            }
            self.game.tick(ft[0]);
            let fed = std::mem::take(&mut self.pending);
            if format!("{:?}", self.game.recorded_inputs_at(frame)) != format!("{fed:?}") {
                return Err(self.fail(Kind::InputLog, format!("replayed frame {frame}: log differs")));
            }
            if self.game.current_scene_frame() != selected.map(|s| s + 1) {
                return Err(self.fail(Kind::FrameGap, format!("replayed frame {frame} recorded elsewhere")));
            }
            self.check_fidelity(frame, Kind::ReplayDiverged)?;
            count("replayed frames verified");
        }
        // Frames past the replay were branched away; the shell's keys follow
        // the replayed log.
        self.oracle.split_off(&(k + n + 1));
        self.held = window[n as usize].held.clone();
        self.edges = InputEdges::default();
        Ok(())
    }
}

// ── Driver ──────────────────────────────────────────────────────────────

type Stats = BTreeMap<&'static str, u64>;

/// Run `ops` on a fresh game on a fresh thread — thread-locals (the physics
/// world, memo tables) start clean, so a repro depends on its ops alone. The
/// first failure, if any, and what the run verified.
fn execute(spec: &Spec, ops: &[Op]) -> (Option<Failure>, Stats) {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        // Caught panics become failures: keep their spew out, keep the site.
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if QUIET.get() {
                let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line()));
                PANIC_AT.set(at.unwrap_or_default());
            } else {
                default(info)
            }
        }));
    });
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                QUIET.set(true);
                let at = Cell::new(0);
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut shell = Shell::new(spec);
                    for (i, op) in ops.iter().enumerate() {
                        at.set(i);
                        shell.op = i;
                        shell.run(op)?;
                    }
                    Ok(())
                }));
                let failure = match run {
                    Ok(checked) => checked.err(),
                    Err(payload) => {
                        let message = payload
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "non-string panic".to_string());
                        Some(Failure {
                            op: at.get(),
                            kind: Kind::Panic,
                            detail: format!("{message}{}", PANIC_AT.take()),
                        })
                    }
                };
                (failure, STATS.take())
            })
            .join()
            .expect("the run thread catches its panics")
    })
}

/// Delta-debug `ops` down to a minimal sequence failing the same way.
fn shrink(spec: &Spec, mut ops: Vec<Op>, mut failure: Failure) -> (Vec<Op>, Failure) {
    let kind = failure.kind.clone();
    let still = |ops: &[Op]| execute(spec, ops).0.filter(|f| f.kind == kind);
    ops.truncate(failure.op + 1);
    let mut chunk = ops.len() / 2;
    while chunk >= 1 {
        let mut i = 0;
        while i < ops.len() {
            let mut candidate = ops.clone();
            candidate.drain(i..(i + chunk).min(ops.len()));
            match still(&candidate) {
                Some(f) => {
                    ops = candidate;
                    failure = f;
                }
                None => i += chunk,
            }
        }
        chunk /= 2;
    }
    // Then shorten each Play.
    for i in 0..ops.len() {
        while let Op::Play { frames, seed } = ops[i] {
            if frames <= 1 {
                break;
            }
            let mut candidate = ops.clone();
            candidate[i] = Op::Play {
                frames: frames / 2,
                seed,
            };
            match still(&candidate) {
                Some(f) => {
                    ops = candidate;
                    failure = f;
                }
                None => break,
            }
        }
    }
    (ops, failure)
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn fuzz(game: &str, default_seeds: u64, default_ops: u64) {
    if let Ok(only) = std::env::var("TIMETRAVEL_FUZZ_GAME") {
        if only != game {
            return;
        }
    }
    let spec = specs().into_iter().find(|s| s.name == game).expect("spec");
    let seeds = env("TIMETRAVEL_FUZZ_SEEDS", default_seeds);
    let ops = env("TIMETRAVEL_FUZZ_OPS", default_ops) as usize;
    let first = env("TIMETRAVEL_FUZZ_SEED0", 1);
    let mut failures = Vec::new();
    let mut verified = Stats::new();
    for seed in first..first + seeds {
        let sequence = gen_ops(seed ^ hash(game.as_bytes()), ops);
        let (found, stats) = execute(&spec, &sequence);
        for (what, n) in stats {
            *verified.entry(what).or_default() += n;
        }
        if let Some(found) = found {
            let (minimal, failure) = shrink(&spec, sequence, found);
            failures.push(format!(
                "{game} seed {seed}: {:?} at op {}: {}\n  repro ({} ops): {minimal:?}",
                failure.kind,
                failure.op,
                failure.detail,
                minimal.len()
            ));
        }
    }
    eprintln!("{game}: {seeds} seeds x {ops} ops: {verified:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn fuzz_character() {
    fuzz("character", 2, 60);
}

#[test]
fn fuzz_physics() {
    fuzz("physics", 2, 60);
}

#[test]
fn fuzz_platformer() {
    fuzz("platformer", 2, 60);
}

#[test]
fn fuzz_crowd() {
    fuzz("crowd", 2, 60);
}

#[test]
fn fuzz_night_terrain() {
    fuzz("night-terrain", 2, 60);
}

#[test]
fn fuzz_closure() {
    fuzz("closure", 2, 60);
}

#[test]
fn fuzz_memo() {
    fuzz("memo", 2, 60);
}
