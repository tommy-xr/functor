//! The shared MVU per-frame body for the Functor Lang producers (docs/time-travel.md
//! T6a). Both shells — the desktop `functor-runner` and the web/wasm runtime —
//! run the SAME game contract and per-frame orchestration; only I/O differs
//! (file-watch vs editor push, winit vs DOM input *delivery*, stderr vs the
//! browser console, native perf timing). Historically the frame body and its
//! private helpers were copy-pasted between the two `functor_lang_game.rs` producers;
//! this module owns them once so they can never drift.
//!
//! Two pieces:
//!
//! - [`Reporter`] — per-frame error reporting: dedupe a persistent message to
//!   one line (a 60fps loop must not flood the sink) and render error spans to
//!   `path:line:col`. The SINK (stderr / console) is a shell-supplied `fn`
//!   pointer and the source rendering is a [`SpanSource`], so this stays in
//!   shared code without pulling in `web_sys`.
//! - [`FrameCtx`] — a transient bundle of `&mut` borrows of the producer's
//!   per-frame state. Its methods ARE the frame body (`before_physics` /
//!   `physics_phase` / `record_frame`, or `run_frame` for all three) and the
//!   duplicated helpers (`absorb`, `pump_subscriptions`, `step_physics`,
//!   `deliver_*`). Each producer builds a `FrameCtx` from its owned fields and
//!   drives the phases; the leaf logic (`drain_effects`, `SceneRecorder`,
//!   `SteppedPhysics`) stays where it already lives.
//!
//! Input *delivery* is deliberately NOT unified (the shells queue/interleave it
//! differently); live input still funnels through [`FrameCtx::absorb`], so the
//! frame body begins at the scrub-commit and never absorbs live input itself.

use std::collections::HashSet;

use functor_lang::{line_col, project::SourceMap, RunError, Session, Span, Value};

use crate::asset::AssetProgress;
use crate::functor_lang_prelude::{
    asset_progress_value, assets_taggers, contains_effect, deliver_physics_events, drain_effects,
    frame_value, http_response_value, needs_update, net_conn_subs, net_event_value,
    perform_deferred_queries, physics_event_taggers, physics_scene_value, split_model_effect,
    sub_messages_for_frame, take_audio_completion, take_http_tagger, take_preload_completion,
    take_ui_handlers, DryRunEffects, EffectLog, EffectRunner, EffectTree, FunctorHost,
    NetEventKind, UiHandler,
};
use crate::input::RecordedInput;
use crate::net::{push_conn_command, ConnCommand, HttpResult};
use crate::physics::{self, PhysicsEvent, SteppedPhysics};
use crate::timetravel::SceneRecorder;
use crate::{Frame, FrameTime};

// ---------------------------------------------------------------------------
// The paused-inspector replay journal (visual-debugger PR2).
//
// During normal play the producer records nothing and renders no Display text —
// the hard perf requirement. Instead each model-updating entry-point call site
// pushes a cheap `(entry, Rc-cloned args, provenance tag)` into a thread-local
// journal. On pause the desktop producer replays each journaled call through
// `Session::call_recorded` (the PR1 recorder) to build the trace; entry points
// are pure functions of their args (the input model is `args[0]`), so replay is
// exact and — effects being plain data that only the drain performs — free of
// side effects.
//
// A THREAD-LOCAL sink (rather than a `FrameCtx` field) keeps this off the web
// producer entirely (it never arms it → the pushes are a `None` check) and off
// the shared effect machinery's signatures. The dry-run forward-step pauses it
// ([`JournalPause`]) so `--ghost` calls are never journaled.
// ---------------------------------------------------------------------------

/// One journaled model-updating call: the entry name, its Rc-cloned args, and a
/// lightweight provenance tag. The provenance STRING (the wire vocabulary) is
/// derived from the tag + args only at trace-build time — never during play, so
/// journaling renders no Display text.
#[derive(Clone)]
pub struct JournalEntry {
    /// The entry-point name — `tick` | `input` | `mouseMove` | `mouseWheel` |
    /// `update` (the wire contract's `entry`).
    pub entry: &'static str,
    /// The call's arguments (Rc-cloned). `args[0]` is the input model; for a
    /// message-bearing call `args[1]` is the message.
    pub args: Vec<Value>,
    pub provenance: Provenance,
}

/// Why a model-updating call ran — the source that drove it. Rendered to the
/// wire provenance string by [`Provenance::render`].
#[derive(Clone, Copy)]
pub enum Provenance {
    Tick,
    Input,
    MouseMove,
    MouseWheel,
    Subscription,
    EffectResult,
    PhysicsQuery,
    Collision,
    NetEvent,
    HttpResponse,
    AudioFinished,
    /// A `preloadThen` load settled (loaded or failed) — the shell's preload
    /// driver reported it.
    PreloadSettled,
    UiEvent,
    /// The frame's pure render pass — never journaled during play; the trace
    /// builder synthesizes one draw invocation against the frozen model.
    Draw,
}

impl Provenance {
    /// The wire provenance string, from the tag + the call's args. The
    /// message-bearing variants render `args[1]` (the message); `tick` renders
    /// its `dt` (`args[1]`); `input` reads the key name (`args[1]`, unquoted)
    /// and the down flag (`args[2]`). Built here at trace time, not during play.
    pub fn render(&self, args: &[Value]) -> String {
        let msg = || args.get(1).map(|v| v.to_string()).unwrap_or_default();
        match self {
            // `dt` is f32-sourced (the protocol's FrameTime), so render it at
            // f32 precision — the f64 Value would show cast noise (…0.2000000029).
            Provenance::Tick => {
                let dt = match args.get(1) {
                    Some(Value::Number(n)) => (*n as f32).to_string(),
                    _ => String::new(),
                };
                format!("tick dt={dt}")
            }
            Provenance::Input => {
                let key = match args.get(1) {
                    // The `Key.*` variant the producers deliver (displays as
                    // its canonical tag, `Key.W`).
                    Some(v @ Value::Variant { .. }) => v.to_string(),
                    _ => String::new(),
                };
                let dir = match args.get(2) {
                    Some(Value::Bool(false)) => "up",
                    _ => "down",
                };
                format!("input: {key} {dir}")
            }
            Provenance::MouseMove => "mouseMove".to_string(),
            Provenance::MouseWheel => "mouseWheel".to_string(),
            Provenance::Subscription => format!("subscription: {}", msg()),
            Provenance::EffectResult => format!("effect result: {}", msg()),
            Provenance::PhysicsQuery => format!("physics query: {}", msg()),
            Provenance::Collision => format!("collision: {}", msg()),
            Provenance::NetEvent => format!("net event: {}", msg()),
            Provenance::HttpResponse => format!("http response: {}", msg()),
            Provenance::AudioFinished => format!("audio finished: {}", msg()),
            Provenance::PreloadSettled => format!("preload settled: {}", msg()),
            Provenance::UiEvent => format!("ui event: {}", msg()),
            Provenance::Draw => "draw".to_string(),
        }
    }
}

thread_local! {
    /// The current frame's journal, `Some` only while the desktop producer is
    /// driving live frames (armed once in `FunctorLangGame::create`). `None`
    /// everywhere else — the web producer, the dry-run forward-step, tests —
    /// so [`journal_push`] is a cheap `None` check with zero allocation.
    static JOURNAL: std::cell::RefCell<Option<Vec<JournalEntry>>> =
        const { std::cell::RefCell::new(None) };
}

/// Push a journal entry if journaling is armed on this thread — otherwise a
/// no-op (one thread-local borrow). Called at every model-updating entry-point
/// call site; `args` is cloned (Rc-shared, cheap).
pub fn journal_push(entry: &'static str, args: &[Value], provenance: Provenance) {
    JOURNAL.with(|j| {
        if let Some(v) = j.borrow_mut().as_mut() {
            v.push(JournalEntry {
                entry,
                args: args.to_vec(),
                provenance,
            });
        }
    });
}

/// Arm journaling for this thread (idempotent). The desktop producer calls this
/// once at startup; the web producer never does, so its shared frame body pays
/// only the `None` check.
pub fn journal_arm() {
    JOURNAL.with(|j| {
        let mut b = j.borrow_mut();
        if b.is_none() {
            *b = Some(Vec::new());
        }
    });
}

/// Swap out the current frame's collected entries, leaving a fresh empty
/// journal armed — called at the end of each real frame (`tick`). `None` when
/// journaling isn't armed (web / tests).
pub fn journal_swap() -> Option<Vec<JournalEntry>> {
    JOURNAL.with(|j| {
        j.borrow_mut()
            .as_mut()
            .map(|v| std::mem::replace(v, Vec::new()))
    })
}

/// Disarm journaling on this thread, dropping any collected entries — for tests
/// that armed it (so a reused test thread starts clean).
pub fn journal_disarm() {
    JOURNAL.with(|j| *j.borrow_mut() = None);
}

/// RAII guard that PAUSES journaling for its lifetime: takes the current
/// journal out (leaving `None`, so nested pushes no-op) and restores it on
/// drop. Wraps the dry-run forward-step so `--ghost` forward-projection calls
/// never land in the journal, even on an unwind from stepped game code.
struct JournalPause(Option<Vec<JournalEntry>>);

impl JournalPause {
    fn enter() -> JournalPause {
        JournalPause(JOURNAL.with(|j| j.borrow_mut().take()))
    }
}

impl Drop for JournalPause {
    fn drop(&mut self) {
        JOURNAL.with(|j| *j.borrow_mut() = self.0.take());
    }
}

/// Where a producer resolves per-frame error spans to `path:line:col: message`.
/// The two shells hold their source differently (a multi-file project map on
/// desktop; the single fetched entry source on web), so this captures both.
pub enum SpanSource {
    /// Desktop: a whole-project source map (multi-file, project-wide spans).
    Project(SourceMap),
    /// Web: the single fetched entry source plus its label path.
    Single { src: String, path: String },
}

impl SpanSource {
    fn render(&self, span: Span, message: &str) -> String {
        match self {
            SpanSource::Project(map) => map.render(span.start, message),
            SpanSource::Single { src, path } => {
                let (line, col) = line_col(src, span.start);
                format!("{path}:{line}:{col}: {message}")
            }
        }
    }
}

