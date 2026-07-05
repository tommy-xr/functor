//! The live physics world: Rapier state, reconcile, fixed-step, snapshots.
//!
//! A [`World`] is the imperative-shell half of physics (docs/physics.md): it
//! owns the mutable Rapier solver state and is driven by two inputs only — the
//! per-frame declared [`PhysicsScene`] (via [`World::reconcile`]) and fixed
//! time steps (via [`World::step_frame`]). Given the same build and the same
//! sequence of those inputs, two worlds stay byte-identical (the determinism
//! goldens in `goldens.rs` assert exactly this).
//!
//! Snapshots serialize the *whole* world — every Rapier set plus broad/narrow
//! phase and this module's own bookkeeping — so `restore` resumes bit-exact
//! mid-flight. Rapier's scratch `PhysicsPipeline` is deliberately skipped
//! (recreated on restore; it holds no simulation-visible state).

use std::collections::BTreeMap;

use rapier3d::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Body, BodyKind, PhysicsScene, Shape};

/// The fixed simulation timestep. `step_frame` accumulates real dt and steps
/// the world in whole `FIXED_DT` substeps — Rapier is never stepped with a
/// variable dt (nondeterministic and unstable).
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Spiral-of-death guard: the most fixed substeps one `step_frame` will run.
/// A hitch longer than `MAX_SUBSTEPS_PER_FRAME * FIXED_DT` of real time drops
/// the backlog (simulation time falls behind wall clock) instead of stepping
/// unboundedly.
pub const MAX_SUBSTEPS_PER_FRAME: u32 = 8;

/// A fire-and-forget command against a declared body (docs/physics.md,
/// Phase 3): plain serializable data, queued via [`World::queue_command`] and
/// applied at the frame's **first fixed substep, after reconcile** — so a
/// body declared and commanded in the same frame works. Being plain data,
/// commands are also the Timeline's replayable per-frame input
/// (`timeline::Command::Apply`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PhysicsCommand {
    /// Instantaneous momentum change.
    ApplyImpulse { tag: String, impulse: [f32; 3] },
    /// A force applied for this frame's substeps only (cleared at frame end —
    /// no rapier force persistence to forget about).
    ApplyForce { tag: String, force: [f32; 3] },
    SetVelocity { tag: String, velocity: [f32; 3] },
    /// Move the live body without touching its declaration (the declared
    /// cache is unchanged, so the next frame's unchanged declaration does not
    /// snap it back).
    Teleport { tag: String, position: [f32; 3] },
}

impl PhysicsCommand {
    /// The command's tag and short kind name, for warnings and logs.
    pub fn tag_and_kind(&self) -> (&str, &'static str) {
        match self {
            PhysicsCommand::ApplyImpulse { tag, .. } => (tag, "applyImpulse"),
            PhysicsCommand::ApplyForce { tag, .. } => (tag, "applyForce"),
            PhysicsCommand::SetVelocity { tag, .. } => (tag, "setVelocity"),
            PhysicsCommand::Teleport { tag, .. } => (tag, "teleport"),
        }
    }
}

/// One contact transition from the physics step (docs/physics.md Phase 5):
/// plain data, in solver order (deterministic). `a`/`b` are the colliding
/// bodies' tags in rapier's pair order — games should check both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsEvent {
    /// `true` = the pair began touching this step; `false` = stopped.
    pub started: bool,
    pub a: String,
    pub b: String,
    /// At least one of the pair is a sensor (an overlap, not a contact).
    pub sensor: bool,
}

/// Commands queued while no physics step consumes them (a game firing
/// commands without a `physics` hook) are bounded — drop-with-warning beats
/// unbounded growth.
const MAX_PENDING_COMMANDS: usize = 1024;

/// Warnings are bounded too: they are only freed by a driver draining them,
/// and a driver that never steps the world never drains — the warning buffer
/// must not become the leak it exists to report.
const MAX_COMMAND_WARNINGS: usize = 64;

/// The nearest intersection from [`World::raycast`].
#[derive(Debug, Clone, PartialEq)]
pub struct RayHit {
    pub tag: String,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// World units from the ray origin (the direction is normalized).
    pub distance: f32,
}

/// One colored wireframe segment from [`World::debug_lines`], in world space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct DebugLine {
    pub a: [f32; 3],
    pub b: [f32; 3],
    /// Plain RGBA in 0..1 (converted from the HSLA rapier emits).
    pub color: [f32; 4],
}

/// Rapier's debug colors are HSLA; convert once at collection so consumers
/// (GL overlay, text dumps) get ordinary RGBA.
fn hsla_to_rgba([h, s, l, a]: [f32; 4]) -> [f32; 4] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    // rem_euclid: a negative hue must land in [0, 360), not keep its sign
    // (Rust `%` would send it to sector 0 with a negative chroma offset).
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m, a]
}

