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

use crate::functor_lang_prelude::{
    contains_effect, deliver_physics_events, drain_effects, frame_value, http_response_value,
    needs_update, net_conn_subs, net_event_value, perform_deferred_queries, physics_event_taggers,
    physics_scene_value, split_model_effect, sub_messages_for_frame, take_audio_completion,
    take_http_tagger, DryRunEffects, EffectLog, EffectRunner, EffectTree, FunctorHost, NetEventKind,
};
use crate::input::{Key, RecordedInput};
use crate::net::{push_conn_command, ConnCommand, HttpResult};
use crate::physics::{self, PhysicsEvent, SteppedPhysics};
use crate::timetravel::SceneRecorder;
use crate::{Frame, FrameTime};

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
                    Err(message) => self.reporter.report_once(format!("[functor-lang] {message}")),
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
                .record(self.model, *self.physics_frame, frame_time.tts as f64);
        } else {
            // Paused frame (`dts == 0`): drain-and-drop the buffer. Paused frames
            // aren't part of the played timeline, so their buffered inputs must
            // NOT leak into the next stepped frame's recorded events.
            self.input_buf.clear();
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
    pub fn step_scene_forward(
        &mut self,
        divisions: usize,
        steps_per_division: usize,
        sub_dt: f32,
        start_tts: f32,
        inputs: &[Vec<RecordedInput>],
    ) -> Vec<(Value, Option<Vec<u8>>)> {
        let mut out = Vec::with_capacity(divisions);
        let mut step = 0usize;
        for _div in 0..divisions {
            for _ in 0..steps_per_division {
                // Replay this fine step's recorded inputs before the frame body,
                // so the model absorbs them exactly as the live frame did (coast
                // when the log has no entry for this step).
                if let Some(events) = inputs.get(step) {
                    for event in events {
                        self.replay_input(*event);
                    }
                }
                let frame_time = FrameTime {
                    dts: sub_dt,
                    tts: start_tts + (step as f32 + 1.0) * sub_dt,
                };
                self.subscriptions_and_tick(frame_time);
                self.physics_phase(frame_time);
                step += 1;
            }
            // Snapshot only at the division boundary — the strobe still has
            // `divisions` frames, but each is the result of accurate fine
            // integration over `steps_per_division` sub-ticks.
            let world = if self.has_physics {
                self.physics_rt.snapshot_world()
            } else {
                None
            };
            out.push((self.model.clone(), world));
        }
        out
    }

    /// Replay one recorded input event during the forward-step, mirroring the
    /// LIVE path (`key_event`/`mouse_move`/`mouse_wheel`): call the game's
    /// `input`/`mouseMove`/`mouseWheel` entry point with the SAME reconstructed
    /// args, then [`Self::absorb`] the result (which honors `suppress_outbound`,
    /// so nothing escapes). A `Key` re-runs `Key::from_i32` on the raw code just
    /// as the live path does; an unknown code is dropped, like live. An entry
    /// point the game doesn't define resolves to an interpreter error, reported
    /// (and silenced) through the dry-run reporter.
    fn replay_input(&mut self, event: RecordedInput) {
        let (entry, args) = match event {
            RecordedInput::Key { code, is_down } => {
                let Some(key) = Key::from_i32(code) else {
                    return;
                };
                (
                    "input",
                    vec![
                        self.model.clone(),
                        Value::String(std::rc::Rc::from(format!("{key:?}").as_str())),
                        Value::Bool(is_down),
                    ],
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
            RecordedInput::MouseWheel { delta } => {
                ("mouseWheel", vec![self.model.clone(), Value::Number(delta as f64)])
            }
        };
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
        let subs = match self
            .session
            .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
        {
            Ok(subs) => subs,
            Err(err) => return self.reporter.frame_error("subscriptions", &err),
        };
        // Reconcile connections EVERY frame — including frame one (before the
        // timer window exists), so a declared connection opens immediately.
        self.reconcile_connections(&subs);
        let Some(prev) = prev else {
            return;
        };
        let msgs = match sub_messages_for_frame(&subs, prev, tts) {
            Ok(msgs) => msgs,
            Err(message) => return self.reporter.report_once(format!("[functor-lang] {message}")),
        };
        for msg in msgs {
            match self
                .session
                .call("update", vec![self.model.clone(), msg], &mut FunctorHost)
            {
                Ok(returned) => self.absorb(returned),
                Err(err) => self.reporter.frame_error("update", &err),
            }
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
                        self.reporter.report_once(format!("[functor-lang] {warning}"));
                    }
                    return steps;
                }
                None => self.reporter.report_once(format!(
                    "[functor-lang] physics must return Physics.scene(gx, gy, gz, [body, …]), got {}",
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
            Err(message) => return self.reporter.report_once(format!("[functor-lang] {message}")),
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
        let subs = match self
            .session
            .call("subscriptions", vec![self.model.clone()], &mut FunctorHost)
        {
            Ok(subs) => subs,
            Err(err) => return self.reporter.frame_error("subscriptions", &err),
        };
        let conns = match net_conn_subs(&subs) {
            Ok(conns) => conns,
            Err(message) => return self.reporter.report_once(format!("[functor-lang] {message}")),
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
        match self
            .session
            .call("update", vec![self.model.clone(), msg], &mut FunctorHost)
        {
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
        match self
            .session
            .call("update", vec![self.model.clone(), msg], &mut FunctorHost)
        {
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
        match self
            .session
            .call("update", vec![self.model.clone(), message], &mut FunctorHost)
        {
            Ok(returned) => self.absorb(returned),
            Err(err) => self.reporter.frame_error("update", &err),
        }
    }
}

/// A silencing error sink for the dry-run forward-step's [`Reporter`]: its
/// per-frame errors are throwaway (the live frame already reports them), so
/// they go nowhere.
fn silent_emit(_: &str) {}

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
    let mut recorder = SceneRecorder::new();
    let mut effect_runner = DryRunEffects::new();
    let mut effect_log = EffectLog::new();
    let mut deferred_queries: Vec<EffectTree> = Vec::new();
    let mut pending_events: Vec<PhysicsEvent> = Vec::new();
    let mut live_conn_keys: HashSet<String> = HashSet::new();
    let mut prev_tts = prev_tts;
    // Throwaway input buffer: the forward-step replays `inputs` directly and
    // never records, so this stays empty — it just satisfies the borrow.
    let mut input_buf: Vec<RecordedInput> = Vec::new();
    let mut reporter = Reporter::new(
        SpanSource::Single {
            src: String::new(),
            path: String::new(),
        },
        silent_emit,
    );

    let result = {
        let mut ctx = FrameCtx {
            session,
            model: &mut model,
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
            has_physics,
            has_subscriptions,
            suppress_outbound: true,
            reporter: &mut reporter,
        };
        ctx.step_scene_forward(divisions, steps_per_division, sub_dt, start_tts, inputs)
    };

    // Natural drop order (declaration-reverse, also on unwind) restores the
    // world scope FIRST, then `dry_world`'s `Drop` removes the throwaway world
    // — so `active_world()` never points at a removed world.
    result
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
        prev_tts,
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