/// Per-frame error reporting, shared by both producers. Dedupes a persistent
/// message to one line and renders `RunError` spans to source positions. The
/// output SINK differs per shell (stderr vs the browser console) — supplied as
/// a plain `fn` pointer at construction — as does the source rendering
/// ([`SpanSource`]).
pub struct Reporter {
    last_error: Option<String>,
    source: SpanSource,
    emit: fn(&str),
}

impl Reporter {
    pub fn new(source: SpanSource, emit: fn(&str)) -> Reporter {
        Reporter {
            last_error: None,
            source,
            emit,
        }
    }

    /// Swap the source rendered against — a hot reload / push replaced it.
    pub fn set_source(&mut self, source: SpanSource) {
        self.source = source;
    }

    /// Clear the dedupe slot: a reload starts a fresh error stream.
    pub fn reset(&mut self) {
        self.last_error = None;
    }

    fn last_error(&self) -> Option<String> {
        self.last_error.clone()
    }

    /// Print a message once per distinct string — a 60fps loop must not flood
    /// the sink with one persistent bug.
    pub fn report_once(&mut self, message: String) {
        if self.last_error.as_deref() != Some(message.as_str()) {
            (self.emit)(&message);
            self.last_error = Some(message);
        }
    }

    /// Render + report a per-frame `RunError` at its source position (deduped).
    /// The span identifies the file too (project-wide spans), so an error in a
    /// sibling module names that module's file.
    pub fn frame_error(&mut self, stage: &str, err: &RunError) {
        self.report_once(self.render_frame_error(stage, err));
    }

    /// Render a per-frame `RunError` to its `[functor-lang] {stage} error at
    /// path:line:col: message` form WITHOUT reporting it — for a caller (the web
    /// error overlay) that also surfaces the text elsewhere, so it can report
    /// once and display the same string without re-deriving the position.
    pub fn render_frame_error(&self, stage: &str, err: &RunError) -> String {
        format!(
            "[functor-lang] {stage} error at {}",
            self.source.render(err.span, &err.message)
        )
    }
}

/// A transient bundle of `&mut` borrows of a producer's per-frame state. The
/// shared MVU frame body and its helpers run through it, so both shells execute
/// byte-identical game logic. Built cheaply (all borrows) whenever a producer
/// needs to run frame work, and dropped at the end of the call.
pub struct FrameCtx<'a> {
    pub session: &'a Session,
    pub model: &'a mut Value,
    pub physics_rt: &'a mut SteppedPhysics,
    /// The physics world's current fixed frame (what the coupled scene
    /// recorder stores per rendered frame).
    pub physics_frame: &'a mut u64,
    pub recorder: &'a mut SceneRecorder,
    pub effect_runner: &'a mut dyn EffectRunner,
    pub effect_log: &'a mut EffectLog,
    pub deferred_queries: &'a mut Vec<EffectTree>,
    pub pending_events: &'a mut Vec<PhysicsEvent>,
    pub live_conn_keys: &'a mut HashSet<String>,
    pub prev_tts: &'a mut Option<f64>,
    /// This frame's buffered input events (docs/time-travel.md T6b): each shell
    /// appends to it in `key_event`/`mouse_move`/`mouse_wheel`, and
    /// [`Self::record_frame`] flushes a played frame's worth into the recorder's
    /// input log (or drains-and-drops it on a paused frame). The dry-run
    /// forward-step points this at a throwaway buffer.
    pub input_buf: &'a mut Vec<RecordedInput>,
    pub has_physics: bool,
    pub has_subscriptions: bool,
    /// The shell's current asset-loading snapshot (`AssetCache::progress()`),
    /// `None` when the driver doesn't track assets (dry-run forward-step,
    /// headless tests). Compared against `delivered_asset_progress` each frame;
    /// a change fires the `Sub.assets` taggers through `update`.
    pub asset_progress: Option<AssetProgress>,
    /// The snapshot the game last saw (shell-owned so it survives frames; only
    /// advanced when a `Sub.assets` tagger actually received it, so a game
    /// that ADDS the sub later — hot reload — still gets the current state).
    pub delivered_asset_progress: &'a mut Option<AssetProgress>,
    /// Suppress OUTBOUND effects (physics command / timeline / send / http /
    /// audio) during a dry-run forward-step (docs/time-travel.md T6b): they
    /// still log and the model still evolves, but nothing escapes to the live
    /// world or the global queues. Always `false` on the live frame body.
    pub suppress_outbound: bool,
    pub reporter: &'a mut Reporter,
}

