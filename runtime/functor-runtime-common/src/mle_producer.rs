//! The shared MVU per-frame body for the MLE producers (docs/time-travel.md
//! T6a). Both shells — the desktop `functor-runner` and the web/wasm runtime —
//! run the SAME game contract and per-frame orchestration; only I/O differs
//! (file-watch vs editor push, winit vs DOM input *delivery*, stderr vs the
//! browser console, native perf timing). Historically the frame body and its
//! private helpers were copy-pasted between the two `mle_game.rs` producers;
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

use mle::{line_col, project::SourceMap, RunError, Session, Span, Value};

use crate::mle_prelude::{
    contains_effect, deliver_physics_events, drain_effects, http_response_value, needs_update,
    net_conn_subs, net_event_value, perform_deferred_queries, physics_event_taggers,
    physics_scene_value, split_model_effect, sub_messages_for_frame, take_audio_completion,
    take_http_tagger, EffectLog, EffectTree, FunctorHost, NetEventKind, RealEffects,
};
use crate::net::{push_conn_command, ConnCommand, HttpResult};
use crate::physics::{PhysicsEvent, SteppedPhysics};
use crate::timetravel::SceneRecorder;
use crate::FrameTime;

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
        let rendered = format!(
            "[mle] {stage} error at {}",
            self.source.render(err.span, &err.message)
        );
        self.report_once(rendered);
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
    pub physics_status: &'a mut (u64, bool, u64),
    pub recorder: &'a mut SceneRecorder,
    pub effect_runner: &'a mut RealEffects,
    pub effect_log: &'a mut EffectLog,
    pub deferred_queries: &'a mut Vec<EffectTree>,
    pub pending_events: &'a mut Vec<PhysicsEvent>,
    pub live_conn_keys: &'a mut HashSet<String>,
    pub prev_tts: &'a mut Option<f64>,
    pub has_physics: bool,
    pub has_subscriptions: bool,
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
                self.physics_status,
                self.has_physics,
            )
        {
            self.deferred_queries.clear();
            self.pending_events.clear();
        }
        // Subscriptions first, so `tick` sees a model that has absorbed this
        // frame's messages (the F# executor's ordering).
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
        let world_ready = physics_steps > 0 || !self.has_physics || self.physics_status.0 > 0;
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
                        );
                        for message in reports {
                            self.reporter.report_once(message);
                        }
                    }
                    Ok(_) => {}
                    Err(message) => self.reporter.report_once(format!("[mle] {message}")),
                },
                Err(err) => self.reporter.frame_error("subscriptions", &err),
            }
        }
    }

    /// Record the settled model of this rendered frame (docs/time-travel.md T1)
    /// plus the physics fixed-frame the world reached, in lockstep, so a coupled
    /// rewind can restore both. `physics_status.0` is the world's current fixed
    /// frame.
    ///
    /// Skip a PAUSED frame (`dts == 0`, i.e. the clock pinned): the sim hasn't
    /// advanced, so recording would only pile up frozen duplicates — inflating
    /// the timeline and pushing a rewind target past the real history.
    /// `dts == 0` is exactly the pinned-and-not-stepping case (a one-shot step
    /// carries `dts > 0`).
    pub fn record_frame(&mut self, frame_time: FrameTime) {
        if frame_time.dts > 0.0 {
            self.recorder.record(self.model, self.physics_status.0);
        }
    }

    /// Take an entry point's return: split off any `(model, effect)` pair,
    /// adopt the model, and drain the effects to a fixed point through `update`
    /// (docs/mle.md B6). Every producer path that runs game code funnels through
    /// here, so effects work uniformly from tick, input, mouse, and messages.
    pub fn absorb(&mut self, returned: Value) {
        let (model, effects) = split_model_effect(returned);
        *self.model = model;
        // Effects are commands, not data — one stored in the model would make
        // the pair sniff ambiguous on a later return (see `split_model_effect`).
        if contains_effect(self.model) {
            self.reporter.report_once(
                "[mle] the model contains an Effect value — Effects are commands, \
not data; return them beside the model as `(model, effect)` instead of storing them"
                    .to_string(),
            );
        }
        let Some(effects) = effects else { return };
        // Only MESSAGE-producing effects need an `update` to receive them —
        // tagger-less physics commands must not be dropped over a missing hook.
        if needs_update(&effects) && self.session.global("update").is_none() {
            self.reporter.report_once(
                "[mle] effects returned but there is no `let update = (model, msg) => …` \
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
    /// (docs/mle.md C4b-2). Subscriptions are recomputed from the current model
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
            Err(message) => return self.reporter.report_once(format!("[mle] {message}")),
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
                    *self.physics_status = advanced.status;
                    let steps = advanced.steps;
                    let warnings = advanced.warnings;
                    // Command effects apply asynchronously (queued at perform
                    // time, applied at the step), so their problems — unknown
                    // tag, queue overflow — surface here, deduped.
                    for warning in warnings {
                        self.reporter.report_once(format!("[mle] {warning}"));
                    }
                    return steps;
                }
                None => self.reporter.report_once(format!(
                    "[mle] physics must return Physics.scene(gx, gy, gz, [body, …]), got {}",
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
        let conns = match net_conn_subs(subs) {
            Ok(conns) => conns,
            Err(message) => return self.reporter.report_once(format!("[mle] {message}")),
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
            Err(message) => return self.reporter.report_once(format!("[mle] {message}")),
        };
        let Some(sub) = conns.into_iter().find(|c| c.key == key) else {
            return; // an event for a no-longer-declared connection: drop it
        };
        let value = net_event_value(kind, conn as u64, &text).to_mle();
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
