//! The shell's one handle on the game producer.
//!
//! Every producer call goes through [`SimDriver`] as an ordered message:
//! `post` (and the named input methods) for fire-and-forget input and events,
//! `call` for queries that need an answer, and [`SimDriver::frame`] for the
//! per-frame [`SimSnapshot`] the render half consumes. The shell keeps the
//! clock and the fixed-step decomposition, so the producer sees the same call
//! stream however it is driven.
//!
//! Inline (the default) the producer runs on the shell's thread. With
//! `--pipelined` it runs on a sim thread: calls queue in order, and each
//! frame's snapshot lands in a single-slot mailbox the render side takes from,
//! so the sim draws frame N while the shell renders N-1. Feedback the shell
//! derives from a rendered frame (asset progress, preload settlement, terrain
//! hydration) then reaches the sim one frame later, so the shell runs lockstep
//! while that feedback is active and it lands on the same step as inline.

use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Instant;

use functor_runtime_common::audio::{AudioCommand, AudioScene};
use functor_runtime_common::net::{ConnCommand, NetCommand};
use functor_runtime_common::physics::DebugLine;
use functor_runtime_common::terrain::TerrainSource;
use functor_runtime_common::ui::{UiEvent, View};
use functor_runtime_common::{Frame, FrameTime, InputSnapshot};

use crate::game::Game;

/// What the shell wants back from one [`SimDriver::frame`] beyond the frame
/// itself. Optional parts cost producer work, so the shell asks only for what
/// it will show.
pub(crate) struct FrameRequest {
    pub time: FrameTime,
    pub ui: bool,
    pub webview: bool,
    pub audio_scene: bool,
    pub physics_lines: bool,
    /// False under the embedder transport: connection commands stay queued
    /// for the debug server's `GET /net/outbound`.
    pub conn_commands: bool,
}

/// Producer capabilities the shell routes input by. Read from the latest
/// snapshot, so a hot reload that adds or removes a hook takes effect one
/// frame later.
#[derive(Clone, Copy, Default)]
pub(crate) struct SimCaps {
    pub samples_input: bool,
    pub uses_captured_mouse_input: bool,
}

/// Everything the render half needs from one sim frame: the drawn frame, the
/// effects it queued, and the timeline/capability reads the shell's chrome
/// shows. Plain data; nothing here points back into the producer.
pub(crate) struct SimSnapshot {
    /// The [`SimDriver::frame`] request this answers.
    pub seq: u64,
    /// Producer time spent since the previous snapshot, this frame included.
    pub sim_ns: u64,
    /// The time the frame was drawn at, which render-time animation samples.
    pub time: FrameTime,
    pub frame: Arc<Frame>,
    pub ui: View,
    pub webview_html: Option<String>,
    pub audio_commands: Vec<AudioCommand>,
    pub audio_scene: Option<AudioScene>,
    pub net_commands: Vec<NetCommand>,
    pub conn_commands: Vec<ConnCommand>,
    pub preload_commands: Vec<functor_runtime_common::asset::preload::PreloadCommand>,
    /// Heightmaps physics asked for, for the shell's asset driver.
    pub terrain_requests: Vec<TerrainSource>,
    /// Some requested heightmap is not yet hydrated on the producer's thread.
    pub terrain_pending: bool,
    pub physics_lines: Vec<DebugLine>,
    pub scene_frame: Option<u64>,
    pub scene_range: Option<(u64, u64)>,
    pub program_revision: u64,
    pub timeline_generation: u64,
    pub caps: SimCaps,
}

/// The producer, run either on the calling thread or on its own sim thread.
pub(crate) struct SimDriver {
    mode: Mode,
    /// Frames requested so far; each snapshot carries its request's number.
    posted: u64,
    /// The newest snapshot handed to the shell.
    consumed: u64,
    /// Producer time since the last snapshot (inline; the sim thread keeps
    /// its own).
    busy_ns: u64,
    /// Time the shell blocked on the sim thread since the last
    /// [`Self::take_wait_ns`].
    wait_ns: u64,
}

