//! The time-travel fuzzer (docs/time-travel.md, "Fuzzing"): seeded random
//! sequences of play / pause / scrub / branch / hot reload over the embedded
//! (web + Quest) producer, driven the way a shell drives it (a [`GameClock`],
//! one fixed step per rendered frame). The desktop producer shares its
//! recorder and frame body. Input while paused models debug `POST /input`
//! (shells ignore window input while paused).
//!
//! After every operation it checks that the independent histories — the model
//! snapshot ring, the physics timeline, the clock, and every recorder
//! [`Participant`](super::Participant) — stay in lockstep, and that seeking to
//! a recorded frame restores exactly what that frame recorded.
//!
//! `functor fuzz` runs it over any project; `tests/timetravel_fuzz.rs` over the
//! examples and fixtures. Hot reloads never touch disk: they push an edited
//! copy of the entry source.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use super::{fnv1a, Channel};
use crate::functor_lang_game_embedded::{FunctorLangEmbeddedGame, NativePlatform, ProducerPlatform};
use crate::functor_lang_producer::EntryRole;
use crate::physics::{self, DEFAULT_WORLD, FIXED_DT};
use crate::protocol::GameProducer;
use crate::{
    FrameTime, GameClock, InputEdges, InputSnapshot, Key, RecordedInput, XrControllerSnapshot,
    XrInputSnapshot,
};

// ── Games ───────────────────────────────────────────────────────────────

/// What to fuzz and which input to drive it with.
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: String,
    /// Entry FIRST, then siblings, as `(file name, source)`.
    pub sources: Vec<(String, String)>,
    pub role: EntryRole,
    /// Keys pressed and released at random.
    pub keys: Vec<Key>,
    pub mouse: bool,
    /// XR thumbsticks and the primary button, through sampled input.
    pub xr: bool,
    /// A tunable constant in the entry: `(exact source text, replacements)`.
    /// A semantic reload swaps in one replacement; a no-op reload only appends
    /// an unused binding. `None`: every reload is a no-op one.
    pub knob: Option<(String, Vec<String>)>,
    /// Point the save store here, emptied before every run, so a repro
    /// depends on its ops alone. `None`: leave the store as configured.
    pub saves: Option<PathBuf>,
    /// Whether a reload may restart history (the model holds closures).
    /// `None`: whatever the recorder predicts.
    pub restarts: Option<bool>,
    /// Answer the game's HTTP requests over a seeded fake transport.
    pub http: bool,
}

/// The optional hooks a loaded program defines.
pub(crate) struct Hooks {
    pub input: bool,
    pub mouse_move: bool,
    pub physics: bool,
}

impl Spec {
    /// A spec driving no input, for `..Spec::new(…)` updates.
    pub fn new(name: &str, sources: Vec<(String, String)>) -> Spec {
        Spec {
            name: name.to_string(),
            sources,
            role: EntryRole::Prefix(String::new()),
            keys: Vec::new(),
            mouse: false,
            xr: false,
            knob: None,
            saves: None,
            restarts: Some(false),
            http: false,
        }
    }

    /// Load the program once and drive what it reads: the keys its source
    /// names (every key if it reads keys but names none), the mouse if it
    /// defines `mouseMove`, XR sticks if its sampled input mentions `xr`. The
    /// knob is the entry's first top-level nonzero float constant used
    /// elsewhere, nudged up by 1, 2 or 3 (keeping whole numbers whole).
    pub fn derive(
        name: &str,
        sources: Vec<(String, String)>,
        role: EntryRole,
    ) -> Result<Spec, String> {
        let game = FunctorLangEmbeddedGame::create_for_role(
            sources.clone(),
            role.clone(),
            Box::new(NativePlatform),
        )?;
        let (hooks, sampled) = (game.fuzz_hooks(), game.samples_input());
        let all: String = sources.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join("\n");
        let reads_keys = hooks.input || sampled;
        let mut keys: Vec<Key> = Key::ALL
            .into_iter()
            .filter(|k| reads_keys && k.ctor_tag().is_some_and(|tag| mentions(&all, &tag)))
            .collect();
        if reads_keys && keys.is_empty() {
            keys = Key::ALL[1..].to_vec();
        }
        let knob = sources.first().and_then(|(_, entry)| knob_in(entry, &all));
        Ok(Spec {
            keys,
            mouse: hooks.mouse_move,
            xr: sampled && mentions(&all, "xr"),
            knob,
            role,
            restarts: None,
            http: mentions(&all, "Effect.httpGet") || mentions(&all, "Effect.httpPost"),
            ..Spec::new(name, sources)
        })
    }
}