impl FrameCtx<'_> {
    /// One full MVU frame: scrub-commit → subscriptions → tick+absorb → physics
    /// (step + query drain + events) → record. Used by shells that don't split
    /// the frame for perf timing (web). Desktop calls the three phases directly
    /// so it can time `tick` and `physics` separately.
    pub fn run_frame(&mut self, frame_time: FrameTime) {
        self.before_physics(frame_time);
        self.physics_phase(frame_time);
        self.record_frame(frame_time);
    }

    /// The pre-physics phase: commit a resuming scrub, pump subscriptions, then
    /// run `tick` and absorb its result. (Desktop times this as `tick_ns`.)
    pub fn before_physics(&mut self, frame_time: FrameTime) {
        // Committing a scrub (docs/time-travel.md T3): if play resumes (dts > 0)
        // while the draggable bar is parked on an earlier frame, branch the
        // timeline from there BEFORE this frame advances — and drop any in-flight
        // frame work so it doesn't cross the branch (the reload discipline).
        if frame_time.dts > 0.0
            && self.recorder.commit_scrub_if_resuming(
                self.model,
                self.physics_rt,
                self.physics_frame,
                self.has_physics,
            )
        {
            self.deferred_queries.clear();
            self.pending_events.clear();
            // The restored model predates the current loading snapshot, so
            // "what the game last saw" no longer holds — invalidate the
            // marker so the branch's first frame redelivers current progress
            // (a rewound loading screen must learn its assets already
            // settled).
            *self.delivered_asset_progress = None;
        }
        self.subscriptions_and_tick(frame_time);
    }

    /// Pump subscriptions then run `tick` and absorb its result — the pure body
    /// of the pre-physics phase, shared by the live frame (`before_physics`,
    /// after the scrub-commit) and the dry-run forward-step
    /// ([`Self::step_scene_forward`], which must NOT scrub-commit). Subscriptions
    /// run first, so `tick` sees a model that has absorbed this frame's messages
    /// (the F# executor's ordering).
    fn subscriptions_and_tick(&mut self, frame_time: FrameTime) {
        self.pump_subscriptions(frame_time.tts as f64);
        let args = vec![
            self.model.clone(),
            Value::Number(frame_time.dts as f64),
            Value::Number(frame_time.tts as f64),
        ];
        journal_push("tick", &args, Provenance::Tick);
        match self.session.call("tick", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error("tick", &err),
        }
    }

    /// The physics phase: reconcile + fixed-step the world, then drain deferred
    /// queries and deliver collision events against the just-stepped world.
    /// (Desktop times this as `physics_ns`.)
    pub fn physics_phase(&mut self, frame_time: FrameTime) {
        let physics_steps = self.step_physics(frame_time.dts as f32);
        // Post-step query drain: deferred raycasts answer against the world
        // just stepped; their messages fold through `update` before `draw`,
        // so this frame's render already reflects them.
        // On a ZERO-substep frame (the accumulator short of FIXED_DT — normal
        // right after load and at >60fps) queries stay deferred, like pending
        // commands, so they never answer against a world that hasn't
        // simulated. Games without a physics hook answer immediately (the
        // lazily-created empty world gives sane misses).
        // A query answers once the world has EVER stepped: normally this
        // frame's steps, but also while PAUSED (frozen mid-flight, frame > 0)
        // and on a short zero-substep frame — so a raycast fired while paused
        // answers against the frozen world instead of deferring forever.
        let world_ready = physics_steps > 0 || !self.has_physics || *self.physics_frame > 0;
        if world_ready && !self.deferred_queries.is_empty() {
            let deferred = std::mem::take(self.deferred_queries);
            let mut reports: Vec<String> = Vec::new();
            perform_deferred_queries(
                self.session,
                self.model,
                deferred,
                self.effect_runner,
                self.effect_log,
                &mut |message| reports.push(message),
                self.suppress_outbound,
            );
            for message in reports {
                self.reporter.report_once(message);
            }
        }
        // Collision events (docs/physics.md Phase 5): this frame's contact
        // transitions, delivered to the `Physics.events` taggers of the
        // CURRENT model's subscriptions — post-step, alongside query answers.
        let events = std::mem::take(self.pending_events);
        if !events.is_empty() && self.has_subscriptions {
            match self
                .session
                .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(subs) => match physics_event_taggers(&subs) {
                    Ok(taggers) if !taggers.is_empty() => {
                        let mut reports: Vec<String> = Vec::new();
                        deliver_physics_events(
                            self.session,
                            self.model,
                            &taggers,
                            &events,
                            self.effect_runner,
                            self.effect_log,
                            &mut |message| reports.push(message),
                            self.suppress_outbound,
                        );
                        for message in reports {
                            self.reporter.report_once(message);
                        }
                    }
                    Ok(_) => {}
                    Err(message) => self
                        .reporter
                        .report_once(format!("[functor-lang] {message}")),
                },
                Err(err) => self.reporter.frame_error("subscriptions", &err),
            }
        }
    }

    /// Record the settled model of this rendered frame (docs/time-travel.md T1)
    /// plus the physics fixed-frame the world reached, in lockstep, so a coupled
    /// rewind can restore both.
    ///
    /// Skip a PAUSED frame (`dts == 0`, i.e. the clock pinned): the sim hasn't
    /// advanced, so recording would only pile up frozen duplicates — inflating
    /// the timeline and pushing a rewind target past the real history.
    /// `dts == 0` is exactly the pinned-and-not-stepping case (a one-shot step
    /// carries `dts > 0`).
    pub fn record_frame(&mut self, frame_time: FrameTime) {
        if frame_time.dts > 0.0 {
            // Record this frame's inputs FIRST (keyed at the recorder's current
            // `rendered_frame`), then the model — `record` advances the clock, so
            // both must land on the same frame (docs/time-travel.md T6b).
            self.recorder.record_inputs(std::mem::take(self.input_buf));
            self.recorder
                .record_timed(self.model, *self.physics_frame, frame_time);
        } else {
            // The shell runs one zero-delta bootstrap before frame zero so the
            // model/physics hooks settle before first draw. It is not a visible
            // timeline frame, but it can mutate a valid model and must therefore
            // prefix an exact reconstruction. Later paused zero-delta renders
            // are not simulation history and are discarded by the recorder.
            self.recorder.record_replay_prefix(
                frame_time,
                std::mem::take(self.input_buf),
            );
        }
    }

    /// Deterministically step the whole scene forward from the CURRENT ctx
    /// state, snapshotting the stepped `(model, world-snapshot)` at each of
    /// `divisions` division boundaries — the headless forward-step that feeds
    /// forward-ghosting (docs/time-travel.md T6b). To keep velocity-integrated
    /// motion (e.g. `examples/mario`'s Euler-integrated jump) FAITHFUL, the sim
    /// is advanced at a FINE `sub_dt` (`steps_per_division` sub-ticks per
    /// snapshot) but sampled only at the boundaries: `divisions` snapshots,
    /// `divisions * steps_per_division` fine ticks total, over a window of
    /// `divisions * steps_per_division * sub_dt`. It runs the frame body MINUS
    /// the scrub-commit (starts at `subscriptions_and_tick`, never
    /// `before_physics`, so it can't branch the throwaway recorder) and MINUS
    /// `record_frame` (nothing is committed to live history).
    ///
    /// This ctx MUST be a DRY-RUN one (see [`forward_step_scene`]): a cloned
    /// model, `suppress_outbound = true` (no effect escapes to the live world /
    /// global queues), a deterministic runner, and `physics_rt` pointed at a
    /// throwaway world — so the live producer state stays untouched.
    ///
    /// The forward-step computes its OWN fine-step time (it does NOT read the
    /// shell `GameClock`): fine step `s` (the running counter across all
    /// divisions) runs `FrameTime { dts: sub_dt, tts = start_tts +
    /// (s+1)*sub_dt }`. A division boundary lands after `steps_per_division`
    /// such steps, so division `div`'s snapshot has `tts = start_tts +
    /// (div+1)*steps_per_division*sub_dt`.
    ///
    /// The determinism boundary — where the projected model diverges from a
    /// live continuation (the physics WORLD snapshot is always exact):
    /// wall-clock `Now` / unseeded `Random` reads (the runner is deterministic
    /// here, so a game reading real time/entropy in the frame body won't
    /// match); a `tts`-driven / seeded game DOES match, since `tts` is
    /// supplied and the runner is deterministic. Physics is NOT a boundary:
    /// the caller scopes the throwaway world active
    /// ([`physics::ActiveWorldScope`]), so readback in the frame body
    /// (`Physics.position` / `Physics.transformed` / `Physics.raycast`)
    /// answers against the projected world and physics COMMANDS
    /// (`applyImpulse` / `teleport` / …) apply to it — a replayed kick kicks
    /// the ghost.
    ///
    /// `inputs` is the recorded input log, now indexed per FINE step (not per
    /// division): index `s` = fine step `s`'s events, i.e. fork frame
    /// `K + 1 + s` (each rendered frame is one fine step at `sub_dt = 1/60`). At
    /// the TOP of each fine step its recorded events are REPLAYED (mirroring the
    /// live order — input arrives before `tick`) so a recorded jump replays.
    /// Beyond the recorded window (`inputs` runs out) the step COASTS on held
    /// state.
    fn step_scene_forward<'a>(
        &mut self,
        divisions: usize,
        steps_per_division: usize,
        input_at: impl Fn(usize) -> Option<&'a [RecordedInput]>,
        mut frame_time_at: impl FnMut(usize) -> FrameTime,
        capture_from_division: usize,
    ) -> Vec<(Value, Option<Vec<u8>>)> {
        let mut out = Vec::with_capacity(divisions.saturating_sub(capture_from_division));
        let mut step = 0usize;
        for div in 0..divisions {
            for _ in 0..steps_per_division {
                // Replay this fine step's recorded inputs before the frame body,
                // so the model absorbs them exactly as the live frame did (coast
                // when the log has no entry for this step).
                if let Some(events) = input_at(step) {
                    // A recorded UiEvent resolved against the LAST RENDER's
                    // handler table live — the tree the user saw, built from
                    // the settled model BEFORE this step's inputs. Rebuild
                    // that table once here, at the step top, so a step with
                    // several inputs can't resolve later UiEvents against a
                    // tree an earlier input already re-shaped (xreview).
                    let ui_handlers = events
                        .iter()
                        .any(|e| matches!(e, RecordedInput::UiEvent(_)))
                        .then(|| self.eval_handler_table("ui"))
                        .unwrap_or_default();
                    // Webview events carry their OWN table: same step-top
                    // rebuild, from `webview(model)` instead of `ui(model)`.
                    let webview_handlers = events
                        .iter()
                        .any(|e| matches!(e, RecordedInput::WebviewEvent(_)))
                        .then(|| self.eval_handler_table("webview"))
                        .unwrap_or_default();
                    for event in events {
                        self.replay_input(event.clone(), &ui_handlers, &webview_handlers);
                    }
                }
                let frame_time = frame_time_at(step);
                self.subscriptions_and_tick(frame_time);
                self.physics_phase(frame_time);
                step += 1;
            }
            // Snapshot only at the division boundary — the strobe still has
            // `divisions` frames, but each is the result of accurate fine
            // integration over `steps_per_division` sub-ticks.
            if div >= capture_from_division {
                let world = if self.has_physics {
                    self.physics_rt.snapshot_world()
                } else {
                    None
                };
                out.push((self.model.clone(), world));
            }
        }
        out
    }

    /// Evaluate `entry(model)` (`"ui"` or `"webview"`) for its HANDLER TABLE
    /// only (the view/tree is discarded) — how the replay path reconstructs
    /// the table a recorded frame's events resolved against. A failed
    /// evaluation reports (silenced under the dry-run reporter) and yields an
    /// empty table, so the events drop as unknown slots.
    fn eval_handler_table(&mut self, entry: &'static str) -> Vec<UiHandler> {
        match self
            .session
            .call(entry, vec![self.model.clone()], &mut FunctorHost)
        {
            Ok(_) => take_ui_handlers(),
            Err(err) => {
                // A failed evaluation must not leak a partial table.
                let _ = take_ui_handlers();
                self.reporter.frame_error(entry, &err);
                Vec::new()
            }
        }
    }

    /// Replay one recorded input event during the forward-step, mirroring the
    /// LIVE path (`key_event`/`mouse_move`/`mouse_wheel`): call the game's
    /// `input`/`mouseMove`/`mouseWheel` entry point with the SAME reconstructed
    /// args, then [`Self::absorb`] the result (which honors `suppress_outbound`,
    /// so nothing escapes). A `Key` re-runs `Key::from_i32` on the raw code just
    /// as the live path does; an unknown code is dropped, like live. An entry
    /// point removed by the edited program is ignored, exactly as the live
    /// shell would ignore that raw event. A `UiEvent` resolves against
    /// `ui_handlers` and a `WebviewEvent` against `webview_handlers` — the
    /// step-top tables the caller rebuilt (the live frame's last-render
    /// contract).
    fn replay_input(
        &mut self,
        event: RecordedInput,
        ui_handlers: &[UiHandler],
        webview_handlers: &[UiHandler],
    ) {
        let (entry, args) = match event {
            RecordedInput::Key { code, is_down } => {
                let Some(key_value) = crate::key_input_value(code) else {
                    return; // unrecognized code / Key::Unknown — dropped, like live.
                };
                (
                    "input",
                    vec![self.model.clone(), key_value, Value::Bool(is_down)],
                )
            }
            RecordedInput::MouseMove { x, y } => (
                "mouseMove",
                vec![
                    self.model.clone(),
                    Value::Number(x as f64),
                    Value::Number(y as f64),
                ],
            ),
            RecordedInput::MouseWheel { delta } => (
                "mouseWheel",
                vec![self.model.clone(), Value::Number(delta as f64)],
            ),
            RecordedInput::UiEvent(event) => {
                self.deliver_ui_event(ui_handlers, &event);
                return;
            }
            RecordedInput::WebviewEvent(event) => {
                self.deliver_ui_event(webview_handlers, &event);
                return;
            }
        };
        if self.session.global(entry).is_none() {
            return;
        }
        match self.session.call(entry, args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error(entry, &err),
        }
    }

    /// Take an entry point's return: split off any `(model, effect)` pair,
    /// adopt the model, and drain the effects to a fixed point through `update`
    /// (docs/functor-lang.md B6). Every producer path that runs game code funnels through
    /// here, so effects work uniformly from tick, input, mouse, and messages.
    pub fn absorb(&mut self, returned: Value) {
        let (model, effects) = split_model_effect(returned);
        *self.model = model;
        // Effects are commands, not data — one stored in the model would make
        // the pair sniff ambiguous on a later return (see `split_model_effect`).
        if contains_effect(self.model) {
            self.reporter.report_once(
                "[functor-lang] the model contains an Effect value — Effects are commands, \
not data; return them beside the model as `(model, effect)` instead of storing them"
                    .to_string(),
            );
        }
        let Some(effects) = effects else { return };
        // Only MESSAGE-producing effects need an `update` to receive them —
        // tagger-less physics commands must not be dropped over a missing hook.
        if needs_update(&effects) && self.session.global("update").is_none() {
            self.reporter.report_once(
                "[functor-lang] effects returned but there is no `let update = (model, msg) => …` \
to receive their messages; dropping them"
                    .to_string(),
            );
            return;
        }
        let mut reports: Vec<String> = Vec::new();
        let deferred = drain_effects(
            self.session,
            self.model,
            effects,
            self.effect_runner,
            self.effect_log,
            &mut |message| reports.push(message),
            self.suppress_outbound,
        );
        // Physics queries wait for the post-step drain (end of the frame), so
        // their taggers answer against THIS frame's stepped world.
        self.deferred_queries.extend(deferred);
        for message in reports {
            self.reporter.report_once(message);
        }
    }

    /// Fire subscription timers over `(prev_tts, tts]` and fold their messages
    /// through `update`, before this frame's `tick` — the message drain seam
    /// (docs/functor-lang.md C4b-2). Subscriptions are recomputed from the current model
    /// each frame, so a model change can silence a timer. Errors report per
    /// message and processing continues.
    fn pump_subscriptions(&mut self, tts: f64) {
        // Advance the window even without subscriptions (or on frame one), so a
        // hot reload that ADDS subscriptions starts from a sane edge.
        let prev = self.prev_tts.replace(tts);
        if !self.has_subscriptions {
            // No subscriptions must not leave a previous program's connections
            // open (a hot reload that dropped them).
            if !self.live_conn_keys.is_empty() {
                self.close_all_connections();
            }
            return;
        }
        let subs =
            match self
                .session
                .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(subs) => subs,
                Err(err) => return self.reporter.frame_error("subscriptions", &err),
            };
        // Reconcile connections EVERY frame — including frame one (before the
        // timer window exists), so a declared connection opens immediately.
        self.reconcile_connections(&subs);
        // Asset progress also delivers on frame one: a loading screen wants
        // the initial snapshot, not the first change after it.
        self.deliver_asset_progress(&subs);
        let Some(prev) = prev else {
            return;
        };
        let msgs = match sub_messages_for_frame(&subs, prev, tts) {
            Ok(msgs) => msgs,
            Err(message) => {
                return self
                    .reporter
                    .report_once(format!("[functor-lang] {message}"))
            }
        };
        for msg in msgs {
            let args = vec![self.model.clone(), msg];
            journal_push("update", &args, Provenance::Subscription);
            match self.session.call("update", args, &mut FunctorHost) {
                Ok(returned) => self.absorb(returned),
                Err(err) => self.reporter.frame_error("update", &err),
            }
        }
    }

    /// Fire the `Sub.assets` taggers when the shell's loading snapshot changed
    /// since the game last saw it, folding `tagger({loaded, total, failed})`
    /// through `update` — the loading-screen seam. No taggers subscribed
    /// leaves `delivered` unmarked, so a game that adds `Sub.assets` later
    /// (hot reload) still receives the current state on its first frame.
    fn deliver_asset_progress(&mut self, subs: &Value) {
        let Some(progress) = self.asset_progress.clone() else {
            return;
        };
        if self.delivered_asset_progress.as_ref() == Some(&progress) {
            return;
        }
        let taggers = match assets_taggers(subs) {
            Ok(taggers) => taggers,
            Err(message) => {
                return self
                    .reporter
                    .report_once(format!("[functor-lang] {message}"))
            }
        };
        if taggers.is_empty() {
            return;
        }
        let record = asset_progress_value(&progress);
        let mut any_delivered = false;
        for tagger in taggers {
            let msg = match self.session.apply(
                tagger,
                vec![record.clone()],
                "Sub.assets tagger",
                &mut FunctorHost,
            ) {
                Ok(msg) => msg,
                Err(err) => {
                    self.reporter.frame_error("Sub.assets tagger", &err);
                    continue;
                }
            };
            any_delivered = true;
            let args = vec![self.model.clone(), msg];
            journal_push("update", &args, Provenance::Subscription);
            match self.session.call("update", args, &mut FunctorHost) {
                Ok(returned) => self.absorb(returned),
                Err(err) => self.reporter.frame_error("update", &err),
            }
        }
        // Only mark delivered when a tagger actually received it — an
        // erroring tagger (usually a transient hot-reload state) must not
        // permanently swallow the snapshot.
        if any_delivered {
            *self.delivered_asset_progress = Some(progress);
        }
    }

    /// The frame's physics phase (docs/physics.md): ask the game what bodies
    /// should exist, reconcile the singleton world to match, and advance it in
    /// fixed substeps. Runs after `tick` so declarations come from the settled
    /// model, and before `render` so `Physics.position`/`Physics.transformed`
    /// in `draw` read the just-stepped world. Returns the number of fixed
    /// substeps taken (0 when there is no `physics` hook, the hook errored, or
    /// the accumulator hasn't reached a full step yet).
    fn step_physics(&mut self, dts: f32) -> u32 {
        if !self.has_physics {
            return 0;
        }
        let args = vec![self.model.clone()];
        match self.session.call("physics", args, &mut FunctorHost) {
            Ok(value) => match physics_scene_value(&value) {
                Some(scene) => {
                    // The recorded drive (Phase 6): every fixed frame goes
                    // through the Timeline, so pause/rewind/replay work.
                    let advanced = self.physics_rt.advance(scene, dts);
                    *self.pending_events = advanced.events;
                    *self.physics_frame = advanced.frame;
                    let steps = advanced.steps;
                    let warnings = advanced.warnings;
                    // Command effects apply asynchronously (queued at perform
                    // time, applied at the step), so their problems — unknown
                    // tag, queue overflow — surface here, deduped.
                    for warning in warnings {
                        self.reporter
                            .report_once(format!("[functor-lang] {warning}"));
                    }
                    return steps;
                }
                None => self.reporter.report_once(format!(
                    "[functor-lang] physics must return Physics.scene(gravity, [body, …]), got {}",
                    value.kind_name()
                )),
            },
            Err(err) => self.reporter.frame_error("physics", &err),
        }
        0
    }

    /// Close every connection this producer still has open (a reload that
    /// dropped `subscriptions`, or shutdown). CloseKey is queued for each; the
    /// live set is cleared.
    pub fn close_all_connections(&mut self) {
        for key in std::mem::take(self.live_conn_keys) {
            push_conn_command(ConnCommand::CloseKey { key });
        }
    }

    /// Open connections newly declared this frame and close ones no longer
    /// declared (keyed by endpoint). Commands go to the shell's connection
    /// manager, drained via `net_drain_conn_commands`. The physics-events
    /// pattern for connections.
    fn reconcile_connections(&mut self, subs: &Value) {
        // A dry-run forward-step (docs/time-travel.md T6b) must not open/close
        // live sockets: connection reconcile is purely OUTBOUND (it feeds
        // nothing back into the model), so suppress it wholesale — the same
        // rule as the six drain arms, keeping the global queues untouched.
        if self.suppress_outbound {
            return;
        }
        let conns = match net_conn_subs(subs) {
            Ok(conns) => conns,
            Err(message) => {
                return self
                    .reporter
                    .report_once(format!("[functor-lang] {message}"))
            }
        };
        // Dedupe by key (first declaration wins its listen/connect role) so a
        // key is opened at most once even if declared twice in one frame.
        let mut declared: HashSet<String> = HashSet::new();
        for conn in &conns {
            if !declared.insert(conn.key.clone()) {
                continue; // already handled this key this frame
            }
            if !self.live_conn_keys.contains(&conn.key) {
                push_conn_command(if conn.listen {
                    ConnCommand::Listen {
                        key: conn.key.clone(),
                        addr: conn.key.clone(),
                    }
                } else {
                    ConnCommand::Connect {
                        key: conn.key.clone(),
                        url: conn.key.clone(),
                    }
                });
            }
        }
        for key in self.live_conn_keys.iter() {
            if !declared.contains(key) {
                push_conn_command(ConnCommand::CloseKey { key: key.clone() });
            }
        }
        *self.live_conn_keys = declared;
    }

    /// Route one inbound connection event to the matching key's tagger and fold
    /// the message through `update`. Taggers are read FRESH from the current
    /// `subscriptions(model)` (never cached — a reload rebinds them).
    pub fn deliver_net_event(&mut self, key: String, kind: NetEventKind, conn: i32, text: String) {
        if !self.has_subscriptions {
            return;
        }
        let subs =
            match self
                .session
                .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
            {
                Ok(subs) => subs,
                Err(err) => return self.reporter.frame_error("subscriptions", &err),
            };
        let conns = match net_conn_subs(&subs) {
            Ok(conns) => conns,
            Err(message) => {
                return self
                    .reporter
                    .report_once(format!("[functor-lang] {message}"))
            }
        };
        let Some(sub) = conns.into_iter().find(|c| c.key == key) else {
            return; // an event for a no-longer-declared connection: drop it
        };
        let value = net_event_value(kind, conn as u64, &text).to_functor_lang();
        let msg = match self
            .session
            .apply(sub.tagger, vec![value], "net event", &mut FunctorHost)
        {
            Ok(msg) => msg,
            Err(err) => return self.reporter.frame_error("net event", &err),
        };
        let args = vec![self.model.clone(), msg];
        journal_push("update", &args, Provenance::NetEvent);
        match self.session.call("update", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error("update", &err),
        }
    }

    /// Route a completed HTTP request to the tagger registered when the request
    /// fired (frames ago), folding the resulting message through `update`. An
    /// orphaned token — a hot reload dropped its tagger while the request was in
    /// flight — is silently ignored.
    pub fn deliver_http_result(&mut self, result: HttpResult) {
        let Some(tagger) = take_http_tagger(result.token) else {
            return;
        };
        let value = http_response_value(&result);
        let msg = match self
            .session
            .apply(tagger, vec![value], "http response", &mut FunctorHost)
        {
            Ok(msg) => msg,
            Err(err) => return self.reporter.frame_error("http response", &err),
        };
        let args = vec![self.model.clone(), msg];
        journal_push("update", &args, Provenance::HttpResponse);
        match self.session.call("update", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error("update", &err),
        }
    }

    /// Route an interaction the shell detected on an interactive UI widget to
    /// its handler and fold the resulting message through `update`
    /// (docs/ui-interaction.md U2). `handlers` is the table the producer kept
    /// from the `ui(model)` evaluation that built the tree the shell rendered
    /// — slots resolve against exactly what the user saw. A verbatim-msg
    /// handler (a button) ignores the event's payload; a tagger handler is
    /// applied to it (a slider's new value, a text input's new text).
    pub fn deliver_ui_event(&mut self, handlers: &[UiHandler], event: &crate::ui::UiEvent) {
        use crate::ui::UiEventKind;
        // Check the receiver exists before doing any work (the `absorb` rule)
        // — a game with widgets but no `update` gets the teaching error, not
        // a wasted tagger application.
        if self.session.global("update").is_none() {
            return self.reporter.report_once(
                "[functor-lang] a ui widget produced a message but there is no \
`let update = (model, msg) => …` to receive it; dropping it"
                    .to_string(),
            );
        }
        let Some(handler) = handlers.get(event.slot as usize) else {
            return self.reporter.report_once(format!(
                "[functor-lang] ui event for unknown widget slot {} (the view registered {}) — dropped",
                event.slot,
                handlers.len()
            ));
        };
        let msg = match (handler, &event.kind) {
            (UiHandler::Msg(msg), _) => msg.clone(),
            (UiHandler::Tagger(tagger), UiEventKind::SliderChanged(value)) => {
                match self.session.apply(
                    tagger.clone(),
                    vec![Value::Number(*value)],
                    "ui event tagger",
                    &mut FunctorHost,
                ) {
                    Ok(msg) => msg,
                    Err(err) => return self.reporter.frame_error("ui event tagger", &err),
                }
            }
            (UiHandler::Tagger(tagger), UiEventKind::TextChanged(text)) => {
                match self.session.apply(
                    tagger.clone(),
                    vec![Value::String(std::rc::Rc::from(text.as_str()))],
                    "ui event tagger",
                    &mut FunctorHost,
                ) {
                    Ok(msg) => msg,
                    Err(err) => return self.reporter.frame_error("ui event tagger", &err),
                }
            }
            (UiHandler::Tagger(_), UiEventKind::Clicked) => {
                return self.reporter.report_once(format!(
                    "[functor-lang] ui click for slot {} reached a tagger handler — a click \
carries no payload to tag; dropped",
                    event.slot
                ));
            }
        };
        let args = vec![self.model.clone(), msg];
        journal_push("update", &args, Provenance::UiEvent);
        match self.session.call("update", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error("update", &err),
        }
    }

    /// Route a finished `playThen` one-shot to the completion MESSAGE registered
    /// when it fired (frames ago), folding it through `update`. Unlike
    /// `deliver_http_result` there is no tagger: the message is delivered
    /// verbatim. An orphaned token — a reload dropped its message mid-flight —
    /// is silently ignored.
    pub fn deliver_audio_completion(&mut self, token: u64) {
        let Some(message) = take_audio_completion(token) else {
            return;
        };
        let args = vec![self.model.clone(), message];
        journal_push("update", &args, Provenance::AudioFinished);
        match self.session.call("update", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error("update", &err),
        }
    }

    /// Route a SETTLED `preloadThen` (loaded or failed) to the completion
    /// MESSAGE registered when it fired — the audio-completion shape:
    /// delivered verbatim, orphaned tokens (a reload dropped the message
    /// mid-flight) silently ignored.
    pub fn deliver_preload_completion(&mut self, token: u64) {
        let Some(message) = take_preload_completion(token) else {
            return;
        };
        let args = vec![self.model.clone(), message];
        journal_push("update", &args, Provenance::PreloadSettled);
        match self.session.call("update", args, &mut FunctorHost) {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error("update", &err),
        }
    }
}