enum Mode {
    Inline(Box<dyn Game>),
    Threaded(Worker),
}

struct Worker {
    jobs: Option<mpsc::Sender<Job>>,
    mailbox: Arc<Mailbox>,
    thread: Option<std::thread::JoinHandle<()>>,
}

enum Job {
    Call(Box<dyn FnOnce(&mut dyn Game) + Send>),
    Frame(FrameRequest, u64),
}

const SIM_EXITED: &str = "[sim] the producer thread exited";

impl SimDriver {
    /// Drive `game` on the calling thread.
    pub fn inline(game: Box<dyn Game>) -> Self {
        Self::with_mode(Mode::Inline(game))
    }

    /// Create the producer on a new sim thread and drive it there. The
    /// producer never leaves that thread, so it need not be `Send`.
    pub fn spawn(make_game: impl FnOnce() -> Box<dyn Game> + Send + 'static) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let mailbox = Arc::new(Mailbox::default());
        let sim_mailbox = mailbox.clone();
        let thread = std::thread::Builder::new()
            .name("functor-sim".to_string())
            // The interpreter recurses; match the main thread's stack so the
            // same programs run in both modes.
            .stack_size(8 << 20)
            .spawn(move || run_worker(make_game(), rx, &sim_mailbox))
            .expect("spawn the sim thread");
        Self::with_mode(Mode::Threaded(Worker {
            jobs: Some(jobs),
            mailbox,
            thread: Some(thread),
        }))
    }

    fn with_mode(mode: Mode) -> Self {
        Self {
            mode,
            posted: 0,
            consumed: 0,
            busy_ns: 0,
            wait_ns: 0,
        }
    }

    pub fn is_threaded(&self) -> bool {
        matches!(self.mode, Mode::Threaded(_))
    }

    /// Queue a call; it runs after everything queued before it.
    pub fn post(&mut self, f: impl FnOnce(&mut dyn Game) + Send + 'static) {
        match &mut self.mode {
            Mode::Inline(game) => {
                let started = Instant::now();
                f(&mut **game);
                self.busy_ns += elapsed_ns(started);
            }
            Mode::Threaded(worker) => worker.send(Job::Call(Box::new(f))),
        }
    }