/// How often `word` occurs in `text` as a whole identifier path (`Key.W`,
/// not `Key.WX`).
fn occurrences(text: &str, word: &str) -> usize {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word)
        .filter(|(at, _)| {
            !text[..*at].chars().next_back().is_some_and(ident)
                && !text[at + word.len()..].chars().next().is_some_and(ident)
        })
        .count()
}

fn mentions(text: &str, word: &str) -> bool {
    occurrences(text, word) > 0
}

/// The first top-level `let name = <nonzero float>` line whose name is used
/// elsewhere, with that value plus 1, 2 and 3.
fn knob_in(entry: &str, all: &str) -> Option<(String, Vec<String>)> {
    entry.lines().find_map(|line| {
        let (name, value) = line.strip_prefix("let ")?.split_once(" = ")?;
        let value = value.trim_end();
        let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        let number = value.parse::<f64>().ok().filter(|v| *v != 0.0 && value.contains('.'))?;
        (plain && occurrences(all, name) > 1).then(|| {
            let nudged = [1.0, 2.0, 3.0].map(|d| format!("let {name} = {:?}", number + d));
            (line.trim_end().to_string(), nudged.to_vec())
        })
    })
}

// ── Operations ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Back(u16),
    Oldest,
    Newest,
    /// Past the newest frame: the recorder clamps.
    Beyond(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
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
    /// log over `frames` frames — on a fork (recorded answers consumed), then
    /// live (input fed through the shell): each must reproduce what was
    /// recorded.
    Replay { back: u16, frames: u8 },
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Target::Back(n) => write!(f, "back:{n}"),
            Target::Oldest => write!(f, "oldest"),
            Target::Newest => write!(f, "newest"),
            Target::Beyond(n) => write!(f, "beyond:{n}"),
        }
    }
}

/// The `--replay` spelling: `play:40:7 seek:back:32 reload:knob:2 …`.
impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Op::Play { frames, seed } => write!(f, "play:{frames}:{seed}"),
            Op::Pause => write!(f, "pause"),
            Op::Resume => write!(f, "resume"),
            Op::Step => write!(f, "step"),
            Op::Seek(t) => write!(f, "seek:{t}"),
            Op::PastEnd(n) => write!(f, "pastend:{n}"),
            Op::Rewind(t) => write!(f, "rewind:{t}"),
            Op::Reload { semantic: false, .. } => write!(f, "reload"),
            Op::Reload { semantic: true, n } => write!(f, "reload:knob:{n}"),
            Op::Replay { back, frames } => write!(f, "replay:{back}:{frames}"),
        }
    }
}

/// Ops as `--replay` takes them, space-separated.
pub fn format_ops(ops: &[Op]) -> String {
    ops.iter().map(Op::to_string).collect::<Vec<_>>().join(" ")
}

/// Parse [`format_ops`]'s text (spaces or commas between ops).
pub fn parse_ops(text: &str) -> Result<Vec<Op>, String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|word| parse_op(word).ok_or_else(|| format!("unknown fuzz op `{word}`")))
        .collect()
}