/// A live Rapier world plus the reconcile bookkeeping that maps declared
/// [`Body`] tags onto Rapier handles.
#[derive(Serialize, Deserialize)]
pub struct World {
    gravity: [f32; 3],
    integration_parameters: IntegrationParameters,
    #[serde(skip, default = "PhysicsPipeline::new")]
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    /// tag → live handles. `BTreeMap` so iteration (despawn order, dumps,
    /// serialization) is deterministic — a requirement, not a nicety.
    tags: BTreeMap<String, (RigidBodyHandle, ColliderHandle)>,
    /// Last-declared body per tag — the memory behind the divergence rule:
    /// reconcile writes a field to the live body only when the new declaration
    /// *differs from this cache*; an unchanged declaration leaves the body to
    /// the simulation.
    declared: BTreeMap<String, Body>,
    accumulator: f32,
    frame: u64,
    /// Commands awaiting the next stepped frame (serialized: a snapshot taken
    /// between queue and step must restore them for bit-exact resume).
    #[serde(default)]
    pending: Vec<PhysicsCommand>,
    /// Bodies given an [`PhysicsCommand::ApplyForce`] this frame — their
    /// forces are cleared once the frame's substeps finish. Intra-call state:
    /// populated and cleared within one `step_frame`/`step`, so it is always
    /// empty at any snapshot point (hence not serialized).
    #[serde(skip, default)]
    forced: Vec<RigidBodyHandle>,
    /// Contact transitions from this frame's substeps, drained by the driver
    /// after `step_frame` (cleared at the next frame's start, so an
    /// unsubscribed game cannot accumulate them). Not serialized: they are
    /// per-frame data, consumed before any snapshot point.
    #[serde(skip, default)]
    events: Vec<PhysicsEvent>,
    /// Command problems (unknown tag, overflow) for the driver to report —
    /// commands are applied asynchronously, so there is no call site to error
    /// at. Drain with [`World::take_command_warnings`].
    #[serde(skip, default)]
    command_warnings: Vec<String>,
}