    /// Run a call after everything queued before it and return its result.
    pub fn call<R: Send + 'static>(
        &mut self,
        f: impl FnOnce(&mut dyn Game) -> R + Send + 'static,
    ) -> R {
        match &mut self.mode {
            Mode::Inline(game) => {
                let started = Instant::now();
                let result = f(&mut **game);
                self.busy_ns += elapsed_ns(started);
                result
            }
            Mode::Threaded(worker) => {
                let (tx, rx) = mpsc::sync_channel(1);
                worker.send(Job::Call(Box::new(move |g| {
                    let _ = tx.send(f(g));
                })));
                let started = Instant::now();
                let result = rx.recv().expect(SIM_EXITED);
                self.wait_ns += elapsed_ns(started);
                result
            }
        }
    }

    pub fn caps(&mut self) -> SimCaps {
        self.call(|g| caps(g))
    }

    /// Request this frame's snapshot and return the newest one ready.
    ///
    /// Inline, that is this frame's. On the sim thread it is the previous
    /// frame's (the sim draws frame N while the shell renders N-1), or this
    /// frame's under `lockstep`. `None` means nothing new: the shell
    /// re-renders what it has, which happens once each time lockstep ends.
    pub fn frame(&mut self, req: FrameRequest, lockstep: bool) -> Option<SimSnapshot> {
        self.posted += 1;
        let seq = self.posted;
        let snapshot = match &mut self.mode {
            Mode::Inline(game) => {
                let started = Instant::now();
                let mut snapshot = snapshot(&mut **game, req);
                snapshot.seq = seq;
                snapshot.sim_ns = std::mem::take(&mut self.busy_ns) + elapsed_ns(started);
                Some(snapshot)
            }
            Mode::Threaded(worker) => {
                worker.send(Job::Frame(req, seq));
                let need = if lockstep { seq } else { (seq - 1).max(1) };
                let started = Instant::now();
                let snapshot = if need > self.consumed {
                    Some(worker.mailbox.take_at_least(need, worker.thread.as_ref()))
                } else {
                    worker.mailbox.try_take()
                };
                self.wait_ns += elapsed_ns(started);
                snapshot
            }
        };
        if let Some(snapshot) = &snapshot {
            self.consumed = snapshot.seq;
        }
        snapshot
    }

    /// The number of the frame last requested with [`Self::frame`].
    pub fn posted(&self) -> u64 {
        self.posted
    }

    pub fn take_wait_ns(&mut self) -> u64 {
        std::mem::take(&mut self.wait_ns)
    }

    pub fn key_event(&mut self, code: i32, is_down: bool) {
        self.post(move |g| g.key_event(code, is_down));
    }

    pub fn mouse_move(&mut self, x: i32, y: i32) {
        self.post(move |g| g.mouse_move(x, y));
    }

    pub fn mouse_wheel(&mut self, delta: i32) {
        self.post(move |g| g.mouse_wheel(delta));
    }

    pub fn mouse_button(&mut self, button: i32, is_down: bool) {
        self.post(move |g| g.mouse_button(button, is_down));
    }

    pub fn ui_event(&mut self, event: UiEvent) {
        self.post(move |g| g.ui_event(event));
    }

    pub fn webview_event(&mut self, event: UiEvent) {
        self.post(move |g| g.webview_event(event));
    }

    pub fn sampled_input(&mut self, snapshot: InputSnapshot) {
        self.post(move |g| g.sampled_input(&snapshot));
    }

    pub fn tick(&mut self, frame_time: FrameTime) {
        self.post(move |g| g.tick(frame_time));
    }

    /// Quit the producer once every queued call has run.
    pub fn quit(self) {
        match self.mode {
            Mode::Inline(mut game) => game.quit(),
            Mode::Threaded(mut worker) => {
                // Closing the queue ends the worker, which then quits.
                worker.jobs = None;
                if let Some(thread) = worker.thread.take() {
                    let _ = thread.join();
                }
            }
        }
    }
}

impl Worker {
    fn send(&self, job: Job) {
        self.jobs
            .as_ref()
            .expect(SIM_EXITED)
            .send(job)
            .expect(SIM_EXITED);
    }
}

/// The sim thread: run queued calls in order, publishing each frame's
/// snapshot to the mailbox, until the queue closes.
fn run_worker(mut game: Box<dyn Game>, jobs: mpsc::Receiver<Job>, mailbox: &Mailbox) {
    let mut busy_ns = 0;
    for job in jobs {
        let started = Instant::now();
        match job {
            Job::Call(f) => {
                f(&mut *game);
                busy_ns += elapsed_ns(started);
            }
            Job::Frame(req, seq) => {
                let mut snapshot = snapshot(&mut *game, req);
                snapshot.seq = seq;
                snapshot.sim_ns = std::mem::take(&mut busy_ns) + elapsed_ns(started);
                mailbox.put(snapshot);
            }
        }
    }
    game.quit();
}

/// The single-slot hand-off from the sim thread to the shell. Latest wins for
/// what is drawn; the effects of a snapshot the shell never took carry into
/// the newer one, so none are lost.
#[derive(Default)]
struct Mailbox {
    slot: Mutex<Option<SimSnapshot>>,
    ready: Condvar,
}

impl Mailbox {
    fn put(&self, mut snapshot: SimSnapshot) {
        let mut slot = self.slot.lock().unwrap();
        if let Some(older) = slot.take() {
            snapshot.absorb_older(older);
        }
        *slot = Some(snapshot);
        self.ready.notify_all();
    }

    fn try_take(&self) -> Option<SimSnapshot> {
        self.slot.lock().unwrap().take()
    }