fn parse_op(word: &str) -> Option<Op> {
    fn num<T: std::str::FromStr>(parts: &[&str], i: usize) -> Option<T> {
        parts.get(i)?.parse().ok()
    }
    let parts: Vec<&str> = word.split(':').collect();
    let target = |rest: &[&str]| match rest {
        ["back", n] => Some(Target::Back(n.parse().ok()?)),
        ["oldest"] => Some(Target::Oldest),
        ["newest"] => Some(Target::Newest),
        ["beyond", n] => Some(Target::Beyond(n.parse().ok()?)),
        _ => None,
    };
    Some(match parts.as_slice() {
        ["play", _, _] => Op::Play { frames: num(&parts, 1)?, seed: num(&parts, 2)? },
        ["pause"] => Op::Pause,
        ["resume"] => Op::Resume,
        ["step"] => Op::Step,
        ["seek", rest @ ..] => Op::Seek(target(rest)?),
        ["pastend", _] => Op::PastEnd(num(&parts, 1)?),
        ["rewind", rest @ ..] => Op::Rewind(target(rest)?),
        ["reload"] => Op::Reload { semantic: false, n: 0 },
        ["reload", "knob", _] => Op::Reload { semantic: true, n: num(&parts, 2)? },
        ["replay", _, _] => Op::Replay { back: num(&parts, 1)?, frames: num(&parts, 2)? },
        _ => return None,
    })
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

/// `count` random ops from `seed`.
pub fn gen_ops(seed: u64, count: usize) -> Vec<Op> {
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
    frame: Option<Drawn>,
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
pub enum Kind {
    /// The program did not load.
    Load,
    Panic,
    /// A frame recorded somewhere other than the selected frame + 1.
    FrameGap,
    /// Model / physics / clock disagree about the current frame.
    Lockstep,
    /// A seek restored different bytes than the frame recorded.
    Fidelity,
    /// Branch-and-replay of the recorder's input log diverged.
    ReplayDiverged,
    /// The fork replay of the recorder's log diverged.
    ForkDiverged,
    /// The recorder's input log differs from what the shell fed.
    InputLog,
    /// An exact seek inside the scrubber range was refused.
    Refused,
    /// A no-op reload changed the selected state or the range.
    Reload,
    /// A recorder participant's range or restore disagrees with the rest.
    Participant,
    /// A request was dispatched twice or without its op, hangs (pending with
    /// nothing in flight or parked to answer it), or a reload lost it.
    Request,
}

#[derive(Debug, Clone)]
pub struct Failure {
    /// Index of the failing op.
    pub op: usize,
    pub kind: Kind,
    pub detail: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?} at op {}: {}", self.kind, self.op, self.detail)
    }
}

/// The live world: `(hash of its bytes minus pending commands, frame,
/// pending commands)`. Pending commands are checked on their own: input
/// between frames legitimately queues them on the live world.
fn world() -> Option<(u64, u64, String)> {
    physics::with_world(DEFAULT_WORLD, |w| world_parts(w))
}

fn world_parts(w: &physics::World) -> (u64, u64, String) {
    let mut core = w.clone();
    let pending = core.take_pending_commands();
    (fnv1a(&core.snapshot()), w.frame(), format!("{pending:?}"))
}

/// A drawn frame's hash; `None` when `draw` failed. The producer then shows
/// its last good frame, which depends on history, not on this frame.
type Drawn = Option<u64>;

/// [`NativePlatform`], noting whether the latest `draw` failed.
struct FuzzPlatform(Rc<Cell<bool>>);

impl ProducerPlatform for FuzzPlatform {
    fn install_sinks(&self) {
        NativePlatform.install_sinks()
    }
    fn set_draw_overlay(&mut self, error: Option<&str>) {
        self.0.set(error.is_some());
    }
    fn on_reload(&mut self) {}
}

struct Shell<'a> {
    spec: &'a Spec,
    game: FunctorLangEmbeddedGame,
    draw_failed: Rc<Cell<bool>>,
    /// The game defines `input` (key events reach it and its log) / `physics`
    /// (every played frame steps the world once).
    events: bool,
    physics: bool,
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
    net: Option<FakeNet>,
}

/// A seeded fake HTTP transport: takes the requests the game queued and
/// answers each a random number of transport ticks later — playing, paused
/// or scrubbed alike — with a body naming its URL and token.
struct FakeNet {
    rng: Rng,
    /// `(token, ticks left, url)`.
    in_flight: Vec<(u64, u32, String)>,
    dispatched: std::collections::BTreeSet<u64>,
}

type Check = Result<(), Failure>;

thread_local! {
    /// Recorded world JSON per frame, kept only under `TIMETRAVEL_FUZZ_DUMP`.
    static DUMP: RefCell<BTreeMap<u64, Vec<u8>>> = Default::default();
    /// Set while a fuzz run's panics are caught and reported.
    static QUIET: Cell<bool> = const { Cell::new(false) };
    /// What the run on this thread verified.
    static STATS: RefCell<Stats> = Default::default();
    /// Where the last caught panic happened.
    static PANIC_AT: RefCell<String> = Default::default();
}

fn count(what: &'static str) {
    STATS.with(|s| *s.borrow_mut().entry(what).or_default() += 1);
}