impl World {
    pub fn new(gravity: [f32; 3]) -> World {
        let mut integration_parameters = IntegrationParameters::default();
        integration_parameters.dt = FIXED_DT;
        World {
            gravity,
            integration_parameters,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::default(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            tags: BTreeMap::new(),
            declared: BTreeMap::new(),
            accumulator: 0.0,
            frame: 0,
            pending: Vec::new(),
            forced: Vec::new(),
            events: Vec::new(),
            command_warnings: Vec::new(),
        }
    }

    /// Queue a command for the next stepped frame (applied after that frame's
    /// reconcile, before its first substep). Problems surface later through
    /// [`World::take_command_warnings`].
    pub fn queue_command(&mut self, command: PhysicsCommand) {
        if self.pending.len() >= MAX_PENDING_COMMANDS {
            let (tag, kind) = command.tag_and_kind();
            self.push_command_warning(format!(
                "physics command queue full ({MAX_PENDING_COMMANDS}); dropping {kind} \
                 \"{tag}\" (is anything stepping the world?)"
            ));
            return;
        }
        self.pending.push(command);
    }

    /// Problems from asynchronously-applied commands (unknown tags, queue
    /// overflow), drained for the driver to report.
    pub fn take_command_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.command_warnings)
    }

    /// Warning texts are stable per (kind, tag) — no per-frame payloads — so
    /// the drivers' report-once dedupe actually holds for a persistent bug.
    fn push_command_warning(&mut self, warning: String) {
        match self.command_warnings.len().cmp(&MAX_COMMAND_WARNINGS) {
            std::cmp::Ordering::Less => self.command_warnings.push(warning),
            std::cmp::Ordering::Equal => self
                .command_warnings
                .push("further physics command warnings suppressed until drained".to_string()),
            std::cmp::Ordering::Greater => {}
        }
    }

    /// Fixed frames stepped since creation (or since the restored snapshot's
    /// origin).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Diff the declared scene against the last-declared cache and apply the
    /// difference to the live world.
    ///
    /// Ordering is load-bearing for determinism (Rapier arena handles depend on
    /// the full insert/remove history), so the scene is canonicalized *by tag*:
    /// despawns first, then spawns/updates, both in tag order. Sorting — rather
    /// than declaration order — makes the handle history invariant to how the
    /// game happened to assemble the body list (docs/physics.md, "sort by
    /// tag"). A tag repeated in the scene keeps its first occurrence (mirrors
    /// `audio::reconcile`).
    pub fn reconcile(&mut self, scene: &PhysicsScene) {
        self.gravity = scene.gravity;

        let mut wanted: BTreeMap<&String, &Body> = BTreeMap::new();
        for body in &scene.bodies {
            wanted.entry(&body.tag).or_insert(body); // first occurrence wins
        }

        // Despawns: live tags no longer declared, in tag order.
        let gone: Vec<String> = self
            .tags
            .keys()
            .filter(|t| !wanted.contains_key(t))
            .cloned()
            .collect();
        for tag in &gone {
            self.despawn(tag);
        }

        // Spawns / updates, in tag order.
        for (tag, body) in wanted {
            match self.declared.get(tag) {
                None => self.spawn(body),
                Some(prev) if prev != body => {
                    let prev = prev.clone();
                    self.apply_divergence(&prev, body);
                }
                Some(_) => {} // unchanged: the simulation owns this body
            }
            self.declared.insert(tag.clone(), body.clone());
        }
    }

    /// Accumulate real (variable) dt and run whole fixed substeps, carrying the
    /// remainder. Returns the number of substeps taken.
    ///
    /// Queued commands apply immediately before the first substep — and stay
    /// pending through a zero-substep frame, so a command is never consumed by
    /// a frame that doesn't simulate (a frame-lasting force would otherwise be
    /// lost without ever integrating).
    pub fn step_frame(&mut self, real_dt: f32) -> u32 {
        // Last frame's undrained events are stale — drop them so a game with
        // no event subscription can't accumulate.
        self.events.clear();
        self.accumulator += real_dt.max(0.0);
        if self.accumulator < FIXED_DT {
            return 0;
        }
        self.apply_pending();
        let mut steps = 0;
        while self.accumulator >= FIXED_DT && steps < MAX_SUBSTEPS_PER_FRAME {
            self.step_fixed();
            self.accumulator -= FIXED_DT;
            steps += 1;
        }
        if steps == MAX_SUBSTEPS_PER_FRAME && self.accumulator >= FIXED_DT {
            // Hitch longer than the cap: drop the whole-step backlog rather
            // than owing ever more substeps, but keep the sub-step remainder so
            // the step phase is preserved.
            self.accumulator %= FIXED_DT;
        }
        self.clear_frame_forces();
        steps
    }

    /// Advance exactly one fixed step, collecting contact transitions into
    /// the frame's event buffer.
    pub fn step_fixed(&mut self) {
        // EventHandler must be Send + Sync; a Mutex'd Vec is the simplest
        // sink (single-threaded here — the lock is uncontended).
        #[derive(Default)]
        struct Sink(std::sync::Mutex<Vec<CollisionEvent>>);
        impl EventHandler for Sink {
            fn handle_collision_event(
                &self,
                _bodies: &RigidBodySet,
                _colliders: &ColliderSet,
                event: CollisionEvent,
                _contact_pair: Option<&ContactPair>,
            ) {
                self.0.lock().unwrap().push(event);
            }
            fn handle_contact_force_event(
                &self,
                _dt: Real,
                _bodies: &RigidBodySet,
                _colliders: &ColliderSet,
                _contact_pair: &ContactPair,
                _total_force_magnitude: Real,
            ) {
            }
        }

        let sink = Sink::default();
        self.pipeline.step(
            Vector::new(self.gravity[0], self.gravity[1], self.gravity[2]),
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &(),
            &sink,
        );
        // Resolve handles to tags NOW, while both sides are still mapped. A
        // pair involving a body despawned earlier this frame (rapier's
        // REMOVED stop-events) has no tag left — those events are dropped:
        // the game already unmade the body, there is nobody to notify about.
        for event in sink.0.into_inner().unwrap() {
            let (h1, h2, started, flags) = match event {
                CollisionEvent::Started(h1, h2, flags) => (h1, h2, true, flags),
                CollisionEvent::Stopped(h1, h2, flags) => (h1, h2, false, flags),
            };
            let tag_of = |handle| {
                self.tags
                    .iter()
                    .find(|(_, &(_, col))| col == handle)
                    .map(|(tag, _)| tag.clone())
            };
            if let (Some(a), Some(b)) = (tag_of(h1), tag_of(h2)) {
                self.events.push(PhysicsEvent {
                    started,
                    a,
                    b,
                    sensor: flags.contains(CollisionEventFlags::SENSOR),
                });
            }
        }
        self.frame += 1;
    }

    /// This frame's contact transitions (drained; see `step_frame`).
    pub fn take_events(&mut self) -> Vec<PhysicsEvent> {
        std::mem::take(&mut self.events)
    }

    /// Drop stale events at a frame boundary (`Simulatable::step`'s twin of
    /// the clear in `step_frame`).
    pub(super) fn events_clear(&mut self) {
        self.events.clear();
    }

    /// Serialize the full world. Byte-equality of two snapshots is the
    /// determinism oracle the goldens assert. JSON keeps snapshots
    /// text-inspectable (LLM-native); if size ever matters the encoding can
    /// swap without touching callers.
    pub fn snapshot(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("physics world state is always serializable")
    }

    /// Restore a snapshot taken by [`World::snapshot`], resuming the simulation
    /// bit-exact from that frame.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), serde_json::Error> {
        *self = serde_json::from_slice(bytes)?;
        Ok(())
    }

    /// Live pose of a declared body: `(position, rotation-quaternion-xyzw)`.
    pub fn body_transform(&self, tag: &str) -> Option<([f32; 3], [f32; 4])> {
        let (rb_handle, _) = self.tags.get(tag)?;
        let rb = self.bodies.get(*rb_handle)?;
        let pos = rb.translation();
        let rot = rb.rotation();
        Some(([pos.x, pos.y, pos.z], [rot.x, rot.y, rot.z, rot.w]))
    }

    /// Live linear velocity of a declared body.
    pub fn body_velocity(&self, tag: &str) -> Option<[f32; 3]> {
        let (rb_handle, _) = self.tags.get(tag)?;
        let rb = self.bodies.get(*rb_handle)?;
        let v = rb.linvel();
        Some([v.x, v.y, v.z])
    }

    /// Cast a ray against the live world (docs/physics.md Phase 4): the
    /// nearest hit's tag, world-space point, surface normal, and distance.
    /// `dir` need not be normalized (it is here, so `max_dist` is in world
    /// units); a zero direction yields `None`.
    ///
    /// Answers against the broad phase **as of the last step** — a world that
    /// has never stepped misses everything (the MLE drivers hold deferred
    /// queries until a frame that actually substepped, so game code only sees
    /// answers from a simulated world; direct callers should step first).
    /// Sensor colliders are hittable like any other (rapier's default query
    /// filter) — an invisible trigger volume can occlude a solid body behind
    /// it.
    pub fn raycast(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<RayHit> {
        let d = vec3(dir);
        let len = d.length();
        if !(len.is_finite() && len > 0.0) {
            return None;
        }
        let ray = rapier3d::prelude::Ray::new(vec3(origin), d / len);
        let pipeline = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            QueryFilter::default(),
        );
        let (col_handle, hit) = pipeline.cast_ray_and_get_normal(&ray, max_dist, true)?;
        // Reverse handle→tag lookup: scenes are small; a scan beats carrying
        // a second map in every snapshot.
        let tag = self
            .tags
            .iter()
            .find(|(_, &(_, col))| col == col_handle)
            .map(|(tag, _)| tag.clone())?;
        let point = ray.origin + ray.dir * hit.time_of_impact;
        Some(RayHit {
            tag,
            position: [point.x, point.y, point.z],
            normal: [hit.normal.x, hit.normal.y, hit.normal.z],
            distance: hit.time_of_impact,
        })
    }

    /// The world as colored wireframe line segments, via Rapier's own debug
    /// renderer (docs/physics.md, "Debug visualization"): collider shapes,
    /// rigid-body frames, joints, and contacts. Render-only — reads the world,
    /// never steps it — and plain data, so it works for a GL overlay and for
    /// headless/text inspection alike.
    pub fn debug_lines(&self) -> Vec<DebugLine> {
        struct Collect(Vec<DebugLine>);
        impl DebugRenderBackend for Collect {
            fn draw_line(
                &mut self,
                _object: DebugRenderObject,
                a: Vector,
                b: Vector,
                color: DebugColor,
            ) {
                self.0.push(DebugLine {
                    a: [a.x, a.y, a.z],
                    b: [b.x, b.y, b.z],
                    color: hsla_to_rgba(color),
                });
            }
        }

        let mut collect = Collect(Vec::new());
        // The pipeline is scratch state (like PhysicsPipeline); a debug-only
        // path can afford to rebuild it per call. Rapier's default mode omits
        // CONTACTS — add it back, since touching-points are exactly what a
        // physics debug view is for. (This is rapier's DebugRenderMode, not
        // the crate's shading enum of the same name.)
        DebugRenderPipeline::new(
            DebugRenderStyle::default(),
            DebugRenderMode::default() | DebugRenderMode::CONTACTS,
        )
        .render(
            &mut collect,
            &self.bodies,
            &self.colliders,
            &self.impulse_joints,
            &self.multibody_joints,
            &self.narrow_phase,
        );
        collect.0
    }

    /// Headless observability (docs/physics.md, "drivable and observable
    /// headlessly"): the world as JSON — frame counter plus per-tag transform,
    /// velocity, and sleep state, in tag order.
    pub fn dump(&self) -> String {
        #[derive(Serialize)]
        struct BodyDump<'a> {
            tag: &'a str,
            position: [f32; 3],
            rotation: [f32; 4],
            velocity: [f32; 3],
            sleeping: bool,
        }
        #[derive(Serialize)]
        struct WorldDump<'a> {
            frame: u64,
            bodies: Vec<BodyDump<'a>>,
        }

        let bodies = self
            .tags
            .iter()
            .filter_map(|(tag, (rb_handle, _))| {
                let rb = self.bodies.get(*rb_handle)?;
                let pos = rb.translation();
                let rot = rb.rotation();
                let vel = rb.linvel();
                Some(BodyDump {
                    tag,
                    position: [pos.x, pos.y, pos.z],
                    rotation: [rot.x, rot.y, rot.z, rot.w],
                    velocity: [vel.x, vel.y, vel.z],
                    sleeping: rb.is_sleeping(),
                })
            })
            .collect();

        serde_json::to_string(&WorldDump {
            frame: self.frame,
            bodies,
        })
        .expect("physics dump is always serializable")
    }

    /// Apply every queued command to the live world (call after reconcile,
    /// before the frame's substeps). Unknown tags become warnings — the body
    /// may have despawned since the command was issued, which is a race the
    /// game can't avoid, so it must not be fatal.
    pub(super) fn apply_pending(&mut self) {
        for command in std::mem::take(&mut self.pending) {
            let (tag, kind) = command.tag_and_kind();
            let (tag, kind) = (tag.to_string(), kind);
            let Some(&(rb_handle, _)) = self.tags.get(tag.as_str()) else {
                self.push_command_warning(format!(
                    "physics {kind} for unknown tag \"{tag}\""
                ));
                continue;
            };
            // Rapier silently ignores impulses/forces/velocities on
            // non-dynamic bodies — warn instead, matching the unknown-tag
            // contract. (Teleport is meaningful for every kind.)
            let is_dynamic = self.bodies[rb_handle].is_dynamic();
            if !matches!(command, PhysicsCommand::Teleport { .. }) && !is_dynamic {
                self.push_command_warning(format!(
                    "physics {kind} on non-dynamic body \"{tag}\" has no effect"
                ));
                continue;
            }
            let rb = &mut self.bodies[rb_handle];
            match &command {
                PhysicsCommand::ApplyImpulse { impulse, .. } => {
                    rb.apply_impulse(vec3(*impulse), true);
                }
                PhysicsCommand::ApplyForce { force, .. } => {
                    // The force persists for ALL of this frame's substeps and
                    // clears after — so a multi-substep hitch frame integrates
                    // it longer than a normal frame. Deliberate: "one frame of
                    // push", matching how the game experiences frames.
                    rb.add_force(vec3(*force), true);
                    self.forced.push(rb_handle);
                }
                PhysicsCommand::SetVelocity { velocity, .. } => {
                    rb.set_linvel(vec3(*velocity), true);
                }
                PhysicsCommand::Teleport { position, .. } => {
                    let rotation = *rb.rotation();
                    rb.set_position(Pose::from_parts(vec3(*position), rotation), true);
                }
            }
        }
    }

    /// Forces last exactly one stepped frame (see [`PhysicsCommand::ApplyForce`]).
    pub(super) fn clear_frame_forces(&mut self) {
        for handle in std::mem::take(&mut self.forced) {
            if let Some(rb) = self.bodies.get_mut(handle) {
                rb.reset_forces(false);
            }
        }
    }

    // ── reconcile internals ─────────────────────────────────────────────

    fn spawn(&mut self, body: &Body) {
        let builder = match body.kind {
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
            BodyKind::Fixed => RigidBodyBuilder::fixed(),
        };
        let rb = builder
            .pose(pose_of(body))
            .linvel(vec3(body.velocity))
            .build();

        let mut collider = collider_of(&body.shape)
            .friction(body.friction)
            .restitution(body.restitution)
            .sensor(body.sensor)
            // Every body reports contact begin/end (Physics.events); rapier
            // only pays for this when a pair's state actually changes.
            .active_events(ActiveEvents::COLLISION_EVENTS);
        if let Some(mass) = body.mass {
            collider = collider.mass(mass);
        }

        let rb_handle = self.bodies.insert(rb);
        let col_handle =
            self.colliders
                .insert_with_parent(collider.build(), rb_handle, &mut self.bodies);
        self.tags.insert(body.tag.clone(), (rb_handle, col_handle));
    }

    fn despawn(&mut self, tag: &str) {
        if let Some((rb_handle, _)) = self.tags.remove(tag) {
            // Removes attached colliders and joints too.
            self.bodies.remove(
                rb_handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
        }
        self.declared.remove(tag);
    }

    /// The declaration for an existing tag changed: write exactly the changed
    /// fields into the live body (docs/physics.md, Authority + divergence).
    /// Structural changes (kind/shape/mass) rebuild the body wholesale — the
    /// tag is the identity, not the handle — but the rebuilt body keeps its
    /// *live* pose/velocities for every field whose declaration didn't change:
    /// a mass change on a falling crate must not teleport it back to its
    /// declared spawn pose.
    fn apply_divergence(&mut self, prev: &Body, next: &Body) {
        if prev.kind != next.kind || prev.shape != next.shape || prev.mass != next.mass {
            let live = self.tags.get(&next.tag).and_then(|(rb, _)| {
                let rb = self.bodies.get(*rb)?;
                Some((*rb.position(), rb.linvel(), rb.angvel()))
            });
            self.despawn(&next.tag);
            self.spawn(next);
            if let (Some((pose, linvel, angvel)), Some((rb_handle, _))) =
                (live, self.tags.get(&next.tag).copied())
            {
                let rb = &mut self.bodies[rb_handle];
                if prev.position == next.position && prev.rotation == next.rotation {
                    rb.set_position(pose, true);
                }
                if prev.velocity == next.velocity {
                    rb.set_linvel(linvel, true);
                }
                // Angular velocity is never declarable — always physics-owned.
                rb.set_angvel(angvel, true);
            }
            return;
        }

        let (rb_handle, col_handle) = self.tags[&next.tag];

        if prev.position != next.position || prev.rotation != next.rotation {
            let pose = pose_of(next);
            let rb = &mut self.bodies[rb_handle];
            // Skip the write when the declaration merely caught up with the
            // simulation (the `Physics.synced` steady state: the model
            // re-declares last frame's physics output). The values would be a
            // no-op, but `wake_up = true` isn't — writing would keep every
            // synced body permanently awake.
            if *rb.position() != pose {
                match next.kind {
                    // Kinematic bodies are *driven*: the next pose is a target
                    // the solver moves them to over the step (so they carry
                    // velocity into contacts) rather than an instant teleport.
                    BodyKind::Kinematic => rb.set_next_kinematic_position(pose),
                    _ => rb.set_position(pose, true),
                }
            }
        }
        if prev.velocity != next.velocity {
            let rb = &mut self.bodies[rb_handle];
            let v = vec3(next.velocity);
            if rb.linvel() != v {
                rb.set_linvel(v, true);
            }
        }
        if prev.friction != next.friction {
            self.colliders[col_handle].set_friction(next.friction);
        }
        if prev.restitution != next.restitution {
            self.colliders[col_handle].set_restitution(next.restitution);
        }
        if prev.sensor != next.sensor {
            self.colliders[col_handle].set_sensor(next.sensor);
        }
        // `authority` is inert in Phase 1: cached (by reconcile) but never
        // written to the live world.
    }
}