/// A silencing error sink for the dry-run forward-step's [`Reporter`]: its
/// per-frame errors are throwaway (the live frame already reports them), so
/// they go nowhere.
fn silent_emit(_: &str) {}

enum ForwardInputs<'a> {
    Dense(&'a [Vec<RecordedInput>]),
    Recorder(&'a SceneRecorder),
}

impl ForwardInputs<'_> {
    fn at(&self, step: usize) -> Option<&[RecordedInput]> {
        match self {
            ForwardInputs::Dense(inputs) => inputs.get(step).map(Vec::as_slice),
            ForwardInputs::Recorder(recorder) => Some(recorder.replay_inputs_at_step(step)),
        }
    }
}

enum ForwardClock<'a> {
    Fixed { tts: f32, dts: f32 },
    Recorder(&'a SceneRecorder),
}

impl ForwardClock<'_> {
    fn next(&mut self, step: usize) -> FrameTime {
        match self {
            ForwardClock::Fixed { tts, dts } => {
                *tts += *dts;
                FrameTime { tts: *tts, dts: *dts }
            }
            ForwardClock::Recorder(recorder) => recorder
                .replay_frame_time_at_step(step)
                .expect("counterfactual replay clock range was validated"),
        }
    }
}

/// RAII guard for a throwaway dry-run physics world: removes it from the global
/// registry on drop, so a panic in the stepped game code (`session.call` runs the
/// Functor Lang interpreter) can't leak the world. `forward_step_scene` runs repeatedly
/// (every ghost rebuild), and a future `catch_unwind` caller would otherwise
/// accumulate leaked worlds.
struct DryWorld(physics::WorldId);