impl<'a> Shell<'a> {
    fn new(spec: &'a Spec) -> Result<Shell<'a>, String> {
        let draw_failed = Rc::new(Cell::new(false));
        let mut game = FunctorLangEmbeddedGame::create_for_role(
            spec.sources.clone(),
            spec.role.clone(),
            Box::new(FuzzPlatform(draw_failed.clone())),
        )
        .map_err(|e| format!("{}: {e}", spec.name))?;
        game.seed_random(1);
        let hooks = game.fuzz_hooks();
        let mut shell = Shell {
            spec,
            game,
            draw_failed,
            events: hooks.input,
            physics: hooks.physics,
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
            net: spec.http.then(|| FakeNet {
                rng: Rng::new(7),
                in_flight: Vec::new(),
                dispatched: Default::default(),
            }),
        };
        // The shell's bootstrap: a zero-delta first frame.
        shell.render_frame(0.0).expect("bootstrap");
        Ok(shell)
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
        self.pump()
    }

    /// One transport tick: answer what is due, then dispatch what the game
    /// queued (including requests those answers chained).
    fn pump(&mut self) -> Check {
        let Some(net) = self.net.as_mut() else {
            return Ok(());
        };
        let due: Vec<(u64, String)> = net
            .in_flight
            .iter()
            .filter(|(_, ticks, _)| *ticks == 0)
            .map(|(token, _, url)| (*token, url.clone()))
            .collect();
        net.in_flight.retain_mut(|(_, ticks, _)| {
            let due = *ticks == 0;
            *ticks = ticks.saturating_sub(1);
            !due
        });
        for (token, url) in due {
            let was_pending = self.game.request_tokens().0.contains(&(Channel::Http, token));
            self.game
                .net_push_http_response(token as i32, 200, format!("{url}#{token}"));
            let parked = self.game.request_tokens().1.contains(&(Channel::Http, token));
            count(match (was_pending, parked) {
                (false, _) => "late responses dropped",
                (true, true) => "responses parked",
                (true, false) => "responses delivered",
            });
        }
        let pending = self.game.request_tokens().0;
        let net = self.net.as_mut().expect("checked above");
        for command in crate::net::drain_commands() {
            let crate::net::NetCommand::HttpRequest { token, url, .. } = command;
            let fail = |detail: String| Failure { op: self.op, kind: Kind::Request, detail };
            if !net.dispatched.insert(token) {
                return Err(fail(format!("request {token} dispatched twice")));
            }
            if !pending.contains(&(Channel::Http, token)) {
                return Err(fail(format!("request {token} dispatched without a pending op")));
            }
            count("requests dispatched");
            net.in_flight.push((token, net.rng.below(40) as u32, url));
        }
        Ok(())
    }

    /// No hung op: every request pending on the tail is still in flight or
    /// parked — something will answer it.
    fn check_requests(&self) -> Check {
        let Some(net) = &self.net else {
            return Ok(());
        };
        let (pending, parked) = self.game.request_tokens();
        for (channel, token) in pending {
            let answered_later = parked.contains(&(channel, token))
                || net.in_flight.iter().any(|(t, _, _)| *t == token);
            if channel == Channel::Http && !answered_later {
                return Err(self.fail(
                    Kind::Request,
                    format!("request {token} hangs: pending, with nothing in flight or parked"),
                ));
            }
        }
        count("request checks");
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
        self.check_participants(frame)?;
        let w = world();
        // One fixed step per recorded frame, from where the selected frame ended.
        let from = selected.and_then(|s| self.oracle.get(&s)).and_then(|r| r.fixed);
        if let (true, Some(from), Some(now)) = (self.physics, from, w.as_ref().map(|w| w.1)) {
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

    fn draw_hash(&mut self, tts: f32) -> Drawn {
        let frame = self.game.render(FrameTime { dts: 0.0, tts });
        (!self.draw_failed.get())
            .then(|| fnv1a(&serde_json::to_vec(&frame).expect("frame serializes")))
    }

    fn feed(&mut self, input: RecordedInput) {
        match &input {
            RecordedInput::Key { code, is_down } => self.game.key_event(*code, *is_down),
            RecordedInput::MouseMove { x, y } => self.game.mouse_move(*x, *y),
            other => panic!("the fuzzer never feeds {other:?}"),
        }
        // Without an `input` hook a key event is dropped unlogged.
        if self.events || !matches!(input, RecordedInput::Key { .. }) {
            self.pending.push(input);
        }
    }

    fn random_input(&mut self, rng: &mut Rng) {
        let keys = &self.spec.keys;
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

    /// The current entry source: the spec's, with the knob and no-op bindings
    /// applied. In memory only: the reload pushes it.
    fn source(&self) -> String {
        let mut src = self.spec.sources[0].1.clone();
        if let (Some(knob), Some((text, _))) = (&self.knob, &self.spec.knob) {
            assert!(src.contains(text.as_str()), "{}: knob not found", self.spec.name);
            src = src.replace(text.as_str(), knob);
        }
        for i in 0..self.noop {
            src.push_str(&format!("\nlet fuzzReload{i} = {i}.0\n"));
        }
        src
    }

    /// The spec's sources with [`Self::source`] as the entry.
    fn sources(&self) -> Vec<(String, String)> {
        let mut sources = self.spec.sources.clone();
        sources[0].1 = self.source();
        sources
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

    /// Every participant's live state is what `frame` (the selected frame)
    /// recorded — right after it records, and after a seek or rewind to it —
    /// and every participant retains the seekable window (they record, prune
    /// and truncate together).
    fn check_participants(&mut self, frame: u64) -> Check {
        let window = self.range();
        for state in self.game.participant_states() {
            if state.range != window {
                return Err(self.fail(
                    Kind::Participant,
                    format!("{} retains {:?}, the window is {window:?}", state.name, state.range),
                ));
            }
            if state.recorded != Some(state.live) {
                return Err(self.fail(
                    Kind::Participant,
                    format!(
                        "frame {frame}: {} live digest {:x}, recorded {:?}",
                        state.name, state.live, state.recorded
                    ),
                ));
            }
        }
        count("participant checks");
        Ok(())
    }

    /// The live state equals what `frame` recorded (adopting unknowns).
    fn check_fidelity(&mut self, frame: u64, kind: Kind) -> Check {
        self.check_participants(frame)?;
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
        count("seeks");
        self.drop_pending();
        let want = target.clamp(lo, hi);
        let got = self.game.current_scene_frame();
        if got != Some(want) {
            return Err(self.fail(Kind::Lockstep, format!("seek {target} landed on {got:?}")));
        }
        self.rebase();
        self.check_fidelity(want, Kind::Fidelity)
    }

    /// A destructive rewind to `target`: every participant ends there.
    fn rewind(&mut self, target: u64) -> Result<u64, Failure> {
        if let Err(e) = self.game.rewind_scene_to(target) {
            return Err(self.fail(Kind::Refused, format!("rewind {target}: {e}")));
        }
        count("rewinds");
        self.drop_pending();
        self.rebase();
        let frame = self.game.current_scene_frame().expect("rewound");
        if self.range().map(|r| r.1) != Some(frame) {
            return Err(self.fail(
                Kind::Lockstep,
                format!("rewind to {frame} kept {:?}", self.range()),
            ));
        }
        Ok(frame)
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
                    let frame = self.rewind(target)?;
                    self.oracle.split_off(&(frame + 1));
                    self.check_fidelity(frame, Kind::Fidelity)?;
                }
            }
            Op::Reload { semantic, n } => self.reload(*semantic, *n)?,
            Op::Replay { back, frames } => self.replay(*back, *frames)?,
        }
        self.pump()?;
        self.check_requests()?;
        self.check_lockstep()
    }

    fn reload(&mut self, semantic: bool, n: u32) -> Check {
        let previous_knob = self.knob.clone();
        let semantic = match (&self.spec.knob, semantic) {
            (Some((_, replacements)), true) => {
                self.knob = Some(replacements[n as usize % replacements.len()].clone());
                true
            }
            _ => {
                self.noop += 1;
                false
            }
        };
        let before = (
            self.game.state_debug(),
            self.range(),
            self.game.current_scene_frame(),
            world(),
        );
        let generation = self.game.scene_timeline_generation();
        // History holding code (closures) can't cross a reload: it restarts at
        // the selected frame, by design.
        let may_restart = self
            .spec
            .restarts
            .unwrap_or_else(|| self.game.reload_may_restart_history());
        count(if self.clock.is_paused() { "reloads while paused" } else { "reloads" });
        let in_flight = self.game.request_tokens();
        let status = match self.game.reload_source(&self.source()) {
            Ok(status) => status,
            // The edited constant broke the program (a fresh load fails too):
            // a rejected push keeps the old one, and nothing else.
            Err(_) if semantic && FunctorLangEmbeddedGame::loads(&self.sources(), &self.spec.role).is_err() => {
                self.knob = previous_knob;
                count("reloads rejected");
                let after = (
                    self.game.state_debug(),
                    self.range(),
                    self.game.current_scene_frame(),
                    world(),
                );
                if self.game.scene_timeline_generation() != generation || after != before {
                    return Err(self.fail(
                        Kind::Reload,
                        format!("a rejected reload changed (model, range, frame, world): {before:?} -> {after:?}"),
                    ));
                }
                return Ok(());
            }
            Err(e) => return Err(self.fail(Kind::Reload, format!("reload rejected: {e}"))),
        };
        self.reloaded_pending |= !self.pending.is_empty();
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
        let restarted = self.game.scene_timeline_generation() != generation;
        if restarted && !(may_restart && after.2.map(|f| (f, f)) == after.1) {
            return Err(self.fail(Kind::Reload, format!("reload restarted history: {after:?}")));
        }
        // A reload that keeps the timeline keeps every request in flight.
        if !restarted && self.game.request_tokens() != in_flight {
            return Err(self.fail(
                Kind::Request,
                format!(
                    "a reload changed the in-flight requests: {in_flight:?} -> {:?}",
                    self.game.request_tokens()
                ),
            ));
        }
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
        self.fork_replay(k, n)?;
        if (k + 1..=k + n).any(|f| self.game.logged_answers(f)) || self.game.arrivals_after(k) {
            // A live branch asks the world afresh, and gets its answers when
            // the transport says: nothing to reproduce.
            return Ok(());
        }
        count("replays");
        let logged: Vec<Vec<RecordedInput>> =
            (k + 1..=k + n).map(|f| self.game.recorded_inputs_at(f)).collect();
        self.rewind(k)?;
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

    /// Scrub to `k`, then replay the recorder's own log for `k+1..=k+n` on a
    /// fork: each frame's model and world must equal what it recorded.
    fn fork_replay(&mut self, k: u64, n: u64) -> Check {
        self.seek(k)?;
        self.clock.pause();
        let replayed = self
            .game
            .replay_scene(n as usize)
            .map_err(|e| self.fail(Kind::ForkDiverged, format!("fork from {k}: {e}")))?;
        if self.net.is_some() && !crate::net::drain_commands().is_empty() {
            return Err(self.fail(Kind::Request, format!("fork from {k} dispatched a request")));
        }
        if replayed.len() as u64 != n {
            return Err(self.fail(
                Kind::ForkDiverged,
                format!("fork from {k}: replayed {} of {n} frames", replayed.len()),
            ));
        }
        for (i, (model, world)) in replayed.into_iter().enumerate() {
            let frame = k + 1 + i as u64;
            let rec = &self.oracle[&frame];
            let mut diffs = Vec::new();
            if rec.model.as_ref() != Some(&model) {
                diffs.push(format!("model\n  was {:?}\n  now {model}", rec.model));
            }
            let now = world.map(|world| {
                let id = physics::create_world([0.0, -9.81, 0.0]);
                let parts = physics::with_world(id, |w| {
                    w.restore_checkpoint(&world);
                    world_parts(w)
                });
                physics::remove_world(id);
                parts.expect("just created")
            });
            if self.physics && rec.world != now.as_ref().map(|w| w.0) {
                diffs.push("world bytes".to_string());
            }
            if self.physics && rec.pending != now.map(|w| w.2) {
                diffs.push("in-flight physics commands".to_string());
            }
            if !diffs.is_empty() {
                return Err(self.fail(
                    Kind::ForkDiverged,
                    format!("fork from {k}, frame {frame}: {}", diffs.join("; ")),
                ));
            }
            count("fork frames verified");
        }
        count("fork replays");
        Ok(())
    }
}

// ── Driver ──────────────────────────────────────────────────────────────

/// What a run verified, by counter (`frames recorded`, `seeks`, …).
pub type Stats = BTreeMap<&'static str, u64>;

/// Run `ops` on a fresh game on a fresh thread — thread-locals (the physics
/// world, memo tables) start clean, so a repro depends on its ops alone. The
/// first failure, if any, and what the run verified.
pub fn execute(spec: &Spec, ops: &[Op]) -> (Option<Failure>, Stats) {
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
        std::thread::Builder::new()
            // Deep game programs recurse further than the default 2 MiB.
            .stack_size(64 << 20)
            .spawn_scoped(scope, || {
                QUIET.set(true);
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(dir) = &spec.saves {
                    let _ = std::fs::remove_dir_all(dir);
                    crate::storage::set_project_root(dir.clone());
                }
                let at = Cell::new(0);
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut shell = Shell::new(spec).map_err(|detail| Failure {
                        op: 0,
                        kind: Kind::Load,
                        detail,
                    })?;
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
            .expect("spawn the fuzz thread")
            .join()
            .expect("the run thread catches its panics")
    })
}

/// Delta-debug `ops` down to a minimal sequence failing the same way.
pub fn shrink(spec: &Spec, mut ops: Vec<Op>, mut failure: Failure) -> (Vec<Op>, Failure) {
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

/// One failing seed, shrunk.
#[derive(Debug, Clone)]
pub struct Found {
    pub seed: u64,
    pub failure: Failure,
    /// The minimal op sequence failing the same way.
    pub repro: Vec<Op>,
}

/// Fuzz `spec` over `seeds`, `ops` ops each: what was verified, and each
/// failing seed shrunk. A seed's ops depend on it and the spec's name only.
pub fn run(spec: &Spec, seeds: impl IntoIterator<Item = u64>, ops: usize) -> (Stats, Vec<Found>) {
    let mut verified = Stats::new();
    let mut found = Vec::new();
    for seed in seeds {
        let sequence = gen_ops(seed ^ fnv1a(spec.name.as_bytes()), ops);
        let (failure, stats) = execute(spec, &sequence);
        for (what, n) in stats {
            *verified.entry(what).or_default() += n;
        }
        let Some(failure) = failure else { continue };
        if failure.kind == Kind::Load {
            // No op ran, and every seed would fail the same way.
            found.push(Found { seed, failure, repro: Vec::new() });
            break;
        }
        let (repro, failure) = shrink(spec, sequence, failure);
        found.push(Found { seed, failure, repro });
    }
    (verified, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_round_trip_through_their_text() {
        let ops = gen_ops(7, 200);
        assert_eq!(parse_ops(&format_ops(&ops)).unwrap().len(), ops.len());
        for (op, parsed) in ops.iter().zip(parse_ops(&format_ops(&ops)).unwrap()) {
            match op {
                // A no-op reload's `n` is unused.
                Op::Reload { semantic: false, .. } => assert_eq!(parsed.to_string(), "reload"),
                _ => assert_eq!(&parsed, op),
            }
        }
        assert!(parse_ops("play:3:1, seek:back:2 reload").is_ok());
        assert!(parse_ops("jump").unwrap_err().contains("`jump`"));
    }

    #[test]
    fn derive_drives_what_the_game_reads() {
        let src = "let speed = 2.0\nlet unused = 1.0\nlet init = { x: 0.0 }\n\
let input = (model, key, isDown) => match key with | Key.W => { x: model.x + speed } | _ => model\n\
let tick = (model, dt, tts) => model\n\
let draw = (model, tts) => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n";
        let spec = Spec::derive(
            "t",
            vec![("game.fun".into(), src.into())],
            EntryRole::Prefix(String::new()),
        )
        .unwrap();
        assert_eq!(spec.keys, [Key::W]);
        assert!(!spec.mouse && !spec.xr);
        let (text, nudged) = spec.knob.clone().unwrap();
        assert_eq!(text, "let speed = 2.0");
        assert_eq!(nudged, ["let speed = 3.0", "let speed = 4.0", "let speed = 5.0"]);
        let (failure, stats) = execute(&spec, &gen_ops(1, 30));
        assert!(failure.is_none(), "{failure:?}");
        assert!(stats["participant checks"] > 0);
    }

    /// A program the fuzzer's no-op reload breaks (it appends a duplicate
    /// `fuzzReload0`): a deterministic failure.
    const BROKEN_BY_RELOAD: &str = "let fuzzReload0 = 1.0\nlet init = 0.0\n\
let tick = (model, dt, tts) => model + 1.0\n\
let draw = (model, tts) => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n";

    #[test]
    fn a_shrunk_repro_fails_the_same_way() {
        let spec = Spec::new("broken", vec![("game.fun".into(), BROKEN_BY_RELOAD.into())]);
        let (_, found) = run(&spec, [1], 40);
        let found = &found[0];
        assert_eq!(found.failure.kind, Kind::Reload, "{}", found.failure);
        assert!(matches!(found.repro[..], [Op::Reload { .. }]), "{:?}", found.repro);
        let replayed = parse_ops(&format_ops(&found.repro)).unwrap();
        assert_eq!(execute(&spec, &replayed).0.map(|f| f.kind), Some(Kind::Reload));
    }
}