fn vec3(v: [f32; 3]) -> Vector {
    Vector::new(v[0], v[1], v[2])
}

fn pose_of(body: &Body) -> Pose {
    // Normalize the declared quaternion: `from_xyzw` doesn't, and a degenerate
    // rotation (all zeros, or junk off the future JSON boundary) would
    // NaN-poison the solver — which `snapshot` can't even flag (serde_json
    // writes non-finite floats as `null`, so the failure surfaces at `restore`,
    // far from the cause).
    let q = Rotation::from_xyzw(
        body.rotation[0],
        body.rotation[1],
        body.rotation[2],
        body.rotation[3],
    );
    let rotation = if q.length_squared().is_finite() && q.length_squared() > f32::EPSILON {
        q.normalize()
    } else {
        Rotation::IDENTITY
    };
    Pose::from_parts(vec3(body.position), rotation)
}

fn collider_of(shape: &Shape) -> ColliderBuilder {
    match *shape {
        Shape::Cuboid { extents } => {
            ColliderBuilder::cuboid(extents[0] / 2.0, extents[1] / 2.0, extents[2] / 2.0)
        }
        Shape::Sphere { radius } => ColliderBuilder::ball(radius),
        Shape::Capsule {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(half_height, radius),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground() -> Body {
        Body::fixed(
            "ground".to_string(),
            Shape::Cuboid {
                extents: [20.0, 0.2, 20.0],
            },
        )
    }

    fn crate_at(tag: &str, position: [f32; 3]) -> Body {
        Body::dynamic(
            tag.to_string(),
            Shape::Cuboid {
                extents: [1.0, 1.0, 1.0],
            },
        )
        .at(position)
    }

    fn scene(bodies: Vec<Body>) -> PhysicsScene {
        PhysicsScene::create([0.0, -9.81, 0.0], bodies)
    }

    #[test]
    fn reconcile_spawns_and_despawns_by_tag() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![ground(), crate_at("a", [0.0, 3.0, 0.0])]));
        assert!(w.body_transform("ground").is_some());
        assert!(w.body_transform("a").is_some());

        w.reconcile(&scene(vec![ground()]));
        assert!(w.body_transform("a").is_none());
        assert!(w.body_transform("ground").is_some());
    }

    #[test]
    fn duplicate_tags_keep_first_occurrence() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![
            crate_at("a", [0.0, 1.0, 0.0]),
            crate_at("a", [9.0, 9.0, 9.0]),
        ]));
        let (pos, _) = w.body_transform("a").unwrap();
        assert_eq!(pos, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn unchanged_declaration_leaves_the_simulation_alone() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        let s = scene(vec![crate_at("a", [0.0, 5.0, 0.0])]);
        w.reconcile(&s);
        for _ in 0..30 {
            w.step_fixed();
        }
        let (fallen, _) = w.body_transform("a").unwrap();
        assert!(fallen[1] < 5.0, "body should have fallen, got {fallen:?}");

        // Re-declaring the same scene must not snap the body back to its
        // declared spawn position — the declaration didn't change.
        w.reconcile(&s);
        let (after, _) = w.body_transform("a").unwrap();
        assert_eq!(after, fallen);
    }

    #[test]
    fn changed_declaration_teleports() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        for _ in 0..30 {
            w.step_fixed();
        }
        w.reconcile(&scene(vec![crate_at("a", [2.0, 10.0, 0.0])]));
        let (pos, _) = w.body_transform("a").unwrap();
        assert_eq!(pos, [2.0, 10.0, 0.0]);
    }

    #[test]
    fn velocity_change_applies_without_teleporting() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        let body = crate_at("a", [0.0, 5.0, 0.0]);
        w.reconcile(&scene(vec![body.clone()]));
        w.reconcile(&scene(vec![body.with_velocity([3.0, 0.0, 0.0])]));
        let (pos, _) = w.body_transform("a").unwrap();
        assert_eq!(pos, [0.0, 5.0, 0.0]);
        assert_eq!(w.body_velocity("a").unwrap(), [3.0, 0.0, 0.0]);
    }

    #[test]
    fn structural_rebuild_keeps_live_state_for_unchanged_fields() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        let falling = crate_at("a", [0.0, 5.0, 0.0]);
        w.reconcile(&scene(vec![falling.clone()]));
        for _ in 0..30 {
            w.step_fixed();
        }
        let (fallen, _) = w.body_transform("a").unwrap();
        let vel = w.body_velocity("a").unwrap();
        assert!(fallen[1] < 5.0 && vel[1] < 0.0);

        // Change ONLY the mass (a structural rebuild). Position/velocity
        // declarations are unchanged, so the rebuilt body must keep its live
        // fallen pose and momentum — not snap back to the declared spawn.
        w.reconcile(&scene(vec![falling.with_mass(10.0)]));
        let (pos, _) = w.body_transform("a").unwrap();
        assert_eq!(pos, fallen);
        assert_eq!(w.body_velocity("a").unwrap(), vel);
    }

    #[test]
    fn degenerate_declared_rotation_falls_back_to_identity() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![
            crate_at("a", [0.0, 1.0, 0.0]).facing([0.0, 0.0, 0.0, 0.0])
        ]));
        w.step_fixed();
        let (_, rot) = w.body_transform("a").unwrap();
        assert_eq!(rot, [0.0, 0.0, 0.0, 1.0]);
        // And the world is still snapshot/restore-clean (no NaN poisoning).
        let snap = w.snapshot();
        let mut r = World::new([0.0, 0.0, 0.0]);
        r.restore(&snap).unwrap();
    }

    #[test]
    fn redeclaring_the_live_pose_does_not_wake_a_sleeping_body() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![ground(), crate_at("a", [0.0, 0.7, 0.0])]));
        // Let the crate settle and fall asleep.
        for _ in 0..300 {
            w.step_fixed();
        }
        let dump: serde_json::Value = serde_json::from_str(&w.dump()).unwrap();
        assert_eq!(
            dump["bodies"][0]["sleeping"], true,
            "crate should be asleep: {dump}"
        );

        // Declare the body at its *live* rest pose (a changed declaration — the
        // synced steady state). The write must be skipped so the body sleeps on.
        let (rest, rot) = w.body_transform("a").unwrap();
        w.reconcile(&scene(vec![
            ground(),
            crate_at("a", rest).facing(rot).with_velocity([0.0, 0.0, 0.0]),
        ]));
        w.step_fixed();
        let dump: serde_json::Value = serde_json::from_str(&w.dump()).unwrap();
        assert_eq!(dump["bodies"][0]["sleeping"], true, "body was woken");
    }

    #[test]
    fn reconcile_is_invariant_to_declaration_order() {
        let mut x = World::new([0.0, -9.81, 0.0]);
        let mut y = World::new([0.0, -9.81, 0.0]);
        let (a, b) = (crate_at("a", [0.0, 2.0, 0.0]), crate_at("b", [0.5, 3.0, 0.0]));
        x.reconcile(&scene(vec![a.clone(), b.clone()]));
        y.reconcile(&scene(vec![b, a]));
        for _ in 0..30 {
            x.step_fixed();
            y.step_fixed();
        }
        assert!(
            x.snapshot() == y.snapshot(),
            "handle history depends on declaration order"
        );
    }

    #[test]
    fn kind_change_rebuilds_the_body() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        // Flip to fixed: the body must stop integrating.
        let fixed = Body::fixed(
            "a".to_string(),
            Shape::Cuboid {
                extents: [1.0, 1.0, 1.0],
            },
        )
        .at([0.0, 5.0, 0.0]);
        w.reconcile(&scene(vec![fixed]));
        for _ in 0..30 {
            w.step_fixed();
        }
        let (pos, _) = w.body_transform("a").unwrap();
        assert_eq!(pos, [0.0, 5.0, 0.0]);
    }

    #[test]
    fn accumulator_carries_the_remainder() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        assert_eq!(w.step_frame(FIXED_DT * 1.5), 1);
        // 0.5 dt left over; another 1.5 makes 2.0 → two steps.
        assert_eq!(w.step_frame(FIXED_DT * 1.5), 2);
        assert_eq!(w.frame(), 3);
    }

    #[test]
    fn substeps_are_capped_and_backlog_dropped() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        assert_eq!(w.step_frame(1.0), MAX_SUBSTEPS_PER_FRAME);
        // Backlog was dropped: a normal frame takes exactly one substep again.
        assert_eq!(w.step_frame(FIXED_DT), 1);
    }

    #[test]
    fn snapshot_restore_round_trips_byte_exact() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![ground(), crate_at("a", [0.0, 3.0, 0.0])]));
        for _ in 0..20 {
            w.step_fixed();
        }
        let snap = w.snapshot();

        let mut restored = World::new([0.0, 0.0, 0.0]);
        restored.restore(&snap).unwrap();
        assert_eq!(restored.frame(), w.frame());
        assert!(restored.snapshot() == snap, "restored snapshot differs");
    }

    #[test]
    fn commands_apply_at_the_frames_first_substep() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        w.queue_command(PhysicsCommand::ApplyImpulse {
            tag: "a".to_string(),
            impulse: [1.0, 0.0, 0.0],
        });
        // Zero-substep frame: the command stays pending, nothing applied.
        assert_eq!(w.step_frame(FIXED_DT / 4.0), 0);
        assert_eq!(w.body_velocity("a").unwrap(), [0.0, 0.0, 0.0]);
        // The next stepping frame consumes it (default cube mass = 1: dv = 1).
        assert_eq!(w.step_frame(FIXED_DT), 1);
        assert!(w.body_velocity("a").unwrap()[0] > 0.0);
        assert!(w.take_command_warnings().is_empty());
    }

    #[test]
    fn set_velocity_and_teleport_commands_apply() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        w.queue_command(PhysicsCommand::SetVelocity {
            tag: "a".to_string(),
            velocity: [0.0, 0.0, 2.0],
        });
        w.queue_command(PhysicsCommand::Teleport {
            tag: "a".to_string(),
            position: [3.0, 1.0, 0.0],
        });
        w.step_frame(FIXED_DT);
        let (pos, _) = w.body_transform("a").unwrap();
        // Teleported, then integrated one 1/60 step of vz = 2.
        assert_eq!(pos[0], 3.0);
        assert!((pos[2] - 2.0 * FIXED_DT).abs() < 1e-5);
        // Teleport did NOT touch the declaration: re-declaring the original
        // spawn pose is unchanged → no snap-back (the divergence rule).
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        let (after, _) = w.body_transform("a").unwrap();
        assert_eq!(after[0], 3.0);
    }

    #[test]
    fn forces_last_exactly_one_stepped_frame() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        w.queue_command(PhysicsCommand::ApplyForce {
            tag: "a".to_string(),
            force: [6.0, 0.0, 0.0],
        });
        w.step_frame(FIXED_DT);
        let v1 = w.body_velocity("a").unwrap()[0];
        assert!(v1 > 0.0, "force should have accelerated the body");
        // Next frame: the force is gone; velocity holds (no gravity/friction
        // in the air).
        w.step_frame(FIXED_DT);
        let v2 = w.body_velocity("a").unwrap()[0];
        assert!((v2 - v1).abs() < 1e-6, "force leaked into the next frame");
    }

    #[test]
    fn command_for_unknown_tag_warns_instead_of_failing() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        w.queue_command(PhysicsCommand::ApplyImpulse {
            tag: "ghost".to_string(),
            impulse: [1.0, 0.0, 0.0],
        });
        w.step_frame(FIXED_DT);
        let warnings = w.take_command_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ghost"), "{warnings:?}");
        // Drained: asking again yields nothing.
        assert!(w.take_command_warnings().is_empty());
    }

    #[test]
    fn pending_commands_survive_a_snapshot() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        w.queue_command(PhysicsCommand::ApplyImpulse {
            tag: "a".to_string(),
            impulse: [1.0, 0.0, 0.0],
        });
        let snap = w.snapshot();
        let mut restored = World::new([0.0, 0.0, 0.0]);
        restored.restore(&snap).unwrap();
        w.step_frame(FIXED_DT);
        restored.step_frame(FIXED_DT);
        assert!(
            w.snapshot() == restored.snapshot(),
            "pending command lost across snapshot/restore"
        );
        assert!(restored.body_velocity("a").unwrap()[0] > 0.0);
    }

    #[test]
    fn command_queue_overflow_drops_with_warning() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        for _ in 0..=MAX_PENDING_COMMANDS {
            w.queue_command(PhysicsCommand::ApplyImpulse {
                tag: "a".to_string(),
                impulse: [0.0, 0.0, 0.0],
            });
        }
        let warnings = w.take_command_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("queue full"), "{warnings:?}");
    }

    #[test]
    fn command_warnings_are_bounded_when_never_drained() {
        // A driver that never steps/drains (e.g. no `physics` hook) must not
        // leak: both the queue AND the warning buffer are capped.
        let mut w = World::new([0.0, 0.0, 0.0]);
        for _ in 0..(MAX_PENDING_COMMANDS + MAX_COMMAND_WARNINGS + 100) {
            w.queue_command(PhysicsCommand::ApplyImpulse {
                tag: "a".to_string(),
                impulse: [0.0, 0.0, 0.0],
            });
        }
        let warnings = w.take_command_warnings();
        assert_eq!(warnings.len(), MAX_COMMAND_WARNINGS + 1);
        assert!(warnings.last().unwrap().contains("suppressed"), "{warnings:?}");
    }

    #[test]
    fn command_on_non_dynamic_body_warns() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![ground()]));
        w.queue_command(PhysicsCommand::ApplyImpulse {
            tag: "ground".to_string(),
            impulse: [0.0, 9.0, 0.0],
        });
        // Teleport IS meaningful for a fixed body: no warning for it.
        w.queue_command(PhysicsCommand::Teleport {
            tag: "ground".to_string(),
            position: [0.0, -0.2, 0.0],
        });
        w.step_frame(FIXED_DT);
        let warnings = w.take_command_warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("non-dynamic"), "{warnings:?}");
    }

    #[test]
    fn contact_events_report_tags_and_transitions() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![ground(), crate_at("a", [0.0, 2.0, 0.0])]));
        // Fall until contact: a Started event naming both tags appears.
        let mut started = None;
        for _ in 0..120 {
            w.step_frame(FIXED_DT);
            let events = w.take_events();
            if let Some(e) = events.iter().find(|e| e.started) {
                started = Some(e.clone());
                break;
            }
        }
        let e = started.expect("the crate should land");
        let pair = [e.a.as_str(), e.b.as_str()];
        assert!(pair.contains(&"a") && pair.contains(&"ground"), "{e:?}");
        assert!(!e.sensor);

        // Undrained events are dropped at the next frame's start: launch the
        // crate (a Stopped event fires), don't drain it, and step a further
        // airborne frame — the stale Stopped must be gone, and a mid-air
        // frame produces no fresh transitions.
        w.queue_command(PhysicsCommand::ApplyImpulse {
            tag: "a".to_string(),
            impulse: [0.0, 8.0, 0.0],
        });
        w.step_frame(FIXED_DT); // launch (separation may register this frame…)
        w.step_frame(FIXED_DT); // …or this one; both deliberately undrained
        w.step_frame(FIXED_DT); // fully airborne: boundary cleared the stale ones,
        assert!(w.take_events().is_empty(), "stale events survived the frame boundary");
    }

    #[test]
    fn sensor_overlaps_are_flagged() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        let zone = Body::fixed(
            "zone".to_string(),
            Shape::Cuboid {
                extents: [4.0, 4.0, 4.0],
            },
        )
        .at([0.0, 2.0, 0.0])
        .as_sensor();
        w.reconcile(&scene(vec![zone, crate_at("a", [0.0, 6.0, 0.0])]));
        let mut hit = None;
        for _ in 0..180 {
            w.step_frame(FIXED_DT);
            if let Some(e) = w.take_events().into_iter().find(|e| e.started) {
                hit = Some(e);
                break;
            }
        }
        let e = hit.expect("the crate should enter the zone");
        assert!(e.sensor, "{e:?}");
    }

    #[test]
    fn raycast_hits_the_nearest_body_with_tag_point_and_normal() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        // The crate floats clear of the ground in a ZERO-gravity scene (the
        // shared `scene()` helper declares -9.81, which would sag it a step),
        // so the asserted geometry is exact: center 0.7, top face at 1.2.
        w.reconcile(&PhysicsScene::create(
            [0.0, 0.0, 0.0],
            vec![ground(), crate_at("a", [0.0, 0.7, 0.0])],
        ));
        // The broad phase ingests colliders at the step — mirror the real
        // query path, which always runs post-step.
        w.step_fixed();
        // Straight down from above: the crate's top face wins over the ground
        // beneath it.
        let hit = w.raycast([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], 100.0).unwrap();
        assert_eq!(hit.tag, "a");
        assert!((hit.position[1] - 1.2).abs() < 1e-4, "{:?}", hit.position);
        assert!((hit.normal[1] - 1.0).abs() < 1e-4, "{:?}", hit.normal);
        assert!((hit.distance - 3.8).abs() < 1e-4, "{}", hit.distance);
        // Direction need not be normalized: distance stays in world units.
        let hit2 = w.raycast([0.0, 5.0, 0.0], [0.0, -7.0, 0.0], 100.0).unwrap();
        assert!((hit2.distance - 3.8).abs() < 1e-4);
        // Beyond max distance, zero direction: misses. Off to the side: the
        // ground.
        assert!(w.raycast([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], 3.0).is_none());
        assert!(w.raycast([0.0, 5.0, 0.0], [0.0, 0.0, 0.0], 10.0).is_none());
        let g = w.raycast([5.0, 5.0, 5.0], [0.0, -1.0, 0.0], 100.0).unwrap();
        assert_eq!(g.tag, "ground");
    }

    #[test]
    fn hsla_converts_to_expected_rgba() {
        // Pure green, and rapier's default dynamic-collider crimson.
        assert_eq!(hsla_to_rgba([120.0, 1.0, 0.5, 1.0]), [0.0, 1.0, 0.0, 1.0]);
        let [r, g, b, a] = hsla_to_rgba([340.0, 1.0, 0.3, 1.0]);
        assert!(r > 0.55 && g == 0.0 && b > 0.15 && a == 1.0, "{r} {g} {b}");
        // Negative hue lands in [0,360) instead of producing negative chroma.
        let [r, ..] = hsla_to_rgba([-30.0, 1.0, 0.5, 1.0]);
        assert!((0.0..=1.0).contains(&r));
    }

    #[test]
    fn debug_lines_cover_declared_bodies() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        assert!(w.debug_lines().is_empty());
        w.reconcile(&scene(vec![ground(), crate_at("a", [0.0, 0.51, 0.0])]));
        for _ in 0..30 {
            w.step_fixed();
        }
        let lines = w.debug_lines();
        // Two cuboid colliders' worth of wireframe + body axes + the resting
        // contact — the exact count is rapier's business; non-empty and finite
        // is the contract.
        assert!(lines.len() > 20, "expected a real wireframe, got {}", lines.len());
        for line in &lines {
            assert!(line.a.iter().chain(&line.b).all(|v| v.is_finite()));
            assert!(line.color.iter().all(|c| (0.0..=1.0).contains(c)));
        }
    }

    #[test]
    fn dump_lists_bodies_in_tag_order() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![
            crate_at("b", [0.0, 1.0, 0.0]),
            crate_at("a", [0.0, 2.0, 0.0]),
        ]));
        let dump: serde_json::Value = serde_json::from_str(&w.dump()).unwrap();
        let tags: Vec<&str> = dump["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["tag"].as_str().unwrap())
            .collect();
        assert_eq!(tags, vec!["a", "b"]);
        assert_eq!(dump["frame"], 0);
    }
}