impl Drop for DryWorld {
    fn drop(&mut self) {
        physics::remove_world(self.0);
    }
}

/// The producer entry the shell / T6d call to run a deterministic headless
/// forward-step (docs/time-travel.md T6b, forward-ghosting). It assembles a
/// DRY-RUN [`FrameCtx`] over entirely THROWAWAY state — a cloned model, a
/// silencing reporter, fresh logs/queues, a deterministic [`DryRunEffects`]
/// runner, `suppress_outbound = true`, and (when the game has physics) a
/// throwaway physics world seeded from a snapshot of the live
/// [`physics::DEFAULT_WORLD`], driven by a fresh [`SteppedPhysics::for_world`]
/// and scoped ACTIVE ([`physics::ActiveWorldScope`]) so game-code physics
/// readback and commands resolve against it — then steps the scene forward at
/// the fine `sub_dt` (`steps_per_division` sub-ticks per snapshot), returning
/// the stepped `(model, world-snapshot)` at each of the `divisions` division
/// boundaries.
///
/// The live producer state (model, world, recorder, clock, and the global
/// effect / net / audio queues) is COMPLETELY untouched: the throwaway world
/// is removed before returning, and nothing outbound escapes the suppressed
/// drain. `prev_tts` seeds the subscription-timer window so timers stay
/// continuous through the step; `start_tts` is the fork point's scene time.
#[allow(clippy::too_many_arguments)]
fn forward_step_scene_with_error(
    session: &Session,
    model: &Value,
    has_physics: bool,
    has_subscriptions: bool,
    prev_tts: Option<f64>,
    clock: ForwardClock<'_>,
    divisions: usize,
    steps_per_division: usize,
    inputs: ForwardInputs<'_>,
    capture_from_division: usize,
) -> (Vec<(Value, Option<Vec<u8>>)>, Option<String>) {
    // Pause the paused-inspector journal for the whole dry run: `--ghost`
    // forward-projection replays `tick`/`update` over throwaway state, and those
    // calls must NOT land in the live frame's journal (they are not real frame
    // executions). Restored on drop, including on an unwind from stepped game
    // code.
    let _journal_pause = JournalPause::enter();
    // Dry-run physics: snapshot the live world and restore it into a fresh
    // throwaway world, so stepping forward never touches the live world,
    // driver, or timeline. The `DryWorld` guard removes it on drop — including on
    // an unwind from the stepped game code (panic-safe cleanup). `None` when the
    // game has no physics — `step_physics` no-ops and no snapshot is taken.
    // (Caveat: for a physics game whose live world hasn't stepped yet, the
    // snapshot read lazily inserts an empty `DEFAULT_WORLD` — benign, and
    // ghosting only runs during live play when the world already exists.)
    let dry_world = if has_physics {
        physics::with_world(physics::DEFAULT_WORLD, |w| w.snapshot()).map(|bytes| {
            let id = physics::create_world([0.0, -9.81, 0.0]);
            physics::with_world(id, |w| {
                let _ = w.restore(&bytes);
            });
            DryWorld(id)
        })
    } else {
        None
    };
    let mut physics_rt = match &dry_world {
        Some(dry) => SteppedPhysics::for_world(dry.0),
        None => SteppedPhysics::new(),
    };
    // Scope game-code physics to the throwaway world for the whole step:
    // readbacks (`Physics.position` / `Physics.transformed` / `Physics.raycast`)
    // in the stepped frame bodies answer against the PROJECTED world, and
    // replayed commands (a recorded kick) apply to it — never the live world.
    let _world_scope = dry_world
        .as_ref()
        .map(|dry| physics::ActiveWorldScope::enter(dry.0));

    let mut model = model.clone();
    let mut physics_frame = 0u64;
    let mut dry_recorder = SceneRecorder::new();
    let mut effect_runner = DryRunEffects::new();
    let mut effect_log = EffectLog::new();
    let mut deferred_queries: Vec<EffectTree> = Vec::new();
    let mut pending_events: Vec<PhysicsEvent> = Vec::new();
    let mut live_conn_keys: HashSet<String> = HashSet::new();
    let mut prev_tts = prev_tts;
    // Throwaway input buffer: the forward-step replays `inputs` directly and
    // never records, so this stays empty — it just satisfies the borrow.
    let mut input_buf: Vec<RecordedInput> = Vec::new();
    // The dry run never tracks assets: no snapshot, throwaway delivered slot.
    let mut delivered_asset_progress: Option<AssetProgress> = None;
    let mut reporter = Reporter::new(
        SpanSource::Single {
            src: String::new(),
            path: String::new(),
        },
        silent_emit,
    );

    let result = {
        let mut clock = clock;
        let mut ctx = FrameCtx {
            session,
            model: &mut model,
            physics_rt: &mut physics_rt,
            physics_frame: &mut physics_frame,
            recorder: &mut dry_recorder,
            effect_runner: &mut effect_runner as &mut dyn EffectRunner,
            effect_log: &mut effect_log,
            deferred_queries: &mut deferred_queries,
            pending_events: &mut pending_events,
            live_conn_keys: &mut live_conn_keys,
            prev_tts: &mut prev_tts,
            input_buf: &mut input_buf,
            has_physics,
            has_subscriptions,
            asset_progress: None,
            delivered_asset_progress: &mut delivered_asset_progress,
            suppress_outbound: true,
            reporter: &mut reporter,
        };
        ctx.step_scene_forward(
            divisions,
            steps_per_division,
            |step| inputs.at(step),
            |step| clock.next(step),
            capture_from_division,
        )
    };

    // Natural drop order (declaration-reverse, also on unwind) restores the
    // world scope FIRST, then `dry_world`'s `Drop` removes the throwaway world
    // — so `active_world()` never points at a removed world.
    (result, reporter.last_error())
}