    /// Wait for a snapshot numbered `seq` or newer.
    fn take_at_least(&self, seq: u64, thread: Option<&std::thread::JoinHandle<()>>) -> SimSnapshot {
        let mut slot = self.slot.lock().unwrap();
        loop {
            if slot.as_ref().is_some_and(|s| s.seq >= seq) {
                return slot.take().unwrap();
            }
            // A panicked producer never publishes again.
            if thread.is_none_or(|t| t.is_finished()) {
                panic!("{SIM_EXITED}");
            }
            slot = self
                .ready
                .wait_timeout(slot, std::time::Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }
}

impl SimSnapshot {
    /// Fold an older, never-taken snapshot's effects in ahead of ours.
    fn absorb_older(&mut self, mut older: SimSnapshot) {
        fn prepend<T>(newer: &mut Vec<T>, mut older: Vec<T>) {
            older.append(newer);
            *newer = older;
        }
        prepend(
            &mut self.audio_commands,
            std::mem::take(&mut older.audio_commands),
        );
        prepend(
            &mut self.net_commands,
            std::mem::take(&mut older.net_commands),
        );
        prepend(
            &mut self.conn_commands,
            std::mem::take(&mut older.conn_commands),
        );
        prepend(
            &mut self.preload_commands,
            std::mem::take(&mut older.preload_commands),
        );
        prepend(
            &mut self.terrain_requests,
            std::mem::take(&mut older.terrain_requests),
        );
        self.terrain_pending |= older.terrain_pending;
        self.sim_ns += older.sim_ns;
    }
}

fn elapsed_ns(started: Instant) -> u64 {
    started.elapsed().as_nanos() as u64
}

fn caps(game: &dyn Game) -> SimCaps {
    SimCaps {
        samples_input: game.samples_input(),
        uses_captured_mouse_input: game.uses_captured_mouse_input(),
    }
}

/// Parse a drained JSON command array; a malformed drain logs and drops.
fn parse_drain<T: serde::de::DeserializeOwned>(json: &str, what: &str) -> Vec<T> {
    if json == "[]" {
        return Vec::new();
    }
    serde_json::from_str(json).unwrap_or_else(|e| {
        eprintln!("[{what}] bad commands json: {e}");
        Vec::new()
    })
}

fn snapshot(game: &mut dyn Game, req: FrameRequest) -> SimSnapshot {
    let frame = Arc::new(game.render(req.time.clone()));
    let audio_scene = req.audio_scene.then(|| {
        serde_json::from_str::<AudioScene>(&game.audio_scene_json())
            .map_err(|e| eprintln!("[audio] bad scene json: {e}"))
            .ok()
    });
    let terrain_requests = functor_runtime_common::terrain::take_heightmap_requests();
    let terrain_pending = terrain_requests
        .iter()
        .any(|source| functor_runtime_common::terrain::hydrated_heightmap(source).is_none());
    SimSnapshot {
        seq: 0,
        sim_ns: 0,
        time: req.time,
        frame,
        ui: if req.ui { game.ui() } else { View::Empty },
        webview_html: req
            .webview
            .then(|| game.webview().map(|node| node.to_html()))
            .flatten(),
        audio_commands: parse_drain(&game.audio_drain_commands(), "audio"),
        audio_scene: audio_scene.flatten(),
        net_commands: parse_drain(&game.net_drain_commands(), "net"),
        conn_commands: if req.conn_commands {
            parse_drain(&game.net_drain_conn_commands(), "net")
        } else {
            Vec::new()
        },
        preload_commands: serde_json::from_str(&game.preload_drain_commands()).unwrap_or_default(),
        terrain_requests,
        terrain_pending,
        physics_lines: if req.physics_lines {
            functor_runtime_common::physics::with_world(
                functor_runtime_common::physics::DEFAULT_WORLD,
                |w| w.debug_lines(),
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        },
        scene_frame: game.current_scene_frame(),
        scene_range: game.scene_frame_range(),
        program_revision: game.scene_program_revision(),
        timeline_generation: game.scene_timeline_generation(),
        caps: caps(game),
    }
}