/// Deterministically project a scene over throwaway state. Runtime errors are
/// intentionally swallowed for visual previews, which simply omit/bound the
/// affected projection; authoritative history replay uses the checked internal
/// result instead.
#[allow(clippy::too_many_arguments)]
pub fn forward_step_scene(
    session: &Session,
    model: &Value,
    has_physics: bool,
    has_subscriptions: bool,
    prev_tts: Option<f64>,
    start_tts: f32,
    sub_dt: f32,
    divisions: usize,
    steps_per_division: usize,
    inputs: &[Vec<RecordedInput>],
) -> Vec<(Value, Option<Vec<u8>>)> {
    forward_step_scene_with_error(
        session,
        model,
        has_physics,
        has_subscriptions,
        prev_tts,
        ForwardClock::Fixed {
            tts: start_tts,
            dts: sub_dt,
        },
        divisions,
        steps_per_division,
        ForwardInputs::Dense(inputs),
        0,
    )
    .0
}

/// Rebuild the complete retained pure-model timeline under the currently loaded
/// program after a plain-data hot reload. Retained snapshots are old data: using
/// them directly would preserve derived state from the old program (Mario's
/// already-launched `vy`, for example), so a changed constant could alter only
/// the beginning of a preview and then converge on the old outcome.
///
/// Replay starts from the edited program's actual `init` and is available only
/// while the complete frame-zero input history is retained. Games with an
/// `update` entry point are excluded for now because their model may depend on
/// effect results or UI/network/audio/asset events that the input log does not
/// yet capture. Physics games likewise need historical world replay.
pub fn materialize_counterfactual_history(
    session: &Session,
    model: &mut Value,
    recorder: &mut SceneRecorder,
    has_physics: bool,
    has_subscriptions: bool,
    preserve_selected_model: bool,
) -> Result<Option<usize>, String> {
    if has_physics || session.global("update").is_some() {
        return Ok(None);
    }
    let Some((lo, _, hi)) = recorder.counterfactual_replay_span()? else {
        return Ok(None);
    };
    let Some(init) = session.global("init") else {
        return Ok(None);
    };
    let recorded_frames = hi as usize + 1;
    let prefix_len = recorder.replay_prefix_len();
    let steps = prefix_len + recorded_frames;
    let (stepped, replay_error) = forward_step_scene_with_error(
        session,
        &init,
        false,
        has_subscriptions,
        None,
        ForwardClock::Recorder(recorder),
        steps,
        1,
        ForwardInputs::Recorder(recorder),
        prefix_len + lo as usize,
    );
    if let Some(error) = replay_error {
        return Err(format!(
            "counterfactual history replay failed; retained old snapshots: {error}"
        ));
    }
    let rebuilt: Vec<Value> = stepped.into_iter().map(|(model, _)| model).collect();
    let selected_override = preserve_selected_model.then(|| model.clone());
    let Some(selected) = recorder.materialize_counterfactual_history(
        &rebuilt,
        selected_override.as_ref(),
    )? else {
        return Err(
            "counterfactual history replay produced an incomplete timeline; retained old snapshots"
                .to_string(),
        );
    };
    *model = selected;
    Ok(Some(recorded_frames))
}

/// Forward-ghosting (docs/time-travel.md T6d): the shared producer body behind
/// both shells' `GameProducer::ghost_frames`. Step the scene forward over a
/// window of `divisions` divisions, each `dt` wide, from `start_tts` (a dry run
/// over throwaway state via [`forward_step_scene`] — the live producer is
/// untouched), then `draw` each stepped model at its division-boundary time —
/// with that division's stepped WORLD snapshot scoped active, so
/// `Physics.transformed` / `Physics.position` in `draw` render the projected
/// poses — and return the frames, each PAIRED with the division-boundary
/// [`FrameTime`] it was drawn at (`dts = 0`: a still of the future), for the
/// shell to composite — each at its own time, so render-time animation (the
/// skinned-skeleton pose) advances through the strobe. To keep velocity-integrated
/// motion (mario's jump) faithful, each division is advanced in FINE
/// `sub_dt = 1/60` sub-steps (`steps_per_division ≈ dt / sub_dt`) and sampled
/// only at the boundary, so the strobe still has `divisions` frames but each is
/// accurate integration. Division `div` draws at
/// `tts = start_tts + (div+1)*steps_per_division*sub_dt`, matching the time
/// `forward_step_scene` stepped the model to (the same f32 arithmetic). Each
/// frame's camera is overridden to the paused view (`last_frame.camera`) so only
/// world motion smears. A draw that errors or doesn't return a Frame is skipped,
/// so the result may be shorter than `divisions`.
///
/// `script_inputs` selects the input source (docs/time-travel.md F2). When
/// `Some`, the ghost forward-steps from `model` (the live anchor — K is NOT
/// resolved from the recorder) replaying the caller-supplied SCRIPT slice, so the
/// strobe is the *scripted* trajectory under the current code. When `None`, the
/// T6d behavior: resolve K and replay the recorder's own log.
#[allow(clippy::too_many_arguments)]
pub fn ghost_frames(
    session: &Session,
    model: &Value,
    recorder: &SceneRecorder,
    has_physics: bool,
    has_subscriptions: bool,
    prev_tts: Option<f64>,
    last_frame: &Frame,
    divisions: usize,
    dt: f32,
    start_tts: f64,
    script_inputs: Option<&[Vec<RecordedInput>]>,
) -> Vec<(Frame, FrameTime)> {
    // Replay the recorded inputs for the frames AFTER the fork point K, so a
    // recorded jump/run ghosts (docs/time-travel.md T6b). The input log is
    // per-rendered-frame = per-fine-step (both 1/60), so it feeds the fine
    // step index directly. Beyond the recorded window the step coasts. Under
    // F2 (`script_inputs = Some`) the caller's per-fine-step script slice is
    // used directly, forward-stepping from the current anchor model.
    let recorded;
    let inputs: &[Vec<RecordedInput>] = match script_inputs {
        Some(slice) => slice,
        None => {
            recorded = match recorder.current_scene_frame() {
                Some(k) => recorder.inputs_from(k + 1),
                None => Vec::new(),
            };
            &recorded
        }
    };
    // Fine sub-step at 1/60; round the division width to a whole number of
    // sub-steps (≥1). At the default 8 divisions over a ~2s window that's
    // dt = 0.25 → ~15 fine steps per division.
    let sub_dt = 1.0f32 / 60.0;
    let steps_per_division = ((dt / sub_dt).round() as usize).max(1);
    let stepped = forward_step_scene(
        session,
        model,
        has_physics,
        has_subscriptions,
        // The projection starts at the selected frame's time. In particular,
        // a scrubbed preview must not seed subscription windows from the old
        // live tail's `prev_tts`.
        recorder.current_scene_frame_tts().or(prev_tts),
        start_tts as f32,
        sub_dt,
        divisions,
        steps_per_division,
        inputs,
    );
    // Ghost draws must read the PROJECTED world: `Physics.transformed` /
    // `Physics.position` in `draw` resolve against the active world, so each
    // division's stepped world snapshot is restored into a throwaway world
    // scoped active for that division's draw — otherwise every ghost frame
    // would render physics bodies at the paused LIVE pose. One throwaway world
    // is reused across divisions (each restore overwrites it); the `DryWorld`
    // guard removes it on every exit path.
    let draw_world = stepped
        .iter()
        .any(|(_, w)| w.is_some())
        .then(|| DryWorld(physics::create_world([0.0, -9.81, 0.0])));
    let mut frames = Vec::with_capacity(stepped.len());
    for (i, (model_i, world_i)) in stepped.iter().enumerate() {
        let _world_scope = match (&draw_world, world_i) {
            (Some(dry), Some(bytes)) => {
                physics::with_world(dry.0, |w| {
                    let _ = w.restore(bytes);
                });
                Some(physics::ActiveWorldScope::enter(dry.0))
            }
            _ => None,
        };
        // Match forward_step_scene's f32 division-boundary tts exactly, so
        // each redrawn frame's tts equals the tts its model was stepped at.
        let tts = start_tts as f32 + (i as f32 + 1.0) * steps_per_division as f32 * sub_dt;
        let args = vec![model_i.clone(), Value::Number(tts as f64)];
        // A draw error or non-Frame return for a division is skipped, not fatal.
        if let Ok(value) = session.call("draw", args, &mut FunctorHost) {
            if let Some(frame) = frame_value(&value) {
                let mut frame = frame.clone();
                // Freeze the view: composite only world motion, not the camera.
                frame.camera = last_frame.camera.clone();
                frames.push((frame, FrameTime { dts: 0.0, tts }));
            }
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functor_lang_prelude::UiHandler;
    use crate::ui::{UiEvent, UiEventKind};

    /// A minimal MVU game with one message per widget shape: a verbatim msg
    /// (the button contract) and taggers over a number / a string (the
    /// slider / text-input contracts). The `expect*` globals are the models
    /// the tests assert against (compared via `Display`, so the expectation
    /// is built by the same evaluator as the result).
    const SRC: &str = "\
        type Msg = | Inc | SetN(n: Float) | SetS(s: String)\n\
        type Model = { count: Float, n: Float, s: String }\n\
        let init = { count: 0.0, n: 0.0, s: \"\" }\n\
        let update = (m: Model, msg: Msg) =>\n\
          match msg with\n\
          | Inc => { m with count: m.count + 1.0 }\n\
          | SetN(n) => { m with n: n }\n\
          | SetS(s) => { m with s: s }\n\
        let incMsg = Inc\n\
        let setN = (v) => SetN(v)\n\
        let setS = (v) => SetS(v)\n\
        let expectInc = { count: 1.0, n: 0.0, s: \"\" }\n\
        let expectSlider = { count: 0.0, n: 0.7, s: \"\" }\n\
        let expectText = { count: 0.0, n: 0.0, s: \"hi\" }\n";

    fn load_session() -> (Session, Value) {
        let project = functor_lang::project::load_single_source("game", SRC)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let model = session.global("init").expect("init");
        (session, model)
    }

    #[test]
    fn failed_counterfactual_replay_keeps_the_old_history_atomically() {
        let src = "\
            let init = 0.0\n\
            let tick = (m, dt, tts) => match m with | 0.0 => 1.0\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));

        let mut recorder = SceneRecorder::new();
        for model in [10.0, 20.0, 30.0] {
            recorder.record_inputs(Vec::new());
            recorder.record(&Value::Number(model), 0, model / 60.0);
        }
        let mut model = Value::Number(30.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        recorder
            .seek_scene_to(0, &mut model, &mut physics, &mut physics_frame, false)
            .expect("scrub to a historical frame");
        recorder.finish_reload(&model, physics_frame, true);

        let error =
            materialize_counterfactual_history(
                &session,
                &mut model,
                &mut recorder,
                false,
                false,
                false,
            )
                .expect_err("the second replayed tick has no matching arm");
        assert!(error.contains("retained old snapshots"), "{error}");

        recorder
            .seek_scene_to(1, &mut model, &mut physics, &mut physics_frame, false)
            .expect("old future remains seekable");
        assert_eq!(model.to_string(), "20");
    }

    #[test]
    fn same_source_counterfactual_replay_matches_cumulative_game_clock_tts() {
        let src = "let init = 0.0\nlet tick = (m, dt, tts) => m + dt + tts\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));

        let mut recorder = SceneRecorder::new();
        let mut recorded = Vec::new();
        let mut tts = 0.0f32;
        let mut live_model = 0.0f64;
        let sub_dt = 1.0f32 / 60.0;
        for frame in 0..1_000 {
            let dts = if frame == 400 { 0.25 } else { sub_dt };
            tts += dts;
            live_model += dts as f64 + tts as f64;
            let value = Value::Number(live_model);
            recorded.push(value.to_string());
            recorder.record_inputs(Vec::new());
            recorder.record_timed(&value, 0, FrameTime { dts, tts });
        }
        let (lo, hi) = recorder.scene_frame_range().expect("retained history");
        assert!(lo > 0, "exercise replay after model-ring pruning");

        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        recorder
            .seek_scene_to(lo, &mut model, &mut physics, &mut physics_frame, false)
            .expect("scrub retained history");
        recorder.finish_reload(&model, physics_frame, true);
        materialize_counterfactual_history(
            &session,
            &mut model,
            &mut recorder,
            false,
            false,
            false,
        )
            .expect("replay succeeds")
            .expect("historical reload replays");

        for frame in lo..=hi {
            recorder
                .seek_scene_to(frame, &mut model, &mut physics, &mut physics_frame, false)
                .expect("seek rebuilt history");
            assert_eq!(
                model.to_string(),
                recorded[frame as usize],
                "same-source replay diverged at frame {frame}"
            );
        }
    }

    #[test]
    fn counterfactual_replay_preserves_paused_input_and_ignores_removed_handlers() {
        let src = "let init = 0.0\nlet tick = (m, dt, tts) => m + 1.0\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));

        let mut recorder = SceneRecorder::new();
        recorder.record_replay_prefix(
            FrameTime { dts: 0.0, tts: 0.0 },
            Vec::new(),
        );
        for frame in 0..3 {
            let inputs = (frame == 1)
                .then(|| {
                    vec![RecordedInput::Key {
                        code: crate::Key::Right as i32,
                        is_down: true,
                    }]
                })
                .unwrap_or_default();
            recorder.record_inputs(inputs);
            recorder.record_timed(
                &Value::Number((frame + 2) as f64),
                0,
                FrameTime {
                    dts: 1.0 / 60.0,
                    tts: (frame + 1) as f32 / 60.0,
                },
            );
        }
        let mut model = Value::Number(0.0);
        let mut physics = SteppedPhysics::new();
        let mut physics_frame = 0;
        recorder
            .seek_scene_to(1, &mut model, &mut physics, &mut physics_frame, false)
            .expect("scrub into history");
        // A live input while paused has already updated the authoritative model
        // but is not part of the recorded future until Resume.
        model = Value::Number(99.0);
        recorder.finish_reload(&model, physics_frame, true);

        assert_eq!(
            materialize_counterfactual_history(
                &session,
                &mut model,
                &mut recorder,
                false,
                false,
                true,
            )
            .expect("removed input handler is not a replay error"),
            Some(3)
        );
        assert_eq!(
            model.to_string(),
            "99",
            "rebuild must not overwrite the authoritative paused-input model"
        );
        recorder
            .seek_scene_to(2, &mut model, &mut physics, &mut physics_frame, false)
            .expect("seek rebuilt recorded future");
        assert_eq!(model.to_string(), "4", "other snapshots are reconstructed");
    }

    /// Drive one `deliver_ui_event` through a throwaway [`FrameCtx`] (the
    /// dry-run construction of `forward_step_scene`, minus physics).
    fn deliver(session: &Session, model: &mut Value, handlers: &[UiHandler], event: &UiEvent) {
        let mut physics_rt = SteppedPhysics::new();
        let mut physics_frame = 0u64;
        let mut recorder = SceneRecorder::new();
        let mut effect_runner = DryRunEffects::new();
        let mut effect_log = EffectLog::new();
        let mut deferred_queries: Vec<EffectTree> = Vec::new();
        let mut pending_events: Vec<PhysicsEvent> = Vec::new();
        let mut live_conn_keys: HashSet<String> = HashSet::new();
        let mut prev_tts: Option<f64> = None;
        let mut input_buf: Vec<RecordedInput> = Vec::new();
        let mut delivered_asset_progress: Option<AssetProgress> = None;
        let mut reporter = Reporter::new(
            SpanSource::Single {
                src: String::new(),
                path: String::new(),
            },
            silent_emit,
        );
        let mut ctx = FrameCtx {
            session,
            model,
            physics_rt: &mut physics_rt,
            physics_frame: &mut physics_frame,
            recorder: &mut recorder,
            effect_runner: &mut effect_runner as &mut dyn EffectRunner,
            effect_log: &mut effect_log,
            deferred_queries: &mut deferred_queries,
            pending_events: &mut pending_events,
            live_conn_keys: &mut live_conn_keys,
            prev_tts: &mut prev_tts,
            input_buf: &mut input_buf,
            has_physics: false,
            has_subscriptions: false,
            asset_progress: None,
            delivered_asset_progress: &mut delivered_asset_progress,
            suppress_outbound: false,
            reporter: &mut reporter,
        };
        ctx.deliver_ui_event(handlers, event);
    }

    fn assert_model(session: &Session, model: &Value, expected_global: &str) {
        let expected = session.global(expected_global).expect(expected_global);
        assert_eq!(model.to_string(), expected.to_string());
    }

    #[test]
    fn ui_click_delivers_the_msg_through_update() {
        let (session, mut model) = load_session();
        let handlers = vec![UiHandler::Msg(session.global("incMsg").unwrap())];
        deliver(
            &session,
            &mut model,
            &handlers,
            &UiEvent {
                slot: 0,
                kind: UiEventKind::Clicked,
            },
        );
        assert_model(&session, &model, "expectInc");
    }

    #[test]
    fn ui_slider_applies_the_tagger_to_the_new_value() {
        let (session, mut model) = load_session();
        // Slot 1: also proves slot addressing picks the right handler.
        let handlers = vec![
            UiHandler::Msg(session.global("incMsg").unwrap()),
            UiHandler::Tagger(session.global("setN").unwrap()),
        ];
        deliver(
            &session,
            &mut model,
            &handlers,
            &UiEvent {
                slot: 1,
                kind: UiEventKind::SliderChanged(0.7),
            },
        );
        assert_model(&session, &model, "expectSlider");
    }

    #[test]
    fn ui_text_change_applies_the_tagger_to_the_new_text() {
        let (session, mut model) = load_session();
        let handlers = vec![UiHandler::Tagger(session.global("setS").unwrap())];
        deliver(
            &session,
            &mut model,
            &handlers,
            &UiEvent {
                slot: 0,
                kind: UiEventKind::TextChanged("hi".to_string()),
            },
        );
        assert_model(&session, &model, "expectText");
    }

    #[test]
    fn ui_event_for_an_unknown_slot_is_dropped() {
        let (session, mut model) = load_session();
        let handlers = vec![UiHandler::Msg(session.global("incMsg").unwrap())];
        deliver(
            &session,
            &mut model,
            &handlers,
            &UiEvent {
                slot: 9,
                kind: UiEventKind::Clicked,
            },
        );
        assert_model(&session, &model, "init"); // unchanged
    }

    #[test]
    fn ui_click_on_a_tagger_handler_is_dropped() {
        let (session, mut model) = load_session();
        let handlers = vec![UiHandler::Tagger(session.global("setN").unwrap())];
        deliver(
            &session,
            &mut model,
            &handlers,
            &UiEvent {
                slot: 0,
                kind: UiEventKind::Clicked,
            },
        );
        assert_model(&session, &model, "init"); // unchanged
    }

    // ---- Paused-inspector replay journal (visual-debugger PR2) --------------

    use crate::functor_lang_prelude::FakeEffects;

    /// A game with a per-second subscription whose `update` returns an
    /// `Effect.now` — one subscription firing drives TWO `update` calls in a
    /// frame (the timer message, then its effect result), so the journal proves
    /// count > 1 and both provenance kinds.
    const INSPECTOR_SRC: &str = "\
        type Msg = | Tick | GotTime(t: Float)\n\
        type Model = { ticks: Float, lastTime: Float }\n\
        let init = { ticks: 0.0, lastTime: 0.0 }\n\
        let update = (m: Model, msg: Msg) =>\n\
          match msg with\n\
          | Tick => ({ m with ticks: m.ticks + 1.0 }, Effect.now((t) => GotTime(t)))\n\
          | GotTime(t) => { m with lastTime: t }\n\
        let subscriptions = (m: Model) => Sub.every(Time.seconds(1.0), Tick)\n\
        let tick = (m: Model, dt: Float, tts: Float) => m\n\
        let draw = (m: Model, tts: Float) =>\n\
          Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n";

    fn inspector_session() -> (Session, Value) {
        let project = functor_lang::project::load_single_source("game", INSPECTOR_SRC)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let model = session.global("init").expect("init");
        (session, model)
    }

    /// Run one live frame (subscriptions + tick) with the journal armed, driving
    /// the model with `FakeEffects` so `Effect.now` is deterministic.
    fn run_inspector_frame(session: &Session, model: &mut Value) {
        let mut physics_rt = SteppedPhysics::new();
        let mut physics_frame = 0u64;
        let mut recorder = SceneRecorder::new();
        let mut effect_runner = FakeEffects::new(42.0, vec![0.0]);
        let mut effect_log = EffectLog::new();
        let mut deferred_queries: Vec<EffectTree> = Vec::new();
        let mut pending_events: Vec<PhysicsEvent> = Vec::new();
        let mut live_conn_keys: HashSet<String> = HashSet::new();
        // prev_tts just below a period boundary so this frame crosses 1.0s and
        // fires the `Sub.every` timer.
        let mut prev_tts: Option<f64> = Some(0.9);
        let mut input_buf: Vec<RecordedInput> = Vec::new();
        let mut delivered_asset_progress: Option<AssetProgress> = None;
        let mut reporter = Reporter::new(
            SpanSource::Single {
                src: INSPECTOR_SRC.to_string(),
                path: "game".to_string(),
            },
            silent_emit,
        );
        let mut ctx = FrameCtx {
            session,
            model,
            physics_rt: &mut physics_rt,
            physics_frame: &mut physics_frame,
            recorder: &mut recorder,
            effect_runner: &mut effect_runner as &mut dyn EffectRunner,
            effect_log: &mut effect_log,
            deferred_queries: &mut deferred_queries,
            pending_events: &mut pending_events,
            live_conn_keys: &mut live_conn_keys,
            prev_tts: &mut prev_tts,
            input_buf: &mut input_buf,
            has_physics: false,
            has_subscriptions: true,
            asset_progress: None,
            delivered_asset_progress: &mut delivered_asset_progress,
            suppress_outbound: false,
            reporter: &mut reporter,
        };
        ctx.before_physics(FrameTime { dts: 0.2, tts: 1.1 });
    }

    const ASSETS_SRC: &str = "\
        let init = { count: 0.0, loaded: 99.0, total: 99.0, failedCount: 99.0 }\n\
        let update = (m, p) =>\n\
          { count: m.count + 1.0, loaded: p.loaded, total: p.total,\n\
            failedCount: List.length(p.failed) }\n\
        let subscriptions = (m) => Sub.assets((p) => p)\n\
        let tick = (m, dt, tts) => m\n\
        let expectedFirst = { count: 1.0, loaded: 1.0, total: 3.0, failedCount: 1.0 }\n\
        let expectedSettled = { count: 2.0, loaded: 3.0, total: 3.0, failedCount: 1.0 }\n\
        let expectedFirstSettled = { count: 1.0, loaded: 3.0, total: 3.0, failedCount: 1.0 }\n";

    /// Run one frame (subscriptions + tick) with the given shell snapshot,
    /// the way a shell drives the producer.
    fn run_assets_frame(
        session: &Session,
        model: &mut Value,
        progress: Option<AssetProgress>,
        delivered: &mut Option<AssetProgress>,
        tts: f64,
    ) {
        let mut physics_rt = SteppedPhysics::new();
        let mut physics_frame = 0u64;
        let mut recorder = SceneRecorder::new();
        let mut effect_runner = DryRunEffects::new();
        let mut effect_log = EffectLog::new();
        let mut deferred_queries: Vec<EffectTree> = Vec::new();
        let mut pending_events: Vec<PhysicsEvent> = Vec::new();
        let mut live_conn_keys: HashSet<String> = HashSet::new();
        let mut prev_tts: Option<f64> = Some(tts - 0.1);
        let mut input_buf: Vec<RecordedInput> = Vec::new();
        let mut reporter = Reporter::new(
            SpanSource::Single {
                src: ASSETS_SRC.to_string(),
                path: "game".to_string(),
            },
            silent_emit,
        );
        let mut ctx = FrameCtx {
            session,
            model,
            physics_rt: &mut physics_rt,
            physics_frame: &mut physics_frame,
            recorder: &mut recorder,
            effect_runner: &mut effect_runner as &mut dyn EffectRunner,
            effect_log: &mut effect_log,
            deferred_queries: &mut deferred_queries,
            pending_events: &mut pending_events,
            live_conn_keys: &mut live_conn_keys,
            prev_tts: &mut prev_tts,
            input_buf: &mut input_buf,
            has_physics: false,
            has_subscriptions: true,
            asset_progress: progress,
            delivered_asset_progress: delivered,
            suppress_outbound: false,
            reporter: &mut reporter,
        };
        ctx.before_physics(FrameTime {
            dts: 0.1,
            tts: tts as f32,
        });
    }

    /// `Sub.assets` delivers the snapshot through `update` exactly when it
    /// changes: on the first sighting, NOT on an identical frame, and again
    /// when loading settles.
    #[test]
    fn asset_progress_flows_to_update_once_per_change() {
        let project = functor_lang::project::load_single_source("game", ASSETS_SRC)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").expect("init");
        let mut delivered: Option<AssetProgress> = None;

        let loading = AssetProgress {
            loaded: 1,
            total: 3,
            failed: vec![("a.glb".to_string(), "404".to_string())],
        };
        run_assets_frame(
            &session,
            &mut model,
            Some(loading.clone()),
            &mut delivered,
            1.1,
        );
        assert_model(&session, &model, "expectedFirst");

        // Same snapshot: no redelivery (count stays 1).
        run_assets_frame(&session, &mut model, Some(loading), &mut delivered, 1.2);
        assert_model(&session, &model, "expectedFirst");

        // Loading settles: the change delivers once more.
        let settled = AssetProgress {
            loaded: 3,
            total: 3,
            failed: vec![("a.glb".to_string(), "404".to_string())],
        };
        run_assets_frame(
            &session,
            &mut model,
            Some(settled.clone()),
            &mut delivered,
            1.3,
        );
        assert_model(&session, &model, "expectedSettled");

        // A time-travel branch restores an older model and INVALIDATES the
        // delivered marker (before_physics / rewind_scene_to): the next frame
        // redelivers the unchanged current snapshot, so a rewound loading
        // screen learns its assets already settled.
        let mut rewound = session.global("init").expect("init");
        delivered = None;
        run_assets_frame(&session, &mut rewound, Some(settled), &mut delivered, 1.4);
        assert_model(&session, &rewound, "expectedFirstSettled");
    }

    #[test]
    fn journal_records_a_frame_with_provenance_and_replay_matches() {
        journal_disarm(); // this thread may be reused across tests
        journal_arm();
        let (session, model) = inspector_session();

        let mut m = model.clone();
        run_inspector_frame(&session, &mut m);
        let journal = journal_swap().expect("journaling armed");

        // Subscription `update` (Tick), its effect-result `update` (GotTime),
        // then `tick` — the frame's model-updating calls, in order.
        assert_eq!(journal.len(), 3, "tick + two updates");
        assert_eq!(journal[0].entry, "update");
        assert_eq!(
            journal[0].provenance.render(&journal[0].args),
            "subscription: Tick"
        );
        assert_eq!(journal[1].entry, "update");
        assert_eq!(
            journal[1].provenance.render(&journal[1].args),
            "effect result: GotTime(42)"
        );
        assert_eq!(journal[2].entry, "tick");
        assert_eq!(
            journal[2].provenance.render(&journal[2].args),
            "tick dt=0.2"
        );

        // Replaying each journaled call through the PR1 recorder reproduces the
        // exact result of a direct `call` (entry points are pure), and records
        // binding sites (the seam the desktop trace serializes).
        for e in &journal {
            let direct = session
                .call(e.entry, e.args.clone(), &mut FunctorHost)
                .expect("direct call");
            let (replayed, inv) = session
                .call_recorded(e.entry, e.args.clone(), &mut FunctorHost)
                .expect("recorded call");
            assert_eq!(replayed.to_string(), direct.to_string());
            assert_eq!(inv.result, direct.to_string());
            // Each call binds its params (`m`, `msg`/`dt`/`tts`) — non-empty.
            assert!(!inv.bindings.is_empty(), "recorded some binding sites");
        }

        journal_disarm();
    }

    #[test]
    fn journal_survives_a_paused_frame_and_excludes_ghost_calls() {
        journal_disarm();
        journal_arm();
        let (session, model) = inspector_session();

        // A real frame fills the journal; swap it out (the producer's frame-end
        // move into `last_frame_journal`).
        let mut m = model.clone();
        run_inspector_frame(&session, &mut m);
        let last_frame = journal_swap().expect("armed");
        assert_eq!(last_frame.len(), 3);

        // A dry-run forward-step (the `--ghost` projection) calls tick/update
        // over throwaway state — and must NOT touch the live journal. After it,
        // the freshly-armed journal is still empty (ghost calls excluded).
        let _ = forward_step_scene(
            &session,
            &m,
            false, // has_physics
            true,  // has_subscriptions
            Some(0.9),
            1.1,
            1.0 / 60.0,
            3, // divisions
            1, // steps per division
            &[],
        );
        let after_ghost = journal_swap().expect("still armed");
        assert!(
            after_ghost.is_empty(),
            "ghost forward-projection calls must not be journaled, got {}",
            after_ghost.len()
        );

        journal_disarm();
    }

    #[test]
    fn journaling_off_by_default_is_a_noop() {
        // With no `journal_arm` (the web / plain-frame path), running a frame
        // journals nothing — the perf gate: no collection, no Display rendering.
        journal_disarm();
        let (session, model) = inspector_session();
        let mut m = model.clone();
        run_inspector_frame(&session, &mut m);
        assert!(
            journal_swap().is_none(),
            "journaling must stay off until explicitly armed"
        );
    }
}
