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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rapier3d::control::{CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::{NonlinearRigidMotion, ShapeCastHit, ShapeCastOptions, ShapeCastStatus};
use rapier3d::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Blast, Body, BodyKind, CharacterSettings, Falloff, ForceVolume, PhysicsScene, Shape};
use super::room::{self, RoomMeshes};
use crate::fused_mesh::ChunkKey;
use crate::timetravel::fnv1a;

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
    /// A force applied for one fixed step (cleared after — no rapier force
    /// persistence to forget about). Under the recorded drive each substep is
    /// a recorded frame, so a force lasts exactly one fixed step.
    ApplyForce { tag: String, force: [f32; 3] },
    SetVelocity { tag: String, velocity: [f32; 3] },
    /// Replace a body's HORIZONTAL velocity, leaving `y` exactly as the world
    /// holds it at apply time.
    ///
    /// The character-controller case: steering owns the horizontal plane and
    /// the solver keeps the vertical axis it is using to resolve the ground
    /// contact, so the game never has to author a `y` it has no opinion about.
    /// The write is exactly transparent on `y` — see
    /// `masked_writes_are_transparent_on_the_untouched_axis`.
    ///
    /// The untouched axis is read from the LIVE body, so several velocity
    /// commands in one frame compose: they apply in queue order, the LAST
    /// write to a given axis wins, and an axis nobody wrote is untouched.
    SetVelocityXZ { tag: String, x: f32, z: f32 },
    /// Replace a body's VERTICAL velocity, leaving the horizontal plane
    /// exactly as the world holds it at apply time — a jump that keeps the
    /// run. Composes with [`PhysicsCommand::SetVelocityXZ`] by the same
    /// last-write-wins-per-axis rule; together the two cover all three axes.
    SetVelocityY { tag: String, y: f32 },
    /// Move the live body without touching its declaration (the declared
    /// cache is unchanged, so the next frame's unchanged declaration does not
    /// snap it back).
    Teleport { tag: String, position: [f32; 3] },
    /// Walk a `Physics.character` by up to `translation` (metres) this step:
    /// collide and slide, climb slopes up to its limit, step up ledges and
    /// follow the ground down. Moves queued for one step chain.
    Move { tag: String, translation: [f32; 3] },
    /// A radial impulse on every dynamic body in range (`Physics.explode`).
    /// Names no body, so it never warns about a tag.
    Explode(Blast),
}

impl PhysicsCommand {
    /// The command's tag and short kind name, for warnings and logs.
    pub fn tag_and_kind(&self) -> (&str, &'static str) {
        match self {
            PhysicsCommand::ApplyImpulse { tag, .. } => (tag, "applyImpulse"),
            PhysicsCommand::ApplyForce { tag, .. } => (tag, "applyForce"),
            PhysicsCommand::SetVelocity { tag, .. } => (tag, "setVelocity"),
            PhysicsCommand::SetVelocityXZ { tag, .. } => (tag, "setVelocityXZ"),
            PhysicsCommand::SetVelocityY { tag, .. } => (tag, "setVelocityY"),
            PhysicsCommand::Teleport { tag, .. } => (tag, "teleport"),
            PhysicsCommand::Move { tag, .. } => (tag, "move"),
            PhysicsCommand::Explode(_) => ("", "explode"),
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
    /// The hit triangle's index in the `Physics.trimesh` source data; `None`
    /// for any other shape.
    pub triangle: Option<u32>,
}

/// Which bodies a [`World::sweep`] can hit. Sensors never.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepFilter {
    /// Fixed bodies only: a held prop stops at the level, not at other props.
    Fixed,
    /// Fixed and dynamic bodies.
    NotKinematic,
    All,
}

/// A rigid pose: position and unit rotation `[x, y, z, w]`.
pub type RigidPose = ([f32; 3], [f32; 4]);

/// The first hit of a [`World::sweep`].
#[derive(Debug, Clone, PartialEq)]
pub struct SweepHit {
    pub tag: String,
    /// How far along the motion, in [0, 1]: the shape touches at the pose
    /// interpolated this far from `from` toward `to`.
    pub fraction: f32,
    /// The contact point, on the hit body.
    pub position: [f32; 3],
    /// The hit surface's outward normal.
    pub normal: [f32; 3],
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

/// What the last [`World::reconcile`] did: the counters behind
/// `Physics.memo`'s claim that an unchanged group costs O(1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileStats {
    /// Declarations compared with the cache: flat bodies plus the bodies of
    /// groups that changed.
    pub compared: usize,
    /// Declarations written to the cache (spawns and changes).
    pub written: usize,
    /// Groups skipped because their `Arc` was the one already reconciled.
    pub groups_skipped: usize,
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
    /// tag → live body. `BTreeMap` so iteration (despawn order, dumps,
    /// serialization) is deterministic — a requirement, not a nicety. A body
    /// owns one collider, except a fused room, which owns one per chunk.
    ///
    /// `tags`, `rooms`, `declared` and `flat` are copy-on-write (`Arc` +
    /// `make_mut`): a checkpoint shares them, and the next write after one
    /// copies the map once. Keyframes of a scene whose declarations hold still
    /// share one copy.
    tags: Arc<BTreeMap<String, RigidBodyHandle>>,
    /// tag → a fused room body's colliders (`Shape::FusedRoom`).
    #[serde(default)]
    rooms: Arc<BTreeMap<String, LiveRoom>>,
    /// Colliders a re-fused room removed since the last step, with its tag:
    /// their contacts end at the next step, and still name the room.
    #[serde(default)]
    retired: Vec<(ColliderHandle, String)>,
    /// Last-declared body per tag — the memory behind the divergence rule:
    /// reconcile writes a field to the live body only when the new declaration
    /// *differs from this cache*; an unchanged declaration leaves the body to
    /// the simulation.
    declared: Arc<BTreeMap<String, Body>>,
    /// Tags the last reconcile saw declared flat (outside any group). With
    /// changed groups' members, the only tags a despawn scan visits.
    flat: Arc<BTreeSet<String>>,
    /// The last-reconciled `Physics.memo` group per key. A scene holding the
    /// same `Arc` again skips the group without visiting its bodies. World
    /// state, so restoring a checkpoint can't leave a stale fast path.
    groups: BTreeMap<String, Arc<[Body]>>,
    /// Fused fixed bodies, one compound per [`FuseKey`], in key order
    /// (copy-on-write, like `tags`). A fused body has a declaration but no
    /// rigid body: it is one part of a compound collider, so fixed–fixed
    /// pairs never reach rapier.
    #[serde(default)]
    fused: Arc<Vec<FusedStatics>>,
    /// Fused parts touching another body as of the last step, `(a, b,
    /// sensor)` with `a < b`: the per-tag contact state rapier keeps per
    /// collider pair, diffed each step into events.
    #[serde(default)]
    fused_touching: BTreeSet<(String, String, bool)>,
    /// Touching pairs of bodies that left a compound in the last reconcile:
    /// the game already heard them start, so the next step drops rapier's
    /// start for them and reports a stop only if they no longer touch.
    #[serde(default)]
    handoff: BTreeSet<(String, String, bool)>,
    /// Live poses of parts unfused by the reconcile in progress, so a body
    /// leaving a compound keeps its pose. Empty between reconciles.
    #[serde(skip, default)]
    unfused: BTreeMap<String, Pose>,
    accumulator: f32,
    frame: u64,
    /// Commands awaiting the next stepped frame (serialized: a snapshot taken
    /// between queue and step must restore them for bit-exact resume).
    #[serde(default)]
    pending: Vec<PhysicsCommand>,
    /// Per character: whether its last move ended on walkable ground. World
    /// state, so snapshots, rewinds and replays restore it.
    #[serde(default)]
    grounded: BTreeMap<String, bool>,
    /// The declared force volumes, applied every step (world state, so a
    /// degraded frame keeps them and a seek restores them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<ForceVolume>,
    /// Boxes `(center, size)` of volumes added, changed or removed since the
    /// last step: it wakes the sleeping bodies in them (a fan switching on or
    /// off). Otherwise a body that settles in a draft too weak to move it
    /// sleeps like any other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    wake_boxes: Vec<([f32; 3], [f32; 3])>,
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
    #[serde(skip, default)]
    reconcile_stats: ReconcileStats,
    /// Not stepped since it was restored from recorded history or kept
    /// across a hot reload (see [`World::is_stale`]).
    #[serde(skip, default)]
    stale: bool,
}

impl Clone for World {
    fn clone(&self) -> Self {
        Self {
            gravity: self.gravity,
            integration_parameters: self.integration_parameters,
            pipeline: PhysicsPipeline::new(),
            islands: self.islands.clone(),
            broad_phase: self.broad_phase.clone(),
            narrow_phase: self.narrow_phase.clone(),
            bodies: self.bodies.clone(),
            colliders: self.colliders.clone(),
            impulse_joints: self.impulse_joints.clone(),
            multibody_joints: self.multibody_joints.clone(),
            ccd_solver: self.ccd_solver.clone(),
            tags: self.tags.clone(),
            rooms: self.rooms.clone(),
            retired: self.retired.clone(),
            declared: self.declared.clone(),
            flat: self.flat.clone(),
            groups: self.groups.clone(),
            fused: self.fused.clone(),
            fused_touching: self.fused_touching.clone(),
            handoff: self.handoff.clone(),
            unfused: self.unfused.clone(),
            accumulator: self.accumulator,
            frame: self.frame,
            pending: self.pending.clone(),
            grounded: self.grounded.clone(),
            volumes: self.volumes.clone(),
            wake_boxes: self.wake_boxes.clone(),
            forced: self.forced.clone(),
            events: self.events.clone(),
            command_warnings: self.command_warnings.clone(),
            reconcile_stats: self.reconcile_stats,
            stale: self.stale,
        }
    }
}

/// Cheap in-process physics checkpoint. Rapier's `SharedShape`, terrain
/// samples and the world's per-body maps (tags, declarations) remain
/// Arc-shared across timeline keyframes and ghost previews, avoiding repeated
/// multi-megabyte JSON snapshots on the frame thread. Rapier's own sets are
/// owned values, so they are copied.
#[derive(Clone)]
pub struct PhysicsSnapshot {
    world: Box<World>,
}

impl std::fmt::Debug for PhysicsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicsSnapshot")
            .field("frame", &self.world.frame)
            .field("bodies", &self.world.bodies.len())
            .field("colliders", &self.world.colliders.len())
            .finish()
    }
}

impl PartialEq for PhysicsSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.world.snapshot() == other.world.snapshot()
    }
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
            tags: Arc::default(),
            rooms: Arc::default(),
            retired: Vec::new(),
            declared: Arc::default(),
            flat: Arc::default(),
            groups: BTreeMap::new(),
            fused: Arc::default(),
            fused_touching: BTreeSet::new(),
            handoff: BTreeSet::new(),
            unfused: BTreeMap::new(),
            accumulator: 0.0,
            frame: 0,
            pending: Vec::new(),
            grounded: BTreeMap::new(),
            volumes: Vec::new(),
            wake_boxes: Vec::new(),
            forced: Vec::new(),
            events: Vec::new(),
            command_warnings: Vec::new(),
            reconcile_stats: ReconcileStats::default(),
            stale: false,
        }
    }

    pub fn checkpoint(&self) -> PhysicsSnapshot {
        PhysicsSnapshot {
            world: Box::new(self.clone()),
        }
    }

    pub fn restore_checkpoint(&mut self, snapshot: &PhysicsSnapshot) {
        *self = snapshot.world.as_ref().clone();
    }

    /// Mark this world as stale until its next step.
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Whether this world hasn't stepped since it was restored from recorded
    /// history (a seek, rewind or past-frame preview) or kept across a hot
    /// reload — what a scrubbed, rewound or paused `draw` reads. Its bodies
    /// are the ones declared back then, so a body the current code adds is
    /// missing for a reason, not a typo.
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// Queue a command for the next stepped frame (applied after that frame's
    /// reconcile, before its first substep). Problems surface later through
    /// [`World::take_command_warnings`].
    pub fn queue_command(&mut self, command: PhysicsCommand) {
        if self.pending.len() >= MAX_PENDING_COMMANDS {
            let (tag, kind) = command.tag_and_kind();
            let what = if tag.is_empty() { kind.to_string() } else { format!("{kind} \"{tag}\"") };
            self.push_command_warning(format!(
                "physics command queue full ({MAX_PENDING_COMMANDS}); dropping {what} \
                 (is anything stepping the world?)"
            ));
            return;
        }
        self.pending.push(command);
    }

    /// The queued commands, still awaiting the next stepped frame.
    pub fn pending_commands(&self) -> &[PhysicsCommand] {
        &self.pending
    }

    /// Drain the queued commands without applying them — the recorded drive
    /// (Phase 6) folds them into the frame's Timeline command list instead,
    /// so replay reproduces them.
    pub fn take_pending_commands(&mut self) -> Vec<PhysicsCommand> {
        std::mem::take(&mut self.pending)
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
    ///
    /// A group whose `Arc` was reconciled last time is skipped: its bodies
    /// would all compare equal. Only flat bodies and changed groups are
    /// visited. Still, solid, convex fixed bodies ([`fusable`]), flat or
    /// grouped, become parts of one compound collider per material and grid
    /// cell ([`FuseKey`]) instead of bodies; a compound is rebuilt only when
    /// its parts changed. Neither depends on grouping, so the rapier history
    /// matches declaring every body flat.
    pub fn reconcile(&mut self, scene: &PhysicsScene) {
        self.gravity = scene.gravity;
        if self.volumes != scene.volumes {
            let (old, new) = (&self.volumes, &scene.volumes);
            let changed = (old.iter().filter(|v| !new.contains(v)))
                .chain(new.iter().filter(|v| !old.contains(v)));
            self.wake_boxes.extend(changed.map(|v| (v.center, v.size)));
            self.volumes = scene.volumes.clone();
        }
        let mut stats = ReconcileStats::default();

        let mut wanted: BTreeMap<&String, &Body> = BTreeMap::new();
        for body in &scene.bodies {
            wanted.entry(&body.tag).or_insert(body); // first occurrence wins
        }
        let mut groups: BTreeMap<&str, &Arc<[Body]>> = BTreeMap::new();
        for group in &scene.groups {
            groups.entry(group.key.as_str()).or_insert(&group.bodies);
        }
        let reconciled = |key: &str, bodies: &Arc<[Body]>| {
            self.groups
                .get(key)
                .is_some_and(|old| Arc::ptr_eq(old, bodies))
        };
        let changed: Vec<&Arc<[Body]>> = groups
            .iter()
            .filter(|(key, bodies)| !reconciled(key, bodies))
            .map(|(_, bodies)| *bodies)
            .collect();
        // Last versions of groups that changed or went away.
        let retired: Vec<Arc<[Body]>> = self
            .groups
            .iter()
            .filter(|(key, old)| {
                !groups
                    .get(key.as_str())
                    .is_some_and(|b| Arc::ptr_eq(old, b))
            })
            .map(|(_, old)| old.clone())
            .collect();
        stats.groups_skipped = groups.len() - changed.len();

        let flat_unchanged = self.flat.len() == wanted.len()
            && self.flat.iter().zip(wanted.keys()).all(|(a, b)| a == *b);
        let next_flat =
            (!flat_unchanged).then(|| Arc::new(wanted.keys().map(|t| (*t).clone()).collect()));
        let old_flat = Arc::clone(&self.flat);

        // Declarations that may differ from the cache, in tag order: flat
        // bodies and changed groups' bodies. Tags are disjoint across them
        // (`Physics.scene` enforces it), so an unchanged group owns none of
        // the tags that went away.
        let mut updates = wanted;
        for bodies in &changed {
            for body in bodies.iter() {
                updates.entry(&body.tag).or_insert(body);
            }
        }
        stats.compared = updates.len();
        let gone: BTreeSet<&String> = old_flat
            .iter()
            .chain(
                retired
                    .iter()
                    .flat_map(|bodies| bodies.iter().map(|b| &b.tag)),
            )
            .filter(|tag| !updates.contains_key(*tag))
            .collect();

        // Changed declarations, and the keys whose compound they change
        // (a part added, edited, removed or turned into a body). Which bodies
        // fuse depends only on the declarations, never on how they were
        // grouped, so flat and memoized scenes build the same compounds.
        let part = |tag: &str| {
            self.declared
                .get(tag)
                .filter(|prev| fusable(prev) && !self.tags.contains_key(tag))
        };
        let mut dirty: BTreeSet<FuseKey> = BTreeSet::new();
        let mut bodies_changed: Vec<(&String, &Body)> = Vec::new();
        let mut joining: Vec<&String> = Vec::new();
        for (&tag, &body) in &updates {
            let fuse = fusable(body);
            if self.declared.get(tag) == Some(body) && !(fuse && self.tags.contains_key(tag)) {
                continue; // unchanged: the simulation owns this body
            }
            if let Some(prev) = part(tag) {
                dirty.insert(fuse_key(prev));
            }
            if !fuse {
                bodies_changed.push((tag, body));
                continue;
            }
            dirty.insert(fuse_key(body));
            if self.tags.contains_key(tag) {
                joining.push(tag);
            }
        }
        dirty.extend(gone.iter().filter_map(|tag| part(tag)).map(fuse_key));

        // Each dirty key's parts: all its fusable declarations, flat or
        // grouped, in tag order.
        let mut refuse: BTreeMap<FuseKey, Vec<&Body>> =
            dirty.iter().map(|m| (*m, Vec::new())).collect();
        if !dirty.is_empty() {
            let unchanged = groups
                .iter()
                .filter(|(key, bodies)| reconciled(key, bodies))
                .flat_map(|(_, bodies)| bodies.iter());
            for body in updates.values().copied().chain(unchanged).filter(|b| fusable(b)) {
                if let Some(parts) = refuse.get_mut(&fuse_key(body)) {
                    parts.push(body);
                }
            }
            for parts in refuse.values_mut() {
                parts.sort_by(|a, b| a.tag.cmp(&b.tag));
            }
        }

        // Unfuse, keeping each part's live pose for whatever it becomes.
        if !dirty.is_empty() {
            let (removed, kept): (Vec<_>, Vec<_>) = self
                .fused
                .iter()
                .cloned()
                .partition(|f| dirty.contains(&f.key));
            self.fused = Arc::new(kept);
            crate::frame_telemetry::note_bodies(0, removed.len());
            for fused in removed {
                let poses: Vec<Pose> = fused.parts.shapes().iter().map(|(pose, _)| *pose).collect();
                self.unfused.extend(fused.tags.into_iter().zip(poses));
                self.colliders
                    .remove(fused.collider, &mut self.islands, &mut self.bodies, true);
            }
        }
        // A re-fused part keeps its live pose where its declaration didn't
        // change, as a rebuilt body does.
        let part_poses: BTreeMap<&String, Pose> = refuse
            .values()
            .flatten()
            .map(|body| {
                let live = self.unfused.get(&body.tag).copied().or_else(|| {
                    self.tags
                        .get(&body.tag)
                        .map(|rb| *self.bodies[*rb].position())
                });
                let pose = match (self.declared.get(&body.tag), live) {
                    (Some(prev), Some(live)) => keep_live(prev, body, pose_of(body), live),
                    _ => pose_of(body),
                };
                (&body.tag, pose)
            })
            .collect();

        // A body becoming a part hands its contacts over, so the next step
        // reports only real transitions.
        let handed: Vec<_> = joining.iter().flat_map(|tag| self.touching_of(tag)).collect();
        self.fused_touching.extend(handed);

        // Despawns (bodies that went away or become parts), in tag order.
        let despawns: BTreeSet<&String> = gone.into_iter().chain(joining).collect();
        for tag in despawns {
            self.despawn(tag);
        }

        // Spawns / updates of bodies, in tag order.
        for (tag, body) in bodies_changed {
            let accepted = match self.declared.get(tag) {
                None => self.spawn(body),
                // Changed, or a part leaving its compound.
                Some(prev) => {
                    let prev = prev.clone();
                    self.apply_divergence(&prev, body)
                }
            };
            if accepted {
                Arc::make_mut(&mut self.declared).insert(tag.clone(), body.clone());
                stats.written += 1;
            } else if let Some(&live) = self.unfused.get(tag.as_str()) {
                // The replacement's shape isn't ready: keep the old body, as
                // a body waiting on its own replacement does.
                let prev = self.declared[tag].clone();
                self.spawn(&prev);
                let rb = self.tags[tag];
                self.bodies[rb].set_position(live, true);
            }
        }

        // Fuse, in key order, parts in tag order.
        for (key, bodies) in refuse.into_iter().filter(|(_, b)| !b.is_empty()) {
            let parts = Compound::new(
                bodies
                    .iter()
                    .map(|b| {
                        let shape = collider_of(&b.shape).expect("fusable shapes are convex").shape;
                        (part_poses[&b.tag], shape)
                    })
                    .collect(),
            );
            let (shape, merged) = merge_boxes(parts.shapes());
            let collider = ColliderBuilder::new(shape)
                .friction(bodies[0].friction)
                .restitution(bodies[0].restitution)
                .build();
            let collider = self.colliders.insert(collider);
            crate::frame_telemetry::note_bodies(1, 0);
            for body in &bodies {
                if self.declared.get(&body.tag) != Some(*body) {
                    Arc::make_mut(&mut self.declared).insert(body.tag.clone(), (*body).clone());
                    stats.written += 1;
                }
            }
            Arc::make_mut(&mut self.fused).push(FusedStatics {
                key,
                collider,
                tags: bodies.iter().map(|b| b.tag.clone()).collect(),
                parts: Arc::new(parts),
                merged,
            });
        }
        self.unfused.clear();
        if !dirty.is_empty() {
            // Key order, for `fused_part`.
            Arc::make_mut(&mut self.fused).sort_by_key(|f| f.key);
            let (still, left): (BTreeSet<_>, BTreeSet<_>) = std::mem::take(&mut self.fused_touching)
                .into_iter()
                .partition(|(a, b, _)| self.fused_part(a).is_some() || self.fused_part(b).is_some());
            self.fused_touching = still;
            self.handoff.extend(left);
        }

        self.flat = next_flat.unwrap_or(old_flat);
        if !changed.is_empty() || !retired.is_empty() {
            self.groups = groups
                .into_iter()
                .map(|(key, bodies)| (key.to_string(), bodies.clone()))
                .collect();
        }
        self.reconcile_stats = stats;
    }

    /// What the last [`World::reconcile`] did.
    pub fn reconcile_stats(&self) -> ReconcileStats {
        self.reconcile_stats
    }

    /// Report the world's size to `/frames`.
    pub(crate) fn note_size(&self) {
        crate::frame_telemetry::note_world_size(self.bodies.len(), self.colliders.len());
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
        self.stale = false;
        self.apply_volumes();
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
        let sensors = self.sensor_poses();
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
        //
        // A fused room is many colliders, but the game hears about bodies: a
        // pair with a room starts when the first of its collider pairs
        // touches and stops when the last lets go, so a chunk re-meshed under
        // a resting ball (or a ball across two chunks) says nothing. Net
        // starts per room pair, keyed by sorted tags and sensor flag.
        let mut room_pairs: BTreeMap<(String, String, bool), i32> = BTreeMap::new();
        for event in sink.0.into_inner().unwrap() {
            let (h1, h2, started, flags) = match event {
                CollisionEvent::Started(h1, h2, flags) => (h1, h2, true, flags),
                CollisionEvent::Stopped(h1, h2, flags) => (h1, h2, false, flags),
            };
            let tag_of = |handle| self.tag_of_collider(handle);
            let (Some(a), Some(b)) = (tag_of(h1), tag_of(h2)) else {
                continue;
            };
            let sensor = flags.contains(CollisionEventFlags::SENSOR);
            let pair = if a <= b { (a.clone(), b.clone(), sensor) } else { (b.clone(), a.clone(), sensor) };
            if started && self.handoff.contains(&pair) {
                continue;
            }
            if self.rooms.contains_key(&a) || self.rooms.contains_key(&b) {
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                *room_pairs.entry((a, b, sensor)).or_default() += if started { 1 } else { -1 };
            } else {
                self.events.push(PhysicsEvent {
                    started,
                    a,
                    b,
                    sensor,
                });
            }
        }
        for ((a, b, sensor), net) in room_pairs {
            let now = self.touching_pairs(&a, &b, sensor);
            let before = now - net;
            if (before == 0) != (now == 0) {
                self.events.push(PhysicsEvent {
                    started: now > 0,
                    a,
                    b,
                    sensor,
                });
            }
        }
        for (a, b, sensor) in std::mem::take(&mut self.handoff) {
            let alive = self.declared.contains_key(&a) && self.declared.contains_key(&b);
            if alive && self.touching_pairs(&a, &b, sensor) == 0 {
                self.events.push(PhysicsEvent {
                    started: false,
                    a,
                    b,
                    sensor,
                });
            }
        }
        self.diff_fused_contacts(&sensors);
        self.retired.clear();
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
        let pose = match self.tags.get(tag) {
            Some(rb) => *self.bodies.get(*rb)?.position(),
            None => self.part_pose(self.fused_part(tag)?),
        };
        let (pos, rot) = (pose.translation, pose.rotation);
        Some(([pos.x, pos.y, pos.z], [rot.x, rot.y, rot.z, rot.w]))
    }

    /// Live linear velocity of a declared body (zero for a fused one).
    pub fn body_velocity(&self, tag: &str) -> Option<[f32; 3]> {
        let Some(rb) = self.tags.get(tag) else {
            return self.fused_part(tag).map(|_| [0.0; 3]);
        };
        let v = self.bodies.get(*rb)?.linvel();
        Some([v.x, v.y, v.z])
    }

    /// Whether a `Physics.character`'s last move ended on walkable ground and
    /// wasn't upward (`false` before any). `None` if `tag` isn't a character.
    pub fn grounded(&self, tag: &str) -> Option<bool> {
        self.declared.get(tag)?.character?;
        Some(self.grounded.get(tag).copied().unwrap_or(false))
    }

    /// Cast a ray against the live world (docs/physics.md Phase 4): the
    /// nearest hit's tag, world-space point, surface normal, and distance.
    /// `dir` need not be normalized (it is here, so `max_dist` is in world
    /// units); a zero direction yields `None`.
    ///
    /// Answers against the broad phase **as of the last step** — a world that
    /// has never stepped misses everything (the Functor Lang drivers hold deferred
    /// queries until a frame that actually substepped, so game code only sees
    /// answers from a simulated world; direct callers should step first).
    /// Sensor colliders are hittable like any other (rapier's default query
    /// filter) — an invisible trigger volume can occlude a solid body behind
    /// it.
    pub fn raycast(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<RayHit> {
        self.raycast_excluding(origin, dir, max_dist, None)
    }

    /// [`Self::raycast`], but ignoring one body's collider.
    ///
    /// This is what a self-probe needs: a grounding ray cast from inside a
    /// character's own capsule would otherwise hit that capsule at distance 0
    /// and report the character standing on itself. An `exclude` tag that
    /// isn't in the world excludes nothing — a body that doesn't exist can't
    /// be hit anyway, so a not-yet-spawned character probes cleanly instead of
    /// erroring on its first frame.
    pub fn raycast_excluding(
        &self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_dist: f32,
        exclude: Option<&str>,
    ) -> Option<RayHit> {
        let d = vec3(dir);
        let len = d.length();
        if !(len.is_finite() && len > 0.0) {
            return None;
        }
        let ray = rapier3d::prelude::Ray::new(vec3(origin), d / len);
        let hit = self
            .queries(QueryFilter::default(), exclude)
            .cast_ray_and_get_normal(&ray, max_dist, true);
        let (tag, _, hit) = self.with_fused_parts(
            hit,
            exclude,
            |hit| hit.time_of_impact,
            |compound, skip| ray_on_parts(compound, &ray, max_dist, skip),
        )?;
        let kept = match hit.feature {
            FeatureId::Face(i) => Some(i),
            _ => None,
        };
        let point = ray.origin + ray.dir * hit.time_of_impact;
        Some(RayHit {
            triangle: self.source_triangle(&tag, kept),
            tag,
            position: [point.x, point.y, point.z],
            normal: [hit.normal.x, hit.normal.y, hit.normal.z],
            distance: hit.time_of_impact,
        })
    }

    /// Sweep `shape` (a box, sphere or capsule, unrotated) from `origin` along
    /// `dir` for up to `max_dist`: the first body it touches, ignoring
    /// `exclude`'s. `distance` is how far the shape travelled, `position` the
    /// contact point, `normal` the hit surface's outward normal. A shape that
    /// starts overlapping something hits it at distance 0, with an approximate
    /// point and normal, unless the sweep moves out of it. Answers as of the last
    /// step, like [`Self::raycast`].
    pub fn shape_cast(
        &self,
        shape: &Shape,
        origin: [f32; 3],
        dir: [f32; 3],
        max_dist: f32,
        exclude: Option<&str>,
    ) -> Option<RayHit> {
        let d = vec3(dir);
        let len = d.length();
        if !(len.is_finite() && len > 0.0) {
            return None;
        }
        let swept = collider_of(shape)?.shape;
        let queries = self.queries(QueryFilter::default(), exclude);
        let from = Pose::from_translation(vec3(origin));
        let dir = d / len;
        let hit = sweep(&queries, &*swept, &from, dir, max_dist, 0.0);
        let (tag, col_handle, hit) = self.with_fused_parts(
            hit,
            exclude,
            |hit| hit.time_of_impact,
            |compound, skip| sweep_parts(compound, &*swept, &from, dir, max_dist, skip),
        )?;
        // Shape casts report no feature, so find the triangle under the
        // contact point (on a shared edge, either neighbour).
        let collider = &self.colliders[col_handle];
        let kept = collider.shape().as_trimesh().map(|mesh| {
            let local = collider.position().inverse_transform_point(hit.witness1);
            mesh.project_local_point_and_get_location(local, false).1 .0
        });
        // Witness and normal 1 are on the hit collider, in world space.
        let (p, n) = (hit.witness1, hit.normal1);
        Some(RayHit {
            triangle: self.source_triangle(&tag, kept),
            tag,
            position: [p.x, p.y, p.z],
            normal: [n.x, n.y, n.z],
            distance: hit.time_of_impact,
        })
    }

    /// Sweep `shape` (a box, sphere or capsule) from pose `from` to pose `to`,
    /// rotating about its centre as it moves: the first `filter` body it
    /// touches, or `None` if the motion is clear. Sensors are never hit.
    ///
    /// A shape starting in an overlap hits at fraction 0 when the motion
    /// presses deeper into it, and passes when it moves out. With no motion
    /// it answers whether `from` overlaps anything (fraction 0) at all.
    /// Answers as of the last step, like [`Self::raycast`].
    pub fn sweep(&self, shape: &Shape, from: RigidPose, to: RigidPose, filter: SweepFilter) -> Option<SweepHit> {
        let swept = collider_of(shape)?.shape;
        let swept = &*swept;
        let filter = match filter {
            SweepFilter::Fixed => QueryFilter::only_fixed(),
            SweepFilter::NotKinematic => QueryFilter::exclude_kinematic(),
            SweepFilter::All => QueryFilter::default(),
        };
        let queries = self.queries(filter.exclude_sensors(), None);
        let start = rigid(from);
        let motion = motion_between(start, rigid(to));
        let still = motion.linvel.length() < STILL && motion.angvel.length() < STILL;
        let motion = (!still).then_some(&motion);
        let reach = reach(swept, &start, motion);
        // Pair by pair rather than rapier's pipeline `cast_shape_nonlinear`:
        // the start-overlap rule needs each pair's contact, and a fused
        // compound's parts are culled to the reach (a 30 m floor part's
        // bounding sphere otherwise reaches every sweep).
        let mut best: Option<(ColliderHandle, Option<usize>, ShapeCastHit)> = None;
        let mut keep = |candidate: (ColliderHandle, Option<usize>, Option<ShapeCastHit>)| {
            let (collider, part, Some(hit)) = candidate else { return };
            if best.as_ref().is_none_or(|b| hit.time_of_impact < b.2.time_of_impact) {
                best = Some((collider, part, hit));
            }
        };
        for (handle, co) in queries.intersect_aabb_conservative(reach) {
            match self.parts(handle) {
                // Fused parts are in world space (the compound sits at the origin).
                Some(compound) => {
                    for i in compound.bvh().intersect_aabb(&reach) {
                        let (pose, part) = &compound.shapes()[i as usize];
                        keep((handle, Some(i as usize), pair_hit(pose, &**part, swept, &start, motion)));
                    }
                }
                None => keep((handle, None, pair_hit(co.position(), co.shape(), swept, &start, motion))),
            }
        }
        let (collider, part, hit) = best?;
        let tag = self.hit_tag(collider, part)?;
        let (p, n) = (hit.witness1, hit.normal1);
        Some(SweepHit {
            tag,
            fraction: hit.time_of_impact.clamp(0.0, 1.0),
            position: [p.x, p.y, p.z],
            normal: [n.x, n.y, n.z],
        })
    }

    /// Scene queries as of the last step, ignoring `exclude`'s whole rigid
    /// body: a self-probe starting inside a character's capsule would
    /// otherwise hit it at distance 0. A tag not in the world excludes
    /// nothing, so a not-yet-spawned character probes cleanly.
    fn queries<'a>(&'a self, mut filter: QueryFilter<'a>, exclude: Option<&str>) -> QueryPipeline<'a> {
        // The rigid body, not the tag's one collider, so a body that later
        // gains a second collider still can't hit itself.
        if let Some(tag) = exclude {
            if let Some(body) = self.tags.get(tag) {
                filter = filter.exclude_rigid_body(*body);
            } else if let Some((compound, _)) = self.fused_part(tag) {
                filter = filter.exclude_collider(compound);
            }
        }
        self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            filter,
        )
    }

    /// The source index of rapier triangle `kept` on `tag`'s trimesh.
    fn source_triangle(&self, tag: &str, kept: Option<u32>) -> Option<u32> {
        match &self.declared.get(tag)?.shape {
            Shape::TriMesh { mesh } => mesh.source_triangle(kept?),
            _ => None,
        }
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

        let mut bodies: Vec<BodyDump> = self
            .tags
            .iter()
            .filter_map(|(tag, rb_handle)| {
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
        // Fused bodies, as the fixed bodies they stand for.
        for fused in self.fused.iter() {
            for (tag, (pose, _)) in fused.tags.iter().zip(fused.parts.shapes()) {
                let (pos, rot) = (pose.translation, pose.rotation);
                bodies.push(BodyDump {
                    tag,
                    position: [pos.x, pos.y, pos.z],
                    rotation: [rot.x, rot.y, rot.z, rot.w],
                    velocity: [0.0; 3],
                    sleeping: false,
                });
            }
        }
        bodies.sort_by(|a, b| a.tag.cmp(b.tag));

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
            if let PhysicsCommand::Explode(blast) = &command {
                self.explode(blast);
                continue;
            }
            let (tag, kind) = command.tag_and_kind();
            let (tag, kind) = (tag.to_string(), kind);
            let Some(&rb_handle) = self.tags.get(tag.as_str()) else {
                match (self.fused_part(&tag), &command) {
                    // A fused body moves within its compound.
                    (Some((c, part)), PhysicsCommand::Teleport { position, .. }) => {
                        let mut pose = self.part_pose((c, part));
                        pose.translation = vec3(*position);
                        self.set_part_pose(c, part, pose);
                    }
                    (Some(_), PhysicsCommand::Move { .. }) => self.push_command_warning(format!(
                        "physics move on \"{tag}\", which is not a Physics.character, has no effect"
                    )),
                    (Some(_), _) => self.push_command_warning(format!(
                        "physics {kind} on non-dynamic body \"{tag}\" has no effect"
                    )),
                    (None, _) => self.push_command_warning(format!(
                        "physics {kind} for unknown tag \"{tag}\""
                    )),
                }
                continue;
            };
            if let PhysicsCommand::Move { translation, .. } = &command {
                if !self.move_character(&tag, rb_handle, vec3(*translation)) {
                    self.push_command_warning(format!(
                        "physics move on \"{tag}\", which is not a Physics.character, has no effect"
                    ));
                }
                continue;
            }
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
            if self.rooms.contains_key(&tag) {
                self.push_command_warning(format!(
                    "physics {kind} on the fused room \"{tag}\" has no effect: it is where \
                     the room is"
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
                // The preserved axes are read from the LIVE body here, not
                // from whatever the game read in `tick` — so these compose
                // with reconcile and with any command already applied from
                // this same frame's queue.
                PhysicsCommand::SetVelocityXZ { x, z, .. } => {
                    let live = rb.linvel();
                    rb.set_linvel(vec3([*x, live.y, *z]), true);
                }
                PhysicsCommand::SetVelocityY { y, .. } => {
                    let live = rb.linvel();
                    rb.set_linvel(vec3([live.x, *y, live.z]), true);
                }
                PhysicsCommand::Teleport { position, .. } => {
                    let rotation = *rb.rotation();
                    rb.set_position(Pose::from_parts(vec3(*position), rotation), true);
                }
                PhysicsCommand::Move { .. } | PhysicsCommand::Explode(_) => {
                    unreachable!("handled above")
                }
            }
        }
    }

    /// Push every dynamic body whose collider is within the blast radius
    /// away from its centre, in tag order. The distance is to the nearest
    /// collider point, so a large crate beside the blast counts; the push
    /// lands at that point, so off-centre hits spin the body.
    fn explode(&mut self, blast: &Blast) {
        let center = vec3(blast.center);
        for &handle in self.tags.values() {
            let rb = &self.bodies[handle];
            if !rb.is_dynamic() {
                continue;
            }
            // Pose from the body, not the collider: a body teleported this
            // frame moves its colliders only at the step.
            let nearest = rb
                .colliders()
                .iter()
                .map(|&c| {
                    let collider = &self.colliders[c];
                    let pose = match collider.position_wrt_parent() {
                        Some(local) => *rb.position() * *local,
                        None => *rb.position(),
                    };
                    collider.shape().project_point(&pose, center, true).point
                })
                .min_by(|a, b| a.distance(center).total_cmp(&b.distance(center)));
            let Some(point) = nearest else { continue };
            let distance = point.distance(center);
            if !(distance <= blast.radius) {
                continue;
            }
            let scale = falloff(blast.falloff, distance / blast.radius);
            let impulse =
                blast_direction(center, rb.center_of_mass(), blast.lift) * blast.strength * scale;
            self.bodies[handle].apply_impulse_at_point(impulse, point, true);
        }
    }

    /// One step of every force volume on the awake dynamic bodies inside it,
    /// after waking the sleepers where volumes changed. O(volumes × bodies),
    /// and only when a volume is declared or changed.
    fn apply_volumes(&mut self) {
        let inside = |rb: &RigidBody, (center, size): ([f32; 3], [f32; 3])| {
            ((rb.center_of_mass() - vec3(center)).abs() - vec3(size) * 0.5).max_element() <= 0.0
        };
        for region in std::mem::take(&mut self.wake_boxes) {
            for &handle in self.tags.values() {
                let rb = &mut self.bodies[handle];
                if rb.is_dynamic() && rb.is_sleeping() && inside(rb, region) {
                    rb.wake_up(true);
                }
            }
        }
        let time = self.frame as f32 * FIXED_DT;
        for volume in &self.volumes {
            let force = vec3(volume.force);
            let seed = fnv1a(volume.tag.as_bytes());
            for (tag, &handle) in self.tags.iter() {
                let rb = &mut self.bodies[handle];
                if !rb.is_dynamic() || rb.is_sleeping() || !inside(rb, (volume.center, volume.size)) {
                    continue;
                }
                let mut push = force;
                if volume.turbulence > 0.0 {
                    push += gust(seed ^ fnv1a(tag.as_bytes()).rotate_left(1), time)
                        * volume.turbulence;
                }
                let mut impulse = push * FIXED_DT;
                if volume.drag > 0.0 {
                    impulse -= rb.linvel() * rb.mass() * (volume.drag * FIXED_DT).min(1.0);
                }
                // No wake: it restarts the sleep timer, which would keep a body
                // the draft can't move awake forever.
                rb.apply_impulse(impulse, false);
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

    fn spawn(&mut self, body: &Body) -> bool {
        if let Shape::FusedRoom { room } = &body.shape {
            // A fixed body with a collider per chunk.
            let rb_handle = self.bodies.insert(RigidBodyBuilder::fixed().build());
            crate::frame_telemetry::note_bodies(1, 0);
            Arc::make_mut(&mut self.tags).insert(body.tag.clone(), rb_handle);
            Arc::make_mut(&mut self.rooms).insert(body.tag.clone(), LiveRoom::default());
            self.sync_room(body, &RoomMeshes::default(), room);
            return true;
        }
        let Some(mut collider) = collider_of(&body.shape) else {
            return false;
        };
        let builder = match body.kind {
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
            BodyKind::Fixed => RigidBodyBuilder::fixed(),
        };
        let mut builder = builder
            .pose(pose_of(body))
            .linvel(vec3(body.velocity))
            // Damping lives on the RIGID BODY (unlike friction/restitution,
            // which are collider material properties): it is a velocity decay
            // the integrator applies, not a contact response. Rapier only
            // integrates a dynamic body, so it is inert on kinematic/fixed.
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping)
            .ccd_enabled(body.continuous);
        if body.rotation_locked {
            // An upright body translates but never tips (the character-capsule
            // case). This stops the solver from ACCUMULATING spin; a body that
            // already has angular velocity keeps it, which is why the reconcile
            // path also cancels the spin when the lock is switched on. At spawn
            // there is none to cancel — angular velocity is not declarable.
            builder = builder.locked_axes(LockedAxes::ROTATION_LOCKED);
        }
        let rb = builder.build();

        collider = with_material(collider, body);
        if let Some(mass) = body.mass {
            collider = collider.mass(mass);
        }

        let rb_handle = self.bodies.insert(rb);
        crate::frame_telemetry::note_bodies(1, 0);
        self.colliders
            .insert_with_parent(collider.build(), rb_handle, &mut self.bodies);
        Arc::make_mut(&mut self.tags).insert(body.tag.clone(), rb_handle);
        true
    }

    /// Bring a fused room body's colliders from `prev` to `next`: rebuild
    /// only the chunks [`room::dirty_chunks`] names, in key order, and the
    /// floor plane if the floor moved.
    fn sync_room(&mut self, body: &Body, prev: &RoomMeshes, next: &RoomMeshes) {
        if !self.rooms.contains_key(&body.tag) {
            return;
        }
        let (Some(&rb_handle), Some(live)) =
            (self.tags.get(&body.tag), Arc::make_mut(&mut self.rooms).get_mut(&body.tag))
        else {
            return;
        };
        // An emptied room (the shell started over) has no floor.
        let floor = if next.chunks.is_empty() {
            None
        } else {
            room::settle_floor(live.floor, room::floor_height(next))
        };
        if floor != live.floor {
            if let Some(handle) = live.floor_plane.take() {
                self.colliders
                    .remove(handle, &mut self.islands, &mut self.bodies, true);
                self.retired.push((handle, body.tag.clone()));
            }
            if let Some(y) = floor {
                let plane = ColliderBuilder::halfspace(rapier3d::na::Unit::new_unchecked(Vector::Y))
                    .translation(Vector::new(0.0, y, 0.0));
                live.floor_plane = Some(self.colliders.insert_with_parent(
                    with_material(plane, body).build(),
                    rb_handle,
                    &mut self.bodies,
                ));
            }
        }
        for key in room::dirty_chunks(prev, live.floor, next, floor) {
            if let Some(handle) = live.chunks.remove(&key) {
                self.colliders
                    .remove(handle, &mut self.islands, &mut self.bodies, true);
                self.retired.push((handle, body.tag.clone()));
            }
            let Some(geometry) = next
                .chunks
                .get(&key)
                .and_then(|mesh| room::chunk_collider(mesh, floor))
            else {
                continue;
            };
            let Some(collider) = chunk_trimesh(geometry) else {
                log::warn!("fused room: chunk {key:?} is not a valid triangle mesh; skipped");
                continue;
            };
            let handle = self.colliders.insert_with_parent(
                with_material(collider, body).build(),
                rb_handle,
                &mut self.bodies,
            );
            live.chunks.insert(key, handle);
        }
        live.floor = floor;
    }

    /// The fused room's colliders (chunks plus the floor plane) and floor
    /// height (`/state.xr.mesh`); `None` without a `Physics.fusedRoom` body.
    pub fn fused_room_colliders(&self) -> Option<(usize, Option<f32>)> {
        let live = self.rooms.values().next()?;
        Some((
            live.chunks.len() + usize::from(live.floor_plane.is_some()),
            live.floor,
        ))
    }

    /// Collider pairs between the bodies tagged `a` and `b` touching now
    /// (overlapping, for a sensor pair).
    fn touching_pairs(&self, a: &str, b: &str, sensor: bool) -> i32 {
        let (Some(&a), Some(&b)) = (self.tags.get(a), self.tags.get(b)) else {
            return 0;
        };
        // Walk the body with fewer colliders: the ball, not the room.
        let count = |body: RigidBodyHandle| self.bodies[body].colliders().len();
        let (mine, other) = if count(a) <= count(b) { (a, b) } else { (b, a) };
        let of_other =
            |c: ColliderHandle| self.colliders.get(c).and_then(|c| c.parent()) == Some(other);
        let mut touching = 0;
        for &c in self.bodies[mine].colliders() {
            touching += if sensor {
                self.narrow_phase
                    .intersection_pairs_with(c)
                    .filter(|&(c1, c2, hit)| hit && of_other(if c1 == c { c2 } else { c1 }))
                    .count()
            } else {
                self.narrow_phase
                    .contact_pairs_with(c)
                    .filter(|p| {
                        let partner = if p.collider1 == c { p.collider2 } else { p.collider1 };
                        p.has_any_active_contact() && of_other(partner)
                    })
                    .count()
            };
        }
        touching as i32
    }

    /// The pairs `tag`'s body touches now, as `fused_touching` entries.
    fn touching_of(&self, tag: &str) -> Vec<(String, String, bool)> {
        let Some(&rb) = self.tags.get(tag) else {
            return Vec::new();
        };
        let mut pairs = Vec::new();
        for &c in self.bodies[rb].colliders() {
            let contacts = self
                .narrow_phase
                .contact_pairs_with(c)
                .filter(|p| p.has_any_active_contact())
                .map(|p| (if p.collider1 == c { p.collider2 } else { p.collider1 }, false));
            let overlaps = self
                .narrow_phase
                .intersection_pairs_with(c)
                .filter(|&(_, _, hit)| hit)
                .map(|(c1, c2, _)| (if c1 == c { c2 } else { c1 }, true));
            for (other, sensor) in contacts.chain(overlaps) {
                if let Some(other) = self.tag_of_collider(other) {
                    let tag = tag.to_string();
                    pairs.push(if tag <= other { (tag, other, sensor) } else { (other, tag, sensor) });
                }
            }
        }
        pairs
    }

    /// The tag of the body `collider` belongs to, or belonged to if a
    /// re-fused room removed it this step.
    fn tag_of_collider(&self, collider: ColliderHandle) -> Option<String> {
        let Some(body) = self.colliders.get(collider).and_then(|c| c.parent()) else {
            return self
                .retired
                .iter()
                .find(|(retired, _)| *retired == collider)
                .map(|(_, tag)| tag.clone());
        };
        // Scenes are small; a scan beats carrying a second map in every
        // snapshot.
        self.tags
            .iter()
            .find(|(_, &rb)| rb == body)
            .map(|(tag, _)| tag.clone())
    }

    /// A world query's hit, with its tag: a fused compound's hit part is
    /// found by casting on its parts (`on_parts`). An excluded fused body's
    /// compound was skipped whole, so its other parts are cast too and the
    /// nearer hit (by `toi`) wins.
    fn with_fused_parts<H>(
        &self,
        hit: Option<(ColliderHandle, H)>,
        exclude: Option<&str>,
        toi: impl Fn(&H) -> f32,
        on_parts: impl Fn(&Compound, Option<usize>) -> Option<(usize, H)>,
    ) -> Option<(String, ColliderHandle, H)> {
        let hit = hit.map(|(collider, hit)| {
            // The part's own hit, so tag and geometry agree on a seam.
            match self.parts(collider).and_then(|parts| on_parts(parts, None)) {
                Some((part, hit)) => (collider, Some(part), hit),
                None => (collider, None, hit),
            }
        });
        let rest = exclude.and_then(|tag| self.fused_part(tag)).and_then(|(c, skip)| {
            let (part, hit) = on_parts(self.parts(c)?, Some(skip))?;
            Some((c, Some(part), hit))
        });
        let (collider, part, hit) = [hit, rest]
            .into_iter()
            .flatten()
            .min_by(|a, b| toi(&a.2).total_cmp(&toi(&b.2)))?;
        Some((self.hit_tag(collider, part)?, collider, hit))
    }

    /// The tag of a query's hit: its part's, on a fused compound.
    fn hit_tag(&self, collider: ColliderHandle, part: Option<usize>) -> Option<String> {
        match (self.fused_tags(collider), part) {
            (Some(tags), Some(part)) => tags.get(part).cloned(),
            _ => self.tag_of_collider(collider),
        }
    }

    /// The compound part that is body `tag`, if `tag` is fused.
    fn fused_part(&self, tag: &str) -> Option<(ColliderHandle, usize)> {
        let key = fuse_key(self.declared.get(tag)?);
        let f = &self.fused[self.fused.binary_search_by_key(&key, |f| f.key).ok()?];
        let part = f.tags.binary_search_by(|t| t.as_str().cmp(tag)).ok()?;
        Some((f.collider, part))
    }

    /// The fused statics whose compound is `collider`.
    fn fused_of(&self, collider: ColliderHandle) -> Option<&FusedStatics> {
        self.fused.iter().find(|f| f.collider == collider)
    }

    /// The tags of `collider`'s parts, if it is a fused compound.
    fn fused_tags(&self, collider: ColliderHandle) -> Option<&[String]> {
        self.fused_of(collider).map(|f| f.tags.as_slice())
    }

    /// `collider`'s unmerged parts, one per tag, if it is a fused compound.
    fn parts(&self, collider: ColliderHandle) -> Option<&Compound> {
        self.fused_of(collider).map(|f| &*f.parts)
    }

    fn part_pose(&self, (collider, part): (ColliderHandle, usize)) -> Pose {
        self.parts(collider).expect("a fused part's compound").shapes()[part].0
    }

    /// Move one fused part, re-merging its compound.
    fn set_part_pose(&mut self, collider: ColliderHandle, part: usize, pose: Pose) {
        let fused = Arc::make_mut(&mut self.fused);
        let f = fused.iter_mut().find(|f| f.collider == collider).expect("a fused part's compound");
        let mut shapes = f.parts.shapes().to_vec();
        shapes[part].0 = pose;
        f.parts = Arc::new(Compound::new(shapes));
        let (shape, merged) = merge_boxes(f.parts.shapes());
        f.merged = merged;
        self.colliders[collider].set_shape(shape);
    }

    /// Sensor poses as the coming step's collision detection sees them (it
    /// runs before integration); only needed beside fused compounds.
    fn sensor_poses(&self) -> Vec<(ColliderHandle, Pose)> {
        if self.fused.is_empty() {
            return Vec::new();
        }
        self.colliders
            .iter()
            .filter(|(_, c)| c.is_sensor())
            .map(|(handle, c)| {
                let pose = match (c.parent(), c.position_wrt_parent()) {
                    (Some(rb), Some(local)) => *self.bodies[rb].position() * *local,
                    _ => *c.position(),
                };
                (handle, pose)
            })
            .collect()
    }

    /// Diff which fused parts touch which bodies into this step's events.
    /// rapier reports the compound as one pair per body; a part touches when
    /// a manifold on its (maybe merged) box has solver contacts (rapier's own
    /// rule for a pair) whose bounds meet the part, and a sensor overlaps the
    /// parts it intersected at detection (`sensors`). A pair whose body is
    /// gone ends unreported, as a despawned body's contacts do.
    fn diff_fused_contacts(&mut self, sensors: &[(ColliderHandle, Pose)]) {
        let mut now = BTreeSet::new();
        let mut events = Vec::new();
        let mut touch = |a: &str, b: String, sensor: bool| {
            let (a, b) = (a.to_string(), b);
            now.insert(if a <= b { (a, b, sensor) } else { (b, a, sensor) });
        };
        let near = self.integration_parameters.prediction_distance();
        for fused in self.fused.iter() {
            let c = fused.collider;
            for pair in self.narrow_phase.contact_pairs_with(c) {
                let mine_first = pair.collider1 == c;
                let other = if mine_first { pair.collider2 } else { pair.collider1 };
                let Some(other) = self.tag_of_collider(other) else {
                    continue;
                };
                for m in pair.manifolds.iter().filter(|m| !m.data.solver_contacts.is_empty()) {
                    let sub = if mine_first { m.subshape1 } else { m.subshape2 };
                    // A merged box stands for several parts: those its contact
                    // patch meets (world space, as the compound's frame). A
                    // point is midway between the surfaces: move it onto ours.
                    let toward_us = if mine_first { -0.5 } else { 0.5 };
                    let points: Vec<Vector> = (m.data.solver_contacts.iter())
                        .map(|p| p.point + m.data.normal * (p.dist * toward_us))
                        .collect();
                    let under = Aabb::from_points(points.iter().copied()).loosened(near);
                    for i in fused.parts.bvh().intersect_aabb(&under) {
                        let part = fused.parts.aabbs()[i as usize].loosened(near);
                        if fused.merged[i as usize] == sub && patch_meets(&points, &part) {
                            touch(&fused.tags[i as usize], other.clone(), false);
                        }
                    }
                }
            }
            let compound = &*fused.parts;
            for (c1, c2, hit) in self.narrow_phase.intersection_pairs_with(c) {
                let other = if c1 == c { c2 } else { c1 };
                let (Some(tag), true) = (self.tag_of_collider(other), hit) else {
                    continue;
                };
                let at = sensors.iter().find(|(c, _)| *c == other).map(|(_, at)| *at);
                let other = &self.colliders[other];
                let at = at.unwrap_or(*other.position());
                for i in compound.bvh().intersect_aabb(&other.shape().compute_aabb(&at)) {
                    let (pose, part) = &compound.shapes()[i as usize];
                    let overlaps = self
                        .narrow_phase
                        .query_dispatcher()
                        .intersection_test(&pose.inv_mul(&at), &**part, other.shape())
                        .unwrap_or(false);
                    if overlaps {
                        touch(&fused.tags[i as usize], tag.clone(), true);
                    }
                }
            }
        }
        // A dynamic sensor with CCD that crossed a part within the step,
        // touching it neither before nor after: a start and a stop, as rapier
        // reports a tunnelling sensor pair.
        for &(handle, before) in sensors {
            let sensor = &self.colliders[handle];
            let ccd = sensor
                .parent()
                .is_some_and(|rb| self.bodies[rb].is_dynamic() && self.bodies[rb].is_ccd_enabled());
            let (after, shape) = (*sensor.position(), sensor.shape());
            let motion = after.translation - before.translation;
            let Some(tag) = ccd.then(|| self.tag_of_collider(handle)).flatten() else {
                continue;
            };
            let swept = shape.compute_aabb(&before).merged(&shape.compute_aabb(&after));
            let dispatcher = self.narrow_phase.query_dispatcher();
            for fused in self.fused.iter() {
                let compound = &*fused.parts;
                for i in compound.bvh().intersect_aabb(&swept) {
                    let (pose, part) = &compound.shapes()[i as usize];
                    let at = |p: &Pose| dispatcher.intersection_test(&pose.inv_mul(p), &**part, shape);
                    let crossed = rapier3d::parry::query::cast_shapes(
                        pose, Vector::ZERO, &**part, &before, motion, shape, cast_options(1.0, 0.0),
                    );
                    if matches!(crossed, Ok(Some(_)))
                        && !at(&before).unwrap_or(true)
                        && !at(&after).unwrap_or(true)
                    {
                        let other = &fused.tags[i as usize];
                        let (a, b) = if *other <= tag { (other, &tag) } else { (&tag, other) };
                        for started in [true, false] {
                            events.push(PhysicsEvent {
                                started,
                                a: a.clone(),
                                b: b.clone(),
                                sensor: true,
                            });
                        }
                    }
                }
            }
        }
        // A pair whose body went away ends unreported, as a despawned body's
        // contacts do.
        let alive = |tag: &str| self.declared.contains_key(tag);
        for (a, b, sensor) in self.fused_touching.difference(&now) {
            if alive(a) && alive(b) {
                events.push(PhysicsEvent {
                    started: false,
                    a: a.clone(),
                    b: b.clone(),
                    sensor: *sensor,
                });
            }
        }
        for (a, b, sensor) in now.difference(&self.fused_touching) {
            events.push(PhysicsEvent {
                started: true,
                a: a.clone(),
                b: b.clone(),
                sensor: *sensor,
            });
        }
        self.events.extend(events);
        self.fused_touching = now;
    }

    fn despawn(&mut self, tag: &str) {
        cow_remove(&mut self.rooms, tag);
        // A despawned body's contacts end unreported, retired ones included.
        self.retired.retain(|(_, retired)| retired != tag);
        if let Some(rb_handle) = cow_remove(&mut self.tags, tag) {
            crate::frame_telemetry::note_bodies(0, 1);
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
        cow_remove(&mut self.declared, tag);
        self.grounded.remove(tag);
    }

    /// One `Move`: rapier's character controller (collide and slide, slope
    /// limit, snap to ground) plus a step-up probe, applied as the body's
    /// next kinematic target. Starts from a target already queued this step,
    /// so moves chain. `false` if `tag` isn't a character.
    fn move_character(
        &mut self,
        tag: &str,
        rb_handle: RigidBodyHandle,
        desired: Vector,
    ) -> bool {
        let Some(settings) = self.declared.get(tag).and_then(|body| body.character) else {
            return false;
        };
        // A character's body owns its one capsule collider.
        let Some(&col_handle) = self.bodies[rb_handle].colliders().first() else {
            return false;
        };
        let shape = self.colliders[col_handle].shared_shape().clone();
        let start = *self.bodies[rb_handle].next_position();
        let controller = KinematicCharacterController {
            up: Vector::Y,
            offset: CharacterLength::Absolute(settings.offset),
            slide: true,
            // `step_up` instead: rapier's autostep needs a wall-like hit, but a
            // capsule meets a step's edge with its rounded bottom.
            autostep: None,
            max_slope_climb_angle: settings.max_slope,
            min_slope_slide_angle: settings.max_slope,
            // Snapped below instead, across both passes.
            snap_to_ground: None,
            normal_nudge_factor: 1.0e-4,
        };
        let queries = self.queries(QueryFilter::default().exclude_sensors(), Some(tag));
        let at = |t: Vector| Pose::from_translation(t) * start;
        let was_grounded = on_walkable_ground(&queries, &*shape, &start, &settings);
        // Walk, then fall: one combined move stalls whenever the capsule sits
        // just inside the offset gap, as its first sweep hits the floor at 0.
        let move_by = |from: Vector, by: Vector| {
            from + controller
                .move_shape(FIXED_DT, &queries, &*shape, &at(from), by, |_| {})
                .translation
        };
        let walked = move_by(Vector::ZERO, Vector::new(desired.x, 0.0, desired.z));
        let mut translation = move_by(walked, Vector::new(0.0, desired.y, 0.0));
        if on_walkable_ground(&queries, &*shape, &at(translation), &settings) {
            if let Some(step) = step_up(&queries, &*shape, &at(translation), desired, translation, &settings) {
                translation += step;
            }
        } else if was_grounded && desired.y <= 0.0 {
            // Follow the ground down a stair or ramp by up to `snap`.
            let below = sweep(&queries, &*shape, &at(translation), -Vector::Y, settings.snap, settings.offset);
            if let Some((_, hit)) = below {
                translation.y -= hit.time_of_impact;
            }
        }
        // Rising (a jump) isn't grounded, even while the feet are still close.
        let grounded =
            desired.y <= 0.0 && on_walkable_ground(&queries, &*shape, &at(translation), &settings);
        self.bodies[rb_handle].set_next_kinematic_position(at(translation));
        self.grounded.insert(tag.to_string(), grounded);
        true
    }

    /// The declaration for an existing tag changed: write exactly the changed
    /// fields into the live body (docs/physics.md, Authority + divergence).
    /// Structural changes (kind/shape/mass) rebuild the body wholesale — the
    /// tag is the identity, not the handle — but the rebuilt body keeps its
    /// *live* pose/velocities for every field whose declaration didn't change:
    /// a mass change on a falling crate must not teleport it back to its
    /// declared spawn pose.
    fn apply_divergence(&mut self, prev: &Body, next: &Body) -> bool {
        // A re-fused room rebuilds only its changed chunks, below.
        let rooms = match (&prev.shape, &next.shape) {
            (Shape::FusedRoom { room: a }, Shape::FusedRoom { room: b }) => Some((a, b)),
            _ => None,
        };
        // (A room is fixed, so its mass means nothing.)
        if prev.kind != next.kind
            || (prev.shape != next.shape && rooms.is_none())
            || (prev.mass != next.mass && rooms.is_none())
            || self.unfused.contains_key(&next.tag)
        {
            if !shape_is_ready(&next.shape) {
                return false;
            }
            let live = self.tags.get(&next.tag).and_then(|rb| {
                let rb = self.bodies.get(*rb)?;
                let live_pose = *rb.position();
                // Rapier documents next_position() as unspecified for
                // non-kinematic bodies. Only a kinematic predecessor can
                // contribute a meaningful queued target across this rebuild.
                let pending_pose = if prev.kind == BodyKind::Kinematic {
                    *rb.next_position()
                } else {
                    live_pose
                };
                Some((
                    live_pose,
                    pending_pose,
                    rb.linvel(),
                    rb.angvel(),
                ))
            });
            // An unfused part: a still fixed body at its live pose.
            let live = live.or_else(|| {
                let pose = *self.unfused.get(&next.tag)?;
                Some((pose, pose, Vector::ZERO, Vector::ZERO))
            });
            self.despawn(&next.tag);
            let spawned = self.spawn(next);
            debug_assert!(spawned);
            if let (Some((live_pose, pending_pose, linvel, angvel)), Some(rb_handle)) =
                (live, self.tags.get(&next.tag).copied())
            {
                let rb = &mut self.bodies[rb_handle];
                if next.kind == BodyKind::Kinematic {
                    // A structural rebuild must preserve both halves of a
                    // position-based kinematic body's state. Its current pose
                    // remains live, while unchanged declaration fields retain
                    // any already-queued target and changed fields target the
                    // new declaration for the next step.
                    let declared_pose = pose_of(next);
                    let mut target_pose = pending_pose;
                    if prev.position != next.position {
                        target_pose.translation = declared_pose.translation;
                    }
                    if prev.rotation != next.rotation {
                        target_pose.rotation = declared_pose.rotation;
                    }
                    if live_pose.rotation.dot(target_pose.rotation) < 0.0 {
                        target_pose.rotation = -target_pose.rotation;
                    }
                    rb.set_position(live_pose, true);
                    rb.set_next_kinematic_position(target_pose);
                } else {
                    let rebuilt_pose = keep_live(prev, next, *rb.position(), live_pose);
                    if *rb.position() != rebuilt_pose {
                        rb.set_position(rebuilt_pose, true);
                    }
                }
                if prev.velocity == next.velocity {
                    rb.set_linvel(linvel, true);
                }
                // Angular velocity is never declarable — always physics-owned.
                // But an upright body has no angular velocity to preserve:
                // carrying the pre-rebuild spin over would leave it rotating
                // forever, since locking the axes only stops inertia from
                // ACCUMULATING spin, it does not cancel spin already present.
                rb.set_angvel(
                    if next.rotation_locked {
                        Vector::ZERO
                    } else {
                        angvel
                    },
                    true,
                );
            }
            return true;
        }

        if let Some((a, b)) = rooms.filter(|(a, b)| a != b) {
            self.sync_room(next, a, b);
        }
        let rb_handle = self.tags[&next.tag];

        if prev.position != next.position || prev.rotation != next.rotation {
            let rb = &mut self.bodies[rb_handle];
            let live_pose = *rb.position();
            // Kinematic declarations may be reconciled more than once before
            // a step. Build on the queued target so a change to one field
            // preserves the other pending field; dynamic/fixed bodies instead
            // preserve their live simulated pose.
            let mut pose = if next.kind == BodyKind::Kinematic {
                *rb.next_position()
            } else {
                live_pose
            };
            let declared_pose = pose_of(next);
            if prev.position != next.position {
                pose.translation = declared_pose.translation;
            }
            if prev.rotation != next.rotation {
                pose.rotation = declared_pose.rotation;
            }
            if next.kind == BodyKind::Kinematic && rb.rotation().dot(pose.rotation) < 0.0 {
                // q and -q describe the same orientation, but Rapier derives a
                // position-based kinematic body's angular velocity from the
                // raw quaternion delta. Keep the target on the live pose's
                // hemisphere so crossing 180° takes the short arc instead of
                // generating an almost-full-turn contact velocity.
                pose.rotation = -pose.rotation;
            }
            match next.kind {
                // Always refresh a changed kinematic declaration, even when it
                // returns to the live pose: an earlier reconcile may have
                // queued a target that this declaration is canceling.
                BodyKind::Kinematic => rb.set_next_kinematic_position(pose),
                // Skip a no-op teleport so a declaration that catches up with
                // the simulation does not wake a settled dynamic body.
                _ if live_pose != pose => rb.set_position(pose, true),
                _ => {}
            }
        }
        if prev.velocity != next.velocity {
            let rb = &mut self.bodies[rb_handle];
            let v = vec3(next.velocity);
            if rb.linvel() != v {
                rb.set_linvel(v, true);
            }
        }
        // Damping is a coefficient the integrator reads, so writing it does not
        // wake the body (the friction/restitution rule) — a sleeping body has
        // no velocity for the new value to act on, and it takes effect the
        // moment something else wakes it.
        if prev.linear_damping != next.linear_damping {
            self.bodies[rb_handle].set_linear_damping(next.linear_damping);
        }
        if prev.angular_damping != next.angular_damping {
            self.bodies[rb_handle].set_angular_damping(next.angular_damping);
        }
        if (prev.friction, prev.restitution, prev.sensor)
            != (next.friction, next.restitution, next.sensor)
        {
            for handle in self.bodies[rb_handle].colliders().to_vec() {
                let collider = &mut self.colliders[handle];
                collider.set_friction(next.friction);
                collider.set_restitution(next.restitution);
                collider.set_sensor(next.sensor);
            }
        }
        if prev.continuous != next.continuous {
            self.bodies[rb_handle].enable_ccd(next.continuous);
        }
        if prev.rotation_locked != next.rotation_locked {
            let rb = &mut self.bodies[rb_handle];
            rb.set_locked_axes(
                if next.rotation_locked {
                    LockedAxes::ROTATION_LOCKED
                } else {
                    LockedAxes::empty()
                },
                true,
            );
            // Locking only zeroes the effective inverse inertia, so an already
            // spinning body would keep its angular velocity and rotate forever.
            // Standing a tumbling body up is the whole point of turning the
            // lock on mid-run (hot-reloading `|> Physics.upright` onto a live
            // capsule), so cancel the spin too.
            if next.rotation_locked {
                rb.set_angvel(Vector::ZERO, true);
            }
        }
        // `authority` is inert in Phase 1: cached (by reconcile) but never
        // written to the live world.
        true
    }
}

/// A blast's fraction at `t = distance / radius` in `0..=1`.
fn falloff(falloff: Falloff, t: f32) -> f32 {
    let linear = (1.0 - t).clamp(0.0, 1.0);
    match falloff {
        Falloff::Linear => linear,
        Falloff::Quadratic => linear * linear,
    }
}

/// The unit push from `center` toward `com`, biased up by `lift`. A body at
/// the centre goes straight up.
fn blast_direction(center: Vector, com: Vector, lift: f32) -> Vector {
    let radial = (com - center).try_normalize().unwrap_or(Vector::Y);
    (radial + Vector::Y * lift).try_normalize().unwrap_or(radial)
}

/// A smooth per-axis gust in `[-1, 1]` at `time` seconds: two sines per axis
/// whose 0.4–2 Hz frequencies and phases come from `seed`. Smooth, so light
/// bodies sway rather than jitter.
fn gust(seed: u64, time: f32) -> Vector {
    let mut rng = crate::net::Rng::new(seed);
    let mut wave = |weight: f32| {
        let hz = 0.4 + 1.6 * rng.next_f32();
        let phase = std::f32::consts::TAU * rng.next_f32();
        weight * (std::f32::consts::TAU * hz * time + phase).sin()
    };
    let mut axis = || wave(0.6) + wave(0.4);
    Vector::new(axis(), axis(), axis())
}

fn vec3(v: [f32; 3]) -> Vector {
    Vector::new(v[0], v[1], v[2])
}

/// `pose` with `live`'s translation and rotation where the declaration
/// didn't change them: a rebuilt body keeps what the simulation did.
fn keep_live(prev: &Body, next: &Body, mut pose: Pose, live: Pose) -> Pose {
    if prev.position == next.position {
        pose.translation = live.translation;
    }
    if prev.rotation == next.rotation {
        pose.rotation = live.rotation;
    }
    pose
}

fn pose_of(body: &Body) -> Pose {
    let rotation = unit_rotation(body.rotation);
    Pose::from_parts(vec3(body.position), rotation)
}

/// Bound the first terrain collision implementation to a 1025² grid. A 4096²
/// render source therefore keeps metre-scale visual detail without retaining
/// >100 MiB of collision data on Quest.
/// Streaming higher-resolution collision tiles is a follow-up.
const MAX_HEIGHTFIELD_CELLS_PER_AXIS: usize = 1024;

fn shape_is_ready(shape: &Shape) -> bool {
    !matches!(shape, Shape::Heightfield { data: None, .. })
}

/// Remove `tag` from a copy-on-write map, copying the map only if `tag` is in
/// it (a despawn scan visits tags a map may not hold).
fn cow_remove<V: Clone>(map: &mut Arc<BTreeMap<String, V>>, tag: &str) -> Option<V> {
    if !map.contains_key(tag) {
        return None;
    }
    Arc::make_mut(map).remove(tag)
}

/// A body that becomes a compound part instead of a body: fixed, still,
/// solid and convex (parry compounds can't nest a mesh).
fn fusable(body: &Body) -> bool {
    body.kind == BodyKind::Fixed
        && !body.sensor
        && body.velocity == [0.0; 3]
        && body.character.is_none()
        && matches!(
            body.shape,
            Shape::Cuboid { .. } | Shape::Sphere { .. } | Shape::Capsule { .. }
        )
}

/// Side of the grid cells fused bodies are grouped by (m).
const FUSE_CELL: f32 = 16.0;

/// The compound a fusable body joins: its contact material (`friction`,
/// `restitution` bits) and the [`FUSE_CELL`] cell of its declared position.
/// Cells keep a rebuild (an edit, a teleport) to one cell's parts.
type FuseKey = (u32, u32, [i32; 3]);

fn fuse_key(body: &Body) -> FuseKey {
    let cell = body.position.map(|x| (x / FUSE_CELL).floor() as i32);
    // `+ 0.0` turns -0.0 into 0.0: bodies equal by `==` share a key.
    (
        (body.friction + 0.0).to_bits(),
        (body.restitution + 0.0).to_bits(),
        cell,
    )
}

/// One key's fused fixed bodies: `parts` (not in rapier) has part `i`, at
/// its world pose, as body `tags[i]`; rapier collides them as one parentless
/// compound collider at the identity pose, boxes merged ([`merge_boxes`]),
/// whose subshape `merged[i]` holds part `i`.
#[derive(Clone, Serialize, Deserialize)]
struct FusedStatics {
    key: FuseKey,
    collider: ColliderHandle,
    /// Sorted (`fused_part` binary-searches them).
    tags: Vec<String>,
    parts: Arc<Compound>,
    merged: Vec<u32>,
}

/// Whether box faces at `p` and `q` coincide up to f32 rounding, so a merged
/// box covers no point its parts don't (a ray through a real crack still
/// finds a part).
fn coincide(p: f32, q: f32) -> bool {
    (p - q).abs() <= 8.0 * f32::EPSILON * p.abs().max(q.abs()).max(1.0)
}

/// Whether the contact patch spanned by `points` (a manifold's solver
/// contacts: a point, a segment or a polygon) meets `aabb`.
fn patch_meets(points: &[Vector], aabb: &Aabb) -> bool {
    use rapier3d::parry::shape::{Cuboid, Segment, Triangle};
    let at = Pose::from_translation(aabb.center());
    let cuboid = Cuboid::new(aabb.half_extents());
    let meets = |shape: &dyn rapier3d::parry::shape::Shape| {
        rapier3d::parry::query::intersection_test(&at, &cuboid, &Pose::IDENTITY, shape)
            .unwrap_or(false)
    };
    match points {
        [p] => aabb.contains_local_point(*p),
        [a, b] => meets(&Segment::new(*a, *b)),
        _ => (0..points.len()).any(|i| {
            (i + 1..points.len()).any(|j| {
                (j + 1..points.len())
                    .any(|k| meets(&Triangle::new(points[i], points[j], points[k])))
            })
        }),
    }
}

/// `parts` as rapier collides them, and each part's subshape there: unrotated
/// boxes with equal cross-sections that touch or overlap along an axis merge
/// into one box, so a body sliding across tiles meets no internal edge.
/// Greedy passes along x, z, then y: x then z merge a floor's tiles into rows,
/// then rows into a slab; y stacks a wall's courses. Other parts and unmerged
/// boxes pass through unchanged.
fn merge_boxes(parts: &[(Pose, SharedShape)]) -> (SharedShape, Vec<u32>) {
    // (min corner, max corner, member parts)
    let mut boxes: Vec<(Vector, Vector, Vec<usize>)> = Vec::new();
    let mut rest = Vec::new();
    for (i, (pose, shape)) in parts.iter().enumerate() {
        match shape.as_cuboid() {
            Some(c) if pose.rotation == Rotation::IDENTITY => {
                let (t, h) = (pose.translation, c.half_extents);
                boxes.push((t - h, t + h, vec![i]));
            }
            _ => rest.push(i),
        }
    }
    for (axis, a, b) in [(0, 1, 2), (2, 0, 1), (1, 0, 2)] {
        // Same cross-section next to each other, in order along `axis`.
        let key = |(lo, hi, _): &(Vector, Vector, Vec<usize>)| {
            [lo[a], hi[a], lo[b], hi[b], lo[axis]]
        };
        boxes.sort_by(|x, y| {
            let (x, y) = (key(x), key(y));
            x.iter().zip(&y).fold(std::cmp::Ordering::Equal, |o, (p, q)| o.then(p.total_cmp(q)))
        });
        let mut runs: Vec<(Vector, Vector, Vec<usize>)> = Vec::new();
        for next in boxes {
            match runs.last_mut() {
                Some((lo, hi, members))
                    if [a, b]
                        .iter()
                        .all(|&k| coincide(lo[k], next.0[k]) && coincide(hi[k], next.1[k]))
                        && (next.0[axis] <= hi[axis] || coincide(next.0[axis], hi[axis]))
                        && (next.1[axis] >= lo[axis] || coincide(next.1[axis], lo[axis])) =>
                {
                    lo[axis] = lo[axis].min(next.0[axis]);
                    hi[axis] = hi[axis].max(next.1[axis]);
                    members.extend(next.2);
                }
                _ => runs.push(next),
            }
        }
        boxes = runs;
    }
    let mut merged = vec![0; parts.len()];
    let mut shapes = Vec::new();
    for (lo, hi, members) in boxes {
        for &i in &members {
            merged[i] = shapes.len() as u32;
        }
        shapes.push(match members[..] {
            [only] => parts[only].clone(),
            _ => {
                let h = (hi - lo) / 2.0;
                (Pose::from_translation(lo + h), SharedShape::cuboid(h.x, h.y, h.z))
            }
        });
    }
    for i in rest {
        merged[i] = shapes.len() as u32;
        shapes.push(parts[i].clone());
    }
    (SharedShape::compound(shapes), merged)
}

/// The part of `compound` the ray hits first, skipping `skip`.
fn ray_on_parts(
    compound: &Compound,
    ray: &rapier3d::prelude::Ray,
    max: f32,
    skip: Option<usize>,
) -> Option<(usize, RayIntersection)> {
    let (part, _) = compound.bvh().cast_ray(ray, max, |i, best| {
        let (pose, shape) = &compound.shapes()[i as usize];
        (Some(i as usize) != skip)
            .then(|| shape.cast_ray(pose, ray, best, true))
            .flatten()
    })?;
    let (pose, shape) = &compound.shapes()[part as usize];
    Some((part as usize, shape.cast_ray_and_get_normal(pose, ray, max, true)?))
}

/// The part of `compound` that `shape` swept from `from` along unit `dir`
/// hits first, skipping `skip`; the hit is in world space, as
/// [`sweep`]'s.
fn sweep_parts(
    compound: &Compound,
    shape: &dyn rapier3d::parry::shape::Shape,
    from: &Pose,
    dir: Vector,
    max: f32,
    skip: Option<usize>,
) -> Option<(usize, rapier3d::parry::query::ShapeCastHit)> {
    let to = Pose::from_translation(dir * max) * *from;
    // An unbounded sweep (`max` infinite) may reach any part.
    let swept = match max.is_finite() {
        true => shape.compute_aabb(from).merged(&shape.compute_aabb(&to)),
        false => *compound.local_aabb(),
    };
    compound
        .bvh()
        .intersect_aabb(&swept)
        .map(|i| i as usize)
        .filter(|&i| Some(i) != skip)
        .filter_map(|i| {
            let (pose, part) = &compound.shapes()[i];
            let hit = rapier3d::parry::query::cast_shapes(
                pose,
                Vector::ZERO,
                &**part,
                from,
                dir,
                shape,
                cast_options(max, 0.0),
            )
            .ok()??;
            Some((i, hit.transform1_by(pose)))
        })
        .min_by(|a, b| a.1.time_of_impact.total_cmp(&b.1.time_of_impact))
}

/// Translation and rotation (per sweep) below which a [`World::sweep`] is a
/// stationary overlap test.
const STILL: f32 = 1.0e-6;

/// How far (at the shape's farthest point) a start overlap is probed along the
/// motion, and how much deeper it must get there to count as pressing rather
/// than sliding along the surface.
const PROBE: f32 = 1.0e-3;
const PRESSING: f32 = 1.0e-5;

/// Normalize a quaternion `[x, y, z, w]`: `from_xyzw` doesn't, and a
/// degenerate rotation (all zeros, or junk off the JSON boundary) would
/// NaN-poison the solver — which `snapshot` can't even flag (serde_json writes
/// non-finite floats as `null`, so the failure surfaces at `restore`, far from
/// the cause). Identity instead.
fn unit_rotation(q: [f32; 4]) -> Rotation {
    let q = Rotation::from_xyzw(q[0], q[1], q[2], q[3]);
    if q.length_squared().is_finite() && q.length_squared() > f32::EPSILON {
        q.normalize()
    } else {
        Rotation::IDENTITY
    }
}

fn rigid((p, q): RigidPose) -> Pose {
    Pose::from_parts(vec3(p), unit_rotation(q))
}

/// `from` to `to` over t in [0, 1]: a constant velocity and a constant
/// rotation about the shape's centre, the shorter way round.
fn motion_between(from: Pose, to: Pose) -> NonlinearRigidMotion {
    let turn = to.rotation * from.rotation.inverse();
    let turn = if turn.w < 0.0 { -turn } else { turn };
    NonlinearRigidMotion::new(from, Vector::ZERO, to.translation - from.translation, turn.to_scaled_axis())
}

/// `shape` at `at` overlapping `other` (at `pos`), as a fraction-0 hit — if
/// `motion` presses it deeper (any overlap when there is no motion).
fn pressed(
    pos: &Pose,
    other: &dyn rapier3d::parry::shape::Shape,
    shape: &dyn rapier3d::parry::shape::Shape,
    at: &Pose,
    motion: Option<&NonlinearRigidMotion>,
) -> Option<ShapeCastHit> {
    let contact = |at: &Pose| rapier3d::parry::query::contact(pos, other, at, shape, 0.0).ok().flatten();
    let c = contact(at)?;
    if let Some(m) = motion {
        // Deeper a little way on? (One witness can't tell: a box flush on a
        // face that turns presses in at one edge and lifts at the other.)
        let travel = m.linvel.length() + m.angvel.length() * shape.compute_local_bounding_sphere().radius;
        let probe = m.position_at_time((PROBE / travel).min(1.0));
        if !contact(&probe).is_some_and(|d| d.dist < c.dist - PRESSING) {
            return None;
        }
    }
    Some(ShapeCastHit {
        time_of_impact: 0.0,
        witness1: c.point1,
        witness2: c.point2,
        normal1: c.normal1,
        normal2: c.normal2,
        status: ShapeCastStatus::PenetratingOrWithinTargetDist,
    })
}

/// `shape`'s first hit on `other` (at `pos`) moving along `motion` over t in
/// [0, 1], in world space: fraction 0 if it starts [`pressed`] into it.
fn pair_hit(
    pos: &Pose,
    other: &dyn rapier3d::parry::shape::Shape,
    shape: &dyn rapier3d::parry::shape::Shape,
    at: &Pose,
    motion: Option<&NonlinearRigidMotion>,
) -> Option<ShapeCastHit> {
    pressed(pos, other, shape, at, motion).or_else(|| {
        let still = NonlinearRigidMotion::constant_position(*pos);
        let hit = rapier3d::parry::query::cast_shapes_nonlinear(&still, other, motion?, shape, 0.0, 1.0, false)
            .ok()??;
        Some(hit.transform1_by(pos))
    })
}

/// What `shape` can touch along `motion`: its bounding sphere about its
/// moving centre (it turns about that centre), or its AABB at `at` when still.
fn reach(shape: &dyn rapier3d::parry::shape::Shape, at: &Pose, motion: Option<&NonlinearRigidMotion>) -> Aabb {
    let Some(m) = motion else {
        return shape.compute_aabb(at);
    };
    let s = shape.compute_local_bounding_sphere();
    let r = Vector::splat(s.radius + s.center.length());
    let (a, b) = (at.translation, at.translation + m.linvel);
    Aabb::new(a.min(b) - r, a.max(b) + r)
}

/// The body's contact material, and contact begin/end reporting.
fn with_material(collider: ColliderBuilder, body: &Body) -> ColliderBuilder {
    collider
        .friction(body.friction)
        .restitution(body.restitution)
        .sensor(body.sensor)
        // Every body reports contact begin/end (Physics.events); rapier
        // only pays for this when a pair's state actually changes.
        .active_events(ActiveEvents::COLLISION_EVENTS)
}

/// A chunk's triangle mesh collider, built as `Physics.trimesh`'s is (one-sided
/// contacts, no density); `None` if rapier rejects the mesh.
fn chunk_trimesh(geometry: room::ChunkGeometry) -> Option<ColliderBuilder> {
    super::trimesh::shape(geometry.positions, geometry.triangles).map(trimesh_collider)
}

/// No density: an open mesh's volume-derived centre of mass is meaningless,
/// and a kinematic body's contact velocities use it.
fn trimesh_collider(shape: SharedShape) -> ColliderBuilder {
    ColliderBuilder::new(shape).density(0.0)
}

/// A fused room body's live colliders.
#[derive(Clone, Default, Serialize, Deserialize)]
struct LiveRoom {
    #[serde(with = "room::pairs")]
    chunks: BTreeMap<ChunkKey, ColliderHandle>,
    /// The floor height, and the plane there that replaces the chunks' floor.
    floor: Option<f32>,
    floor_plane: Option<ColliderHandle>,
}

fn collider_of(shape: &Shape) -> Option<ColliderBuilder> {
    match shape {
        Shape::Cuboid { extents } => Some(ColliderBuilder::cuboid(
            extents[0] / 2.0,
            extents[1] / 2.0,
            extents[2] / 2.0,
        )),
        Shape::Sphere { radius } => Some(ColliderBuilder::ball(*radius)),
        Shape::Capsule {
            half_height,
            radius,
        } => Some(ColliderBuilder::capsule_y(*half_height, *radius)),
        Shape::TriMesh { mesh } => Some(trimesh_collider(mesh.shape())),
        Shape::Heightfield { geometry, data } => data
            .as_deref()
            .map(|data| heightfield_collider(geometry, data)),
        // One collider per chunk (`World::sync_room`).
        Shape::FusedRoom { .. } => None,
    }
}

/// A sweep that stops `target` short of what it hits. A start overlap is a
/// hit at 0 unless the sweep moves out of it.
fn sweep(
    queries: &QueryPipeline,
    shape: &dyn rapier3d::parry::shape::Shape,
    from: &Pose,
    dir: Vector,
    max: f32,
    target: f32,
) -> Option<(ColliderHandle, rapier3d::parry::query::ShapeCastHit)> {
    queries.cast_shape(from, dir, shape, cast_options(max, target))
}

fn cast_options(max: f32, target: f32) -> ShapeCastOptions {
    ShapeCastOptions {
        max_time_of_impact: max,
        target_distance: target,
        stop_at_penetration: false,
        compute_impact_geometry_on_penetration: true,
    }
}

/// Resting on an upward-facing contact within the offset plus 5 cm (the
/// controller's own contact reach), unless the face there is steeper than the
/// slope limit. The face comes from a short ray at the contact point, because a
/// capsule rolling off a ledge touches its edge at a slant. The sweep starts
/// one offset up, so a capsule resting just inside its gap still hits cleanly.
fn on_walkable_ground(
    queries: &QueryPipeline,
    shape: &dyn rapier3d::parry::shape::Shape,
    at: &Pose,
    s: &CharacterSettings,
) -> bool {
    let from = Pose::from_translation(Vector::Y * s.offset) * *at;
    let Some((_, hit)) = sweep(queries, shape, &from, -Vector::Y, 2.0 * s.offset + 0.05, 0.0)
    else {
        return false;
    };
    let face = Ray::new(hit.witness1 + Vector::Y * 0.05, -Vector::Y);
    let steep = queries
        .cast_ray_and_get_normal(&face, 0.1, true)
        .is_some_and(|(_, ray)| ray.normal.y < s.max_slope.cos() - 1.0e-4);
    hit.normal1.y > 1.0e-3 && !steep
}

/// Step-up probe for a grounded move that was mostly blocked: lift by the
/// headroom (at most `step_height`), move far enough forward to stand past the
/// riser, and drop onto the tread. Returns the extra translation, or `None`
/// for a wall, a step too tall or a tread too steep.
fn step_up(
    queries: &QueryPipeline,
    shape: &dyn rapier3d::parry::shape::Shape,
    at: &Pose,
    desired: Vector,
    applied: Vector,
    s: &CharacterSettings,
) -> Option<Vector> {
    let want = Vector::new(desired.x, 0.0, desired.z);
    let len = want.length();
    if len < 1.0e-6 {
        return None;
    }
    let dir = want / len;
    if applied.dot(dir) > 0.5 * len {
        return None; // most of the move went through
    }
    let lift = sweep(queries, shape, at, Vector::Y, s.step_height, s.offset)
        .map_or(s.step_height, |(_, hit)| hit.time_of_impact);
    let lifted = Pose::from_translation(Vector::Y * lift) * *at;
    // The capsule's bottom must clear the riser's edge, or the next move's
    // snap pulls it back down.
    let forward = shape.as_capsule()?.radius + 4.0 * s.offset;
    if sweep(queries, shape, &lifted, dir, forward, 0.0).is_some() {
        return None;
    }
    let planted = Pose::from_translation(dir * forward) * lifted;
    let (_, tread) = sweep(queries, shape, &planted, -Vector::Y, lift, s.offset)?;
    let rise = lift - tread.time_of_impact;
    if rise <= s.offset || tread.normal1.y < s.max_slope.cos() {
        return None;
    }
    let step = Vector::Y * rise + dir * forward;
    let landed = Pose::from_translation(step) * *at;
    queries.intersect_shape(landed, shape).next().is_none().then_some(step)
}

fn heightfield_collider(
    terrain: &crate::terrain::TerrainGeometry,
    data: &crate::asset::pipelines::HeightmapData,
) -> ColliderBuilder {
    debug_assert!(data.is_valid());
    let (cells_x, cells_z) = collision_cell_counts(data);
    let height_range = terrain.max_height - terrain.min_height;
    let matrix = Array2::from_fn(cells_z + 1, cells_x + 1, |row, col| {
        sample_height_for_collision(data, col, cells_x, row, cells_z)
    });
    let shape = SharedShape::heightfield_with_flags(
        matrix,
        Vector::new(terrain.width, height_range, terrain.depth),
        HeightFieldFlags::FIX_INTERNAL_EDGES,
    );
    ColliderBuilder::new(shape).translation(Vector::new(0.0, terrain.min_height, 0.0))
}

fn sample_height_for_collision(
    data: &crate::asset::pipelines::HeightmapData,
    target_x: usize,
    target_cells_x: usize,
    target_z: usize,
    target_cells_z: usize,
) -> f32 {
    let source_cells_x = data.width.saturating_sub(1) as usize;
    let source_cells_z = data.height.saturating_sub(1) as usize;
    let source_x = target_x as f64 * source_cells_x as f64 / target_cells_x as f64;
    let source_z = target_z as f64 * source_cells_z as f64 / target_cells_z as f64;
    let x0 = source_x.floor() as usize;
    let z0 = source_z.floor() as usize;
    let x1 = (x0 + 1).min(source_cells_x);
    let z1 = (z0 + 1).min(source_cells_z);
    let fx = (source_x - x0 as f64) as f32;
    let fz = (source_z - z0 as f64) as f32;
    let width = data.width as usize;
    let height = |x, z| data.samples[z * width + x] as f32 / 65535.0;
    let (h00, h10, h01, h11) = (
        height(x0, z0),
        height(x1, z0),
        height(x0, z1),
        height(x1, z1),
    );
    if fx + fz <= 1.0 {
        h00 + fx * (h10 - h00) + fz * (h01 - h00)
    } else {
        h11 + (1.0 - fx) * (h01 - h11) + (1.0 - fz) * (h10 - h11)
    }
}

fn collision_cell_counts(data: &crate::asset::pipelines::HeightmapData) -> (usize, usize) {
    (
        (data.width.saturating_sub(1) as usize).min(MAX_HEIGHTFIELD_CELLS_PER_AXIS),
        (data.height.saturating_sub(1) as usize).min(MAX_HEIGHTFIELD_CELLS_PER_AXIS),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn flat_heightfield(width: u32, height: u32, sample: u16) -> Shape {
        Shape::Heightfield {
            geometry: crate::terrain::TerrainGeometry {
                source: crate::terrain::TerrainSource {
                    locator: "test.png".to_string(),
                    while_pending: Vec::new(),
                },
                width: 20.0,
                depth: 20.0,
                min_height: 0.0,
                max_height: 10.0,
            },
            data: Some(Arc::new(crate::asset::pipelines::HeightmapData {
                samples: vec![sample; (width * height) as usize],
                width,
                height,
                revision: 1,
            })),
        }
    }

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
    fn heightfield_raycast_matches_declared_elevation() {
        let terrain = flat_heightfield(260, 260, 32768);
        let Shape::Heightfield {
            data: Some(data), ..
        } = &terrain
        else {
            unreachable!()
        };
        assert_eq!(collision_cell_counts(data), (259, 259));

        let mut world = World::new([0.0, -9.81, 0.0]);
        world.reconcile(&scene(vec![Body::fixed("terrain".to_string(), terrain)]));
        world.step_frame(FIXED_DT);
        let hit = world
            .raycast([0.0, 20.0, 0.0], [0.0, -1.0, 0.0], 30.0)
            .expect("ray should hit the heightfield");
        assert_eq!(hit.tag, "terrain");
        assert!((hit.position[1] - 5.0).abs() < 0.01, "{hit:?}");
        assert!(hit.normal[1] > 0.99, "{hit:?}");
    }

    // A capsule dropped onto terrain must come to rest at the elevation the
    // heightmap declares. This is the end-to-end check that the collision
    // surface agrees with the rendered one: `Scene.terrain` and
    // `Physics.heightfield` derive from the same samples, but nothing else
    // asserts that a body standing on the result is where the map says the
    // ground is.
    #[test]
    fn a_capsule_settles_on_the_heightfield_at_the_declared_elevation() {
        const HALF_HEIGHT: f32 = 0.9;
        const RADIUS: f32 = 0.4;
        // Uniform samples, so the expected ground height is exact rather than
        // an interpolation of a slope: 0.25 of a 0..40 m range = 10 m.
        let terrain = flat_heightfield(64, 64, 16384);
        let Shape::Heightfield { geometry, .. } = &terrain else {
            unreachable!()
        };
        let expected_ground =
            geometry.min_height + 0.25 * (geometry.max_height - geometry.min_height);

        let mut world = World::new([0.0, -9.81, 0.0]);
        let scene = PhysicsScene::create(
            [0.0, -9.81, 0.0],
            vec![
                Body::fixed("terrain".to_string(), terrain.clone()),
                Body::dynamic(
                    "walker".to_string(),
                    Shape::Capsule {
                        half_height: HALF_HEIGHT,
                        radius: RADIUS,
                    },
                )
                .at([0.0, expected_ground + 6.0, 0.0])
                .as_upright(),
            ],
        );
        world.reconcile(&scene);

        // Long enough to fall 6 m and for the contact to settle.
        for _ in 0..240 {
            world.step_frame(FIXED_DT);
        }

        let (position, _) = world
            .body_transform("walker")
            .expect("the walker should still be in the world");
        // A capsule rests centre-height above the surface: half the segment
        // plus the cap radius.
        let resting_centre = expected_ground + HALF_HEIGHT + RADIUS;
        assert!(
            (position[1] - resting_centre).abs() < 0.1,
            "settled at y={:.3}, expected ~{resting_centre:.3} (ground {expected_ground:.3})",
            position[1]
        );
        // It must come to rest, not tunnel through or jitter on the surface.
        assert!(
            position[1] > expected_ground,
            "fell through the heightfield: y={:.3}",
            position[1]
        );
    }

    #[test]
    fn heightfield_uses_rapier_triangles_not_bilinear_interpolation() {
        let shape = Shape::Heightfield {
            geometry: crate::terrain::TerrainGeometry {
                source: crate::terrain::TerrainSource {
                    locator: "triangle-test.png".to_string(),
                    while_pending: Vec::new(),
                },
                width: 20.0,
                depth: 20.0,
                min_height: 0.0,
                max_height: 10.0,
            },
            data: Some(Arc::new(crate::asset::pipelines::HeightmapData {
                // Only the far (+X,+Z) corner is raised.
                samples: vec![0, 0, 0, 65535],
                width: 2,
                height: 2,
                revision: 1,
            })),
        };
        let mut world = World::new([0.0, 0.0, 0.0]);
        world.reconcile(&scene(vec![Body::fixed("terrain".to_string(), shape)]));
        world.step_frame(FIXED_DT);

        let lower = world
            .raycast([-5.0, 20.0, -5.0], [0.0, -1.0, 0.0], 30.0)
            .expect("lower triangle");
        let upper = world
            .raycast([5.0, 20.0, 5.0], [0.0, -1.0, 0.0], 30.0)
            .expect("upper triangle");
        assert!(lower.position[1].abs() < 0.001, "{lower:?}");
        assert!((upper.position[1] - 5.0).abs() < 0.001, "{upper:?}");
    }

    #[test]
    fn rectangular_heightfield_keeps_source_x_on_world_x() {
        let shape = Shape::Heightfield {
            geometry: crate::terrain::TerrainGeometry {
                source: crate::terrain::TerrainSource {
                    locator: "axis-test.png".to_string(),
                    while_pending: Vec::new(),
                },
                width: 30.0,
                depth: 10.0,
                min_height: 0.0,
                max_height: 10.0,
            },
            data: Some(Arc::new(crate::asset::pipelines::HeightmapData {
                // A left-to-right ramp repeated along Z. Rectangular source
                // dimensions make an accidental row/X transposition visible.
                samples: vec![0, 32768, 65535, 0, 32768, 65535],
                width: 3,
                height: 2,
                revision: 1,
            })),
        };
        let mut world = World::new([0.0, 0.0, 0.0]);
        world.reconcile(&scene(vec![Body::fixed("terrain".to_string(), shape)]));
        world.step_frame(FIXED_DT);

        let left = world
            .raycast([-7.5, 20.0, 0.0], [0.0, -1.0, 0.0], 30.0)
            .expect("left side of the X ramp");
        let right = world
            .raycast([7.5, 20.0, 0.0], [0.0, -1.0, 0.0], 30.0)
            .expect("right side of the X ramp");
        assert!((left.position[1] - 2.5).abs() < 0.001, "{left:?}");
        assert!((right.position[1] - 7.5).abs() < 0.001, "{right:?}");
    }

    #[test]
    fn capped_heightfield_samples_exact_fractional_source_positions() {
        let data = crate::asset::pipelines::HeightmapData {
            // Along X: 0, 0, 1, 1. The middle of this three-cell source is
            // halfway up the discontinuity; flooring would incorrectly pick 0.
            samples: vec![0, 0, 65535, 65535, 0, 0, 65535, 65535],
            width: 4,
            height: 2,
            revision: 1,
        };
        let middle = sample_height_for_collision(&data, 1, 2, 0, 1);
        assert!((middle - 0.5).abs() < 0.0001, "{middle}");
    }

    #[test]
    fn pending_heightfield_defers_spawn_and_preserves_the_last_settled_surface() {
        let ready = flat_heightfield(3, 3, 32768);
        let mut pending = ready.clone();
        let Shape::Heightfield { data, .. } = &mut pending else {
            unreachable!()
        };
        *data = None;

        let mut world = World::new([0.0, -9.81, 0.0]);
        world.reconcile(&scene(vec![Body::fixed(
            "terrain".to_string(),
            pending.clone(),
        )]));
        assert!(world.body_transform("terrain").is_none());

        world.reconcile(&scene(vec![Body::fixed("terrain".to_string(), ready)]));
        assert!(world.body_transform("terrain").is_some());

        world.reconcile(&scene(vec![Body::fixed("terrain".to_string(), pending)]));
        assert!(world.body_transform("terrain").is_some());
    }

    #[test]
    fn collision_resolution_and_timeline_heightfield_memory_are_bounded() {
        let oversized = crate::asset::pipelines::HeightmapData {
            samples: Vec::new(),
            width: 4097,
            height: 4097,
            revision: 1,
        };
        assert_eq!(collision_cell_counts(&oversized), (1024, 1024));

        // Exercise the actual cap, not just its arithmetic: the full retained
        // collider is one shared shape and remains shared by checkpoints.
        let terrain = flat_heightfield(1025, 1025, 32768);
        let mut world = World::new([0.0, -9.81, 0.0]);
        world.reconcile(&scene(vec![Body::fixed("terrain".to_string(), terrain)]));
        assert_eq!(world.colliders.len(), 1);
        let checkpoint = world.checkpoint();
        let live_shape = world.colliders.iter().next().unwrap().1.shared_shape();
        let saved_shape = checkpoint
            .world
            .colliders
            .iter()
            .next()
            .unwrap()
            .1
            .shared_shape();
        assert!(Arc::ptr_eq(&live_shape.0, &saved_shape.0));
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
    fn rotation_only_divergence_preserves_live_translation() {
        let quarter_turn = std::f32::consts::FRAC_1_SQRT_2;
        let rotated = [0.0, quarter_turn, 0.0, quarter_turn];

        for structural in [false, true] {
            let mut w = World::new([0.0, -9.81, 0.0]);
            let original = crate_at("a", [0.0, 5.0, 0.0]);
            w.reconcile(&scene(vec![original.clone()]));
            for _ in 0..30 {
                w.step_fixed();
            }
            let (fallen, _) = w.body_transform("a").unwrap();
            assert!(fallen[1] < 5.0);

            let changed = if structural {
                Body::dynamic(
                    "a".to_string(),
                    Shape::Sphere { radius: 0.5 },
                )
                .at([0.0, 5.0, 0.0])
                .facing(rotated)
            } else {
                original.facing(rotated)
            };
            w.reconcile(&scene(vec![changed]));

            let (position, rotation) = w.body_transform("a").unwrap();
            assert_eq!(
                position, fallen,
                "rotation-only divergence reset translation (structural={structural})"
            );
            for (actual, expected) in rotation.into_iter().zip(rotated) {
                assert!((actual - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn kinematic_rotation_crosses_180_degrees_on_the_short_arc() {
        let half_179_degrees = 179.0_f32.to_radians() / 2.0;
        let (sin, cos) = half_179_degrees.sin_cos();
        let before = [0.0, sin, 0.0, cos];
        // The canonical rotation builder wraps 181° to -179° so `w` stays
        // positive. This is the same orientation as +181°, but lies on the
        // opposite quaternion hemisphere from `before`.
        let after = [0.0, -sin, 0.0, cos];
        let body = |rotation| {
            Body::kinematic(
                "spinner".to_string(),
                Shape::Cuboid {
                    extents: [1.0, 1.0, 1.0],
                },
            )
            .facing(rotation)
        };

        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![body(before)]));
        w.reconcile(&scene(vec![body(after)]));
        w.step_fixed();

        let rb_handle = w.tags["spinner"];
        let angular_velocity = w.bodies[rb_handle].angvel().y;
        let expected = 2.0_f32.to_radians() / FIXED_DT;
        assert!(
            (angular_velocity - expected).abs() < 0.001,
            "expected a +2° short-arc step ({expected:.3} rad/s), got \
             {angular_velocity:.3} rad/s"
        );
    }

    #[test]
    fn kinematic_rotation_target_can_be_canceled_before_a_step() {
        let quarter_turn = std::f32::consts::FRAC_1_SQRT_2;
        let identity = [0.0, 0.0, 0.0, 1.0];
        let turned = [0.0, quarter_turn, 0.0, quarter_turn];
        let body = |rotation| {
            Body::kinematic(
                "spinner".to_string(),
                Shape::Cuboid {
                    extents: [1.0, 1.0, 1.0],
                },
            )
            .facing(rotation)
        };

        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![body(identity)]));
        w.reconcile(&scene(vec![body(turned)]));
        w.reconcile(&scene(vec![body(identity)]));
        w.step_fixed();

        let (_, rotation) = w.body_transform("spinner").unwrap();
        for (actual, expected) in rotation.into_iter().zip(identity) {
            assert!((actual - expected).abs() < 1e-6);
        }
        let rb_handle = w.tags["spinner"];
        assert!(w.bodies[rb_handle].angvel().length() < 1e-6);
    }

    #[test]
    fn structural_rebuild_preserves_and_retargets_kinematic_pose() {
        let quarter_turn = std::f32::consts::FRAC_1_SQRT_2;
        let identity = [0.0, 0.0, 0.0, 1.0];
        let turned = [0.0, quarter_turn, 0.0, quarter_turn];
        let cuboid = |rotation| {
            Body::kinematic(
                "spinner".to_string(),
                Shape::Cuboid {
                    extents: [1.0, 1.0, 1.0],
                },
            )
            .facing(rotation)
        };
        let sphere = |rotation| {
            Body::kinematic("spinner".to_string(), Shape::Sphere { radius: 1.0 })
                .facing(rotation)
        };

        // An unchanged declaration field retains its pending target when a
        // shape change rebuilds the body.
        let mut preserved = World::new([0.0, 0.0, 0.0]);
        preserved.reconcile(&scene(vec![cuboid(identity)]));
        preserved.reconcile(&scene(vec![cuboid(turned)]));
        preserved.reconcile(&scene(vec![sphere(turned)]));
        let (_, before_step) = preserved.body_transform("spinner").unwrap();
        assert_eq!(before_step, identity);
        preserved.step_fixed();
        let (_, after_step) = preserved.body_transform("spinner").unwrap();
        for (actual, expected) in after_step.into_iter().zip(turned) {
            assert!((actual - expected).abs() < 1e-6);
        }

        // A changed field alongside the rebuild replaces the pending target;
        // it does not teleport immediately or leave the old target queued.
        let mut retargeted = World::new([0.0, 0.0, 0.0]);
        retargeted.reconcile(&scene(vec![cuboid(identity)]));
        retargeted.reconcile(&scene(vec![cuboid(turned)]));
        retargeted.reconcile(&scene(vec![sphere(identity)]));
        let (_, before_step) = retargeted.body_transform("spinner").unwrap();
        assert_eq!(before_step, identity);
        retargeted.step_fixed();
        let (_, after_step) = retargeted.body_transform("spinner").unwrap();
        assert_eq!(after_step, identity);
        let rb_handle = retargeted.tags["spinner"];
        assert!(retargeted.bodies[rb_handle].angvel().length() < 1e-6);
    }

    #[test]
    fn kind_change_to_kinematic_seeds_target_from_live_pose() {
        let quarter_turn = std::f32::consts::FRAC_1_SQRT_2;
        let shape = Shape::Cuboid {
            extents: [1.0, 1.0, 1.0],
        };
        let live_pose = pose_of(
            &Body::kinematic("pose".to_string(), shape.clone())
                .at([2.0, 3.0, 4.0])
                .facing([0.0, quarter_turn, 0.0, quarter_turn]),
        );

        for old_kind in [BodyKind::Dynamic, BodyKind::Fixed] {
            let original = match old_kind {
                BodyKind::Dynamic => Body::dynamic("mover".to_string(), shape.clone()),
                BodyKind::Fixed => Body::fixed("mover".to_string(), shape.clone()),
                BodyKind::Kinematic => unreachable!(),
            };
            let mut w = World::new([0.0, 0.0, 0.0]);
            w.reconcile(&scene(vec![original]));
            match w.tags.get("mover") {
                Some(&rb) => w.bodies[rb].set_position(live_pose, true),
                // A fixed body is a compound part: move the part.
                None => {
                    let (c, part) = w.fused_part("mover").unwrap();
                    w.set_part_pose(c, part, live_pose);
                }
            }

            // The kind changes, but neither declared pose field does. The new
            // kinematic body's current and next poses must both inherit the
            // old body's live pose; a non-kinematic next_position is not a
            // defined source for the target.
            w.reconcile(&scene(vec![Body::kinematic(
                "mover".to_string(),
                shape.clone(),
            )]));
            let kinematic_handle = w.tags["mover"];
            assert_eq!(*w.bodies[kinematic_handle].position(), live_pose);
            assert_eq!(*w.bodies[kinematic_handle].next_position(), live_pose);
            w.step_fixed();
            assert_eq!(*w.bodies[kinematic_handle].position(), live_pose);
        }
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

    // Angular velocity is not declarable and has no command, so these tests
    // reach into the live body directly.
    fn spin_up(w: &mut World, tag: &str, angvel: [f32; 3]) {
        let rb = w.tags[tag];
        w.bodies[rb].set_angvel(Vector::new(angvel[0], angvel[1], angvel[2]), true);
    }

    fn spin_of(w: &World, tag: &str) -> f32 {
        let rb = w.tags[tag];
        w.bodies[rb].angvel().length()
    }

    // An upright body must not tip. Locking the axes stops the solver from
    // ACCUMULATING spin, but it does not cancel spin already present -- so
    // switching the lock on (hot-reloading `|> Physics.upright` onto a live,
    // tumbling capsule) has to zero the angular velocity too, or the body
    // rotates forever at a fixed rate.
    #[test]
    fn turning_upright_on_cancels_existing_spin() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        spin_up(&mut w, "a", [0.0, 4.0, 0.0]);
        assert!(spin_of(&w, "a") > 1.0, "spin was not set up");

        // Re-declare the SAME body as upright: the ordinary reconcile path.
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0]).as_upright()]));
        assert_eq!(spin_of(&w, "a"), 0.0);
        for _ in 0..30 {
            w.step_fixed();
        }
        assert_eq!(spin_of(&w, "a"), 0.0, "an upright body regained spin");
    }

    // The same through the STRUCTURAL rebuild path: a mass change despawns and
    // respawns the body, then restores its live velocities -- which must not
    // hand the old spin back to a now-locked body.
    #[test]
    fn an_upright_body_rebuilt_structurally_does_not_regain_spin() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        spin_up(&mut w, "a", [0.0, 4.0, 0.0]);
        assert!(spin_of(&w, "a") > 1.0);

        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])
            .as_upright()
            .with_mass(2.0)]));
        assert_eq!(spin_of(&w, "a"), 0.0);
    }

    // A body that is NOT upright keeps its spin across a structural rebuild --
    // the pre-existing behavior the fix above must leave alone.
    #[test]
    fn a_free_body_keeps_its_spin_across_a_rebuild() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])]));
        spin_up(&mut w, "a", [0.0, 4.0, 0.0]);
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0]).with_mass(2.0)]));
        assert!(spin_of(&w, "a") > 1.0);
    }

    // Damping is a RIGID-BODY property (friction/restitution are collider
    // material properties), so it has to reach the body builder at spawn.
    // Default 0.0 = no damping: existing games are unchanged.
    #[test]
    fn damping_is_applied_at_spawn_and_defaults_to_none() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![
            crate_at("damped", [0.0, 5.0, 0.0])
                .with_linear_damping(0.5)
                .with_angular_damping(0.25),
            crate_at("free", [5.0, 5.0, 0.0]),
        ]));
        assert_eq!(w.bodies[w.tags["damped"]].linear_damping(), 0.5);
        assert_eq!(w.bodies[w.tags["damped"]].angular_damping(), 0.25);
        assert_eq!(w.bodies[w.tags["free"]].linear_damping(), 0.0);
        assert_eq!(w.bodies[w.tags["free"]].angular_damping(), 0.0);
    }

    // A body coasting with no gravity keeps its velocity forever without
    // damping, and bleeds it off with damping — the marble-settles case.
    #[test]
    fn linear_damping_bleeds_off_velocity() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![
            crate_at("damped", [0.0, 5.0, 0.0])
                .with_velocity([4.0, 0.0, 0.0])
                .with_linear_damping(2.0),
            crate_at("free", [50.0, 5.0, 0.0]).with_velocity([4.0, 0.0, 0.0]),
        ]));
        for _ in 0..120 {
            w.step_fixed();
        }
        let damped = w.body_velocity("damped").unwrap()[0];
        let free = w.body_velocity("free").unwrap()[0];
        assert!((free - 4.0).abs() < 1e-3, "undamped body slowed: {free}");
        assert!(damped < 0.6, "damped body did not slow: {damped}");
    }

    // Angular damping is the other half: spin bleeds off.
    #[test]
    fn angular_damping_bleeds_off_spin() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![
            crate_at("a", [0.0, 5.0, 0.0]).with_angular_damping(3.0)
        ]));
        spin_up(&mut w, "a", [0.0, 4.0, 0.0]);
        for _ in 0..120 {
            w.step_fixed();
        }
        assert!(spin_of(&w, "a") < 0.2, "spin held: {}", spin_of(&w, "a"));
    }

    // Changing a declared damping value writes it onto the LIVE body (the
    // friction/restitution divergence rule) — no rebuild, no pose or velocity
    // reset; the new drag simply applies from the next step. And a STRUCTURAL
    // change (mass) respawns the body, which must re-derive damping from the
    // new declaration rather than lose it.
    #[test]
    fn changing_damping_reconciles_onto_the_live_body() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![
            crate_at("a", [0.0, 5.0, 0.0]).with_velocity([4.0, 0.0, 0.0])
        ]));
        for _ in 0..30 {
            w.step_fixed();
        }
        let moved = w.body_transform("a").unwrap().0;

        // Re-declare the SAME body with damping: the ordinary reconcile path.
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])
            .with_velocity([4.0, 0.0, 0.0])
            .with_linear_damping(2.0)
            .with_angular_damping(1.0)]));
        assert_eq!(w.bodies[w.tags["a"]].linear_damping(), 2.0);
        assert_eq!(w.bodies[w.tags["a"]].angular_damping(), 1.0);
        // Neither pose nor velocity was disturbed by the attribute change.
        assert_eq!(w.body_transform("a").unwrap().0, moved);
        assert!((w.body_velocity("a").unwrap()[0] - 4.0).abs() < 1e-4);

        for _ in 0..120 {
            w.step_fixed();
        }
        assert!(w.body_velocity("a").unwrap()[0] < 0.6);

        // A structural change (mass) despawns and respawns the body: damping
        // must come back with it.
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])
            .with_velocity([4.0, 0.0, 0.0])
            .with_linear_damping(2.0)
            .with_angular_damping(1.0)
            .with_mass(3.0)]));
        assert_eq!(w.bodies[w.tags["a"]].linear_damping(), 2.0);
        assert_eq!(w.bodies[w.tags["a"]].angular_damping(), 1.0);

        // …and taking the attribute back off restores the coast.
        w.reconcile(&scene(vec![crate_at("a", [0.0, 5.0, 0.0])
            .with_velocity([4.0, 0.0, 0.0])
            .with_mass(3.0)]));
        assert_eq!(w.bodies[w.tags["a"]].linear_damping(), 0.0);
        assert_eq!(w.bodies[w.tags["a"]].angular_damping(), 0.0);
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

    // ── per-axis velocity writes ────────────────────────────────────────

    fn set_xz(tag: &str, x: f32, z: f32) -> PhysicsCommand {
        PhysicsCommand::SetVelocityXZ {
            tag: tag.to_string(),
            x,
            z,
        }
    }

    fn set_y(tag: &str, y: f32) -> PhysicsCommand {
        PhysicsCommand::SetVelocityY {
            tag: tag.to_string(),
            y,
        }
    }

    /// A body in a gravity-free world with a known starting velocity, for the
    /// composition tests: with no gravity and no contacts the velocity after
    /// one step is exactly what the command queue left behind.
    fn drifting(velocity: [f32; 3]) -> World {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&PhysicsScene::create(
            [0.0, 0.0, 0.0],
            vec![crate_at("a", [0.0, 0.0, 0.0]).with_velocity(velocity)],
        ));
        w
    }

    #[test]
    fn set_velocity_xz_leaves_the_vertical_axis_alone() {
        let mut w = drifting([1.0, -5.0, 2.0]);
        w.queue_command(set_xz("a", 7.0, -7.0));
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_velocity("a").unwrap(), [7.0, -5.0, -7.0]);
        assert!(w.take_command_warnings().is_empty());
    }

    #[test]
    fn set_velocity_y_leaves_the_horizontal_plane_alone() {
        let mut w = drifting([1.0, -5.0, 2.0]);
        w.queue_command(set_y("a", 7.2));
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_velocity("a").unwrap(), [1.0, 7.2, 2.0]);
    }

    /// The composition rule: commands apply in queue order against the LIVE
    /// body, so the last write to a given axis wins and an axis nobody wrote
    /// is untouched. The two masks partition all three axes, so issuing both
    /// in one frame is exactly a whole-vector `SetVelocity`.
    #[test]
    fn same_frame_masked_writes_compose_per_axis() {
        // Disjoint masks: together they equal the full write.
        let mut w = drifting([1.0, -5.0, 2.0]);
        w.queue_command(set_xz("a", 3.0, 4.0));
        w.queue_command(set_y("a", 9.0));
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_velocity("a").unwrap(), [3.0, 9.0, 4.0]);

        // Order-independent, because the masks do not overlap.
        let mut w = drifting([1.0, -5.0, 2.0]);
        w.queue_command(set_y("a", 9.0));
        w.queue_command(set_xz("a", 3.0, 4.0));
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_velocity("a").unwrap(), [3.0, 9.0, 4.0]);

        // Overlapping masks: last wins, per axis.
        let mut w = drifting([1.0, -5.0, 2.0]);
        w.queue_command(set_xz("a", 3.0, 4.0));
        w.queue_command(set_xz("a", 6.0, 8.0));
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_velocity("a").unwrap(), [6.0, -5.0, 8.0]);

        // A masked write reads the axis it preserves from the live body, so a
        // whole-vector write queued EARLIER in the same frame is visible to it
        // — not the stale value the game read in `tick`.
        let mut w = drifting([1.0, -5.0, 2.0]);
        w.queue_command(PhysicsCommand::SetVelocity {
            tag: "a".to_string(),
            velocity: [0.0, 11.0, 0.0],
        });
        w.queue_command(set_xz("a", 3.0, 4.0));
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_velocity("a").unwrap(), [3.0, 11.0, 4.0]);

        // And the reverse order lets the full write clobber the masked one —
        // "last wins per axis" with the full mask covering everything.
        let mut w = drifting([1.0, -5.0, 2.0]);
        w.queue_command(set_xz("a", 3.0, 4.0));
        w.queue_command(PhysicsCommand::SetVelocity {
            tag: "a".to_string(),
            velocity: [0.0, 11.0, 0.0],
        });
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_velocity("a").unwrap(), [0.0, 11.0, 0.0]);
    }

    #[test]
    fn masked_writes_share_the_unknown_and_non_dynamic_warnings() {
        let mut w = World::new([0.0, 0.0, 0.0]);
        w.reconcile(&scene(vec![ground(), crate_at("a", [0.0, 5.0, 0.0])]));
        w.queue_command(set_xz("nobody", 1.0, 1.0));
        w.queue_command(set_y("ground", 1.0));
        w.step_frame(FIXED_DT);
        let warnings = w.take_command_warnings();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        // The warning must name the command the game actually called, not the
        // whole-vector one it is a sibling of.
        assert_eq!(
            warnings[0],
            "physics setVelocityXZ for unknown tag \"nobody\""
        );
        assert_eq!(
            warnings[1],
            "physics setVelocityY on non-dynamic body \"ground\" has no effect"
        );
    }

    /// The capsule's feet offset (half-height + radius), from
    /// `examples/physics-controller`.
    const FEET: f32 = 0.9;

    /// Drive a character capsule for `frames` fixed steps, calling `policy`
    /// each frame to queue whatever commands it wants. Returns the body's
    /// final `(x, y)`.
    ///
    /// Mirrors `examples/physics-controller`: gravity -22, an upright
    /// frictionless capsule (half-height 0.5 + radius 0.4, so the feet sit
    /// 0.9 below the center) standing on a fixed slab whose top is y = 0. The
    /// correct resting height is therefore y = 0.90.
    fn run_capsule(
        spawn_y: f32,
        frames: u32,
        dt: impl Fn(u32) -> f32,
        policy: impl Fn(&mut World),
    ) -> (f32, f32) {
        const GRAVITY: [f32; 3] = [0.0, -22.0, 0.0];
        let declared = PhysicsScene::create(
            GRAVITY,
            vec![
                Body::fixed(
                    "ground".to_string(),
                    Shape::Cuboid {
                        extents: [40.0, 1.0, 40.0],
                    },
                )
                .at([0.0, -0.5, 0.0])
                .with_friction(0.8),
                Body::dynamic(
                    "player".to_string(),
                    Shape::Capsule {
                        half_height: 0.5,
                        radius: 0.4,
                    },
                )
                .at([0.0, spawn_y, 0.0])
                .with_friction(0.0)
                .with_restitution(0.0)
                .with_mass(1.0)
                .as_upright(),
            ],
        );
        let mut w = World::new(GRAVITY);
        for frame in 0..frames {
            // Re-declared unchanged every frame, exactly as a `physics` hook
            // does: an unchanged declaration means the simulation owns the
            // body, so this is not a per-frame snap back to the spawn.
            w.reconcile(&declared);
            policy(&mut w);
            w.step_frame(dt(frame));
        }
        let (pos, _) = w.body_transform("player").unwrap();
        (pos[0], pos[1])
    }

    fn run_capsule_fixed(
        spawn_y: f32,
        frames: u32,
        policy: impl Fn(&mut World),
    ) -> (f32, f32) {
        run_capsule(spawn_y, frames, |_| FIXED_DT, policy)
    }

    /// A masked write is EXACTLY transparent on the axes it does not name:
    /// the body ends where the solver alone would have put it, so a game can
    /// steer the horizontal plane without having any opinion about the
    /// vertical one.
    ///
    /// That is the property the character-controller case needs, and it is
    /// what `Physics.setVelocity` cannot offer — a controller steering with
    /// the whole-vector command must author a `y` every frame, and the only
    /// values available to it are a stale read or a guess.
    ///
    /// Investigated while adding this: the two sinking failures reported in
    /// #503 (a capsule resting at 0.40 instead of 0.90) do NOT reproduce at
    /// this layer. Echoing the read back is bit-identical to issuing no
    /// command at all — the read and the command's apply point observe the
    /// same world state — and commanding `y = 0` while grounded recovers to
    /// the same resting height, because rapier resolves the landing's
    /// penetration positionally rather than through the velocity the game
    /// overwrites. Both were checked here after a real drop-and-land, under a
    /// fixed frame time and under a jittered one (0- and 2-substep frames).
    /// So this command's case is ergonomic and architectural, not a sinking
    /// fix; whatever sank in #503 lived in the example, not in the surface.
    #[test]
    fn masked_writes_are_transparent_on_the_untouched_axis() {
        // Dropped from a height, so it lands with real impact speed and the
        // solver has actual penetration to recover from.
        const SPAWN: f32 = 3.0;
        let frames = 260;
        // A NONZERO horizontal write, so this compares a real steering command
        // against no command — not one no-op against another. The capsule is
        // frictionless on flat ground, so driving x and z cannot itself change
        // the height the solver settles at.
        let steer = |w: &mut World| w.queue_command(set_xz("player", 4.0, -2.0));

        let (idle_x, untouched) = run_capsule_fixed(SPAWN, frames, |_| {});
        let (driven_x, masked) = run_capsule_fixed(SPAWN, frames, steer);
        // Tolerance, not bit-equality: the steered capsule really is sliding,
        // so the solver does slightly different contact work and the last
        // couple of float bits differ (~1e-7 here). Demanding an exact match
        // would also be a cross-platform trap — this runs on x86 and ARM CI.
        // The claim under test is that steering does not move the standing
        // height, and 1e-4 is ~4000x tighter than the 0.4-unit sink the
        // whole-vector command was supposed to cause.
        assert!(
            (masked - untouched).abs() < 1e-4,
            "a masked write moved the axis it does not name: {masked} vs {untouched}"
        );
        assert!(
            (masked - FEET).abs() < 0.02,
            "the capsule did not rest at its standing height: {masked}"
        );

        // The steered axis really was driven — otherwise the equality above
        // would be the trivial one, two runs that both commanded nothing.
        assert!(
            (idle_x).abs() < 1e-6,
            "the uncommanded capsule drifted in x: {idle_x}"
        );
        assert!(
            driven_x > 10.0,
            "the steered capsule did not travel: x = {driven_x}"
        );

        // Again under a JITTERED frame time, the way the real runtime drives
        // it: some frames take no substep and some take two, so a command can
        // sit pending across a frame and two can land before one substep.
        let jitter = |f: u32| FIXED_DT * (0.6 + 0.9 * ((f % 7) as f32 / 6.0));
        let (_, untouched) = run_capsule(SPAWN, frames, jitter, |_| {});
        let (jx, masked) = run_capsule(SPAWN, frames, jitter, steer);
        assert!(
            (masked - untouched).abs() < 1e-4,
            "a masked write moved the untouched axis under a jittered frame \
             time: {masked} vs {untouched}"
        );
        assert!(jx > 10.0, "the steered capsule did not travel: x = {jx}");

        // (The vertical mask's mirror property — a `y` write leaving x and z
        // untouched — is pinned exactly in
        // `set_velocity_y_leaves_the_horizontal_plane_alone`, in the
        // contact-free world where it is a clean no-op. Commanding `y` here
        // would not be a no-op: it cancels the fall and changes when the
        // capsule lands.)
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

    // ── Physics.memo groups ─────────────────────────────────────────────

    /// A tag-sorted group, as `Physics.memo` builds it.
    fn group(key: &str, mut bodies: Vec<Body>) -> super::super::BodyGroup {
        bodies.sort_by(|a, b| a.tag.cmp(&b.tag));
        super::super::BodyGroup {
            key: key.to_string(),
            bodies: bodies.into(),
        }
    }

    /// `n` fixed 1 m boxes tiling a floor at y = 0, tagged `s0000`….
    fn floor_tiles(n: usize) -> Vec<Body> {
        (0..n)
            .map(|i| {
                Body::fixed(
                    format!("s{i:04}"),
                    Shape::Cuboid {
                        extents: [1.0, 1.0, 1.0],
                    },
                )
                .at([(i % 30) as f32 - 15.0, 0.0, (i / 30) as f32 - 25.0])
            })
            .collect()
    }

    fn with_groups(bodies: Vec<Body>, groups: Vec<super::super::BodyGroup>) -> PhysicsScene {
        PhysicsScene {
            groups,
            ..scene(bodies)
        }
    }

    /// A step's events as `(a, b, started, sensor)` with `a <= b`, sorted:
    /// comparable whichever way round rapier named a pair.
    fn sorted_events(events: Vec<PhysicsEvent>) -> Vec<(String, String, bool, bool)> {
        let mut events: Vec<_> = events
            .into_iter()
            .map(|e| match e.a <= e.b {
                true => (e.a, e.b, e.started, e.sensor),
                false => (e.b, e.a, e.started, e.sensor),
            })
            .collect();
        events.sort();
        events
    }

    /// `body` as a still kinematic body: it collides as the fixed one does
    /// but never fuses, the baseline fused bodies are checked against.
    fn separate(body: Body) -> Body {
        Body {
            kind: BodyKind::Kinematic,
            ..body
        }
    }

    /// Bodies fused into compounds.
    fn fused_bodies(w: &World) -> usize {
        w.fused.iter().map(|f| f.tags.len()).sum()
    }

    #[test]
    fn an_unchanged_group_is_skipped_by_identity() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        let statics = group("statics", floor_tiles(1500));
        let frame = with_groups(vec![crate_at("a", [0.0, 3.0, 0.0])], vec![statics.clone()]);
        w.reconcile(&frame);
        assert_eq!(
            w.reconcile_stats(),
            ReconcileStats {
                compared: 1501,
                written: 1501,
                groups_skipped: 0
            }
        );
        for _ in 0..300 {
            w.step_frame(FIXED_DT);
            w.reconcile(&frame);
            assert_eq!(
                w.reconcile_stats(),
                ReconcileStats {
                    compared: 1,
                    written: 0,
                    groups_skipped: 1
                }
            );
        }
        assert_eq!((w.tags.len(), fused_bodies(&w)), (1, 1500));
    }

    #[test]
    fn a_rebuilt_equal_group_does_no_rapier_work() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&with_groups(
            vec![],
            vec![group("statics", floor_tiles(1500))],
        ));
        w.step_frame(FIXED_DT);
        let before = w.snapshot();
        // A hot reload rebuilds the site: a new `Arc`, equal bodies.
        w.reconcile(&with_groups(
            vec![],
            vec![group("statics", floor_tiles(1500))],
        ));
        assert_eq!(
            w.reconcile_stats(),
            ReconcileStats {
                compared: 1500,
                written: 0,
                groups_skipped: 0
            }
        );
        assert!(w.snapshot() == before);
    }

    #[test]
    fn a_changed_group_diffs_only_itself() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        let statics = group("statics", floor_tiles(1500));
        let doors = |y: f32| {
            group(
                "doors",
                (0..10)
                    .map(|i| {
                        Body::kinematic(format!("door{i}"), Shape::Sphere { radius: 0.5 })
                            .at([i as f32, y, 9.0])
                    })
                    .collect(),
            )
        };
        w.reconcile(&with_groups(vec![], vec![statics.clone(), doors(1.0)]));
        w.reconcile(&with_groups(vec![], vec![statics.clone(), doors(2.0)]));
        assert_eq!(
            w.reconcile_stats(),
            ReconcileStats {
                compared: 10,
                written: 10,
                groups_skipped: 1
            }
        );
        // Dropping a group despawns exactly its tags.
        w.reconcile(&with_groups(vec![], vec![statics]));
        assert_eq!((w.tags.len(), fused_bodies(&w)), (0, 1500));
        assert_eq!(w.declared.len(), 1500);
    }

    /// The world minus the reconcile bookkeeping that differs by how bodies
    /// were declared (flat vs grouped): rapier state, tags and the cache.
    fn sim_state(w: &World) -> Vec<u8> {
        let mut w = w.clone();
        w.flat = Arc::default();
        w.groups.clear();
        w.snapshot()
    }

    fn teleport_tile(w: &mut World, y: f32) {
        w.queue_command(PhysicsCommand::Teleport {
            tag: "s0000".to_string(),
            position: [0.0, y, 0.0],
        });
        w.step_frame(FIXED_DT);
    }

    #[test]
    fn a_tag_moving_between_flat_and_a_group_keeps_its_part() {
        let tile = || floor_tiles(1).remove(0);
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![tile()]));
        teleport_tile(&mut w, 1.0);
        let before = sim_state(&w);
        w.reconcile(&with_groups(vec![], vec![group("g", vec![tile()])]));
        assert!(sim_state(&w) == before);
        w.reconcile(&scene(vec![tile()]));
        assert!(sim_state(&w) == before);
        assert!(w.groups.is_empty());
        assert_eq!(w.body_transform("s0000").unwrap().0, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_part_becoming_a_body_and_back_keeps_its_pose() {
        let tile = || floor_tiles(1).remove(0);
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(vec![tile()]));
        teleport_tile(&mut w, 1.0);
        // Kinematic: a body where the part was.
        w.reconcile(&scene(vec![separate(tile())]));
        assert_eq!((w.tags.len(), fused_bodies(&w)), (1, 0));
        assert_eq!(w.body_transform("s0000").unwrap().0, [0.0, 1.0, 0.0]);
        teleport_tile(&mut w, 2.0);
        // Fixed again: a part where the body was.
        w.reconcile(&scene(vec![tile()]));
        assert_eq!((w.tags.len(), fused_bodies(&w)), (0, 1));
        assert_eq!(w.body_transform("s0000").unwrap().0, [0.0, 2.0, 0.0]);
        assert_eq!(w.colliders.len(), 1);
    }

    /// Floor tiles plus a box under tile `s0167`, fused or not.
    fn queried(fused: bool) -> World {
        let mut bodies = floor_tiles(300);
        bodies.push(
            Body::fixed("under".to_string(), Shape::Cuboid { extents: [1.0, 1.0, 1.0] })
                .at([2.0, -3.0, -20.0]),
        );
        if !fused {
            bodies = bodies.into_iter().map(separate).collect();
        }
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(bodies));
        w.step_frame(FIXED_DT);
        w
    }

    #[test]
    fn queries_on_fused_bodies_answer_as_unfused_ones() {
        let (flat, memo) = (queried(false), queried(true));
        assert_eq!((memo.tags.len(), fused_bodies(&memo)), (0, 301));
        let ball = Shape::Sphere { radius: 0.25 };
        for exclude in [None, Some("s0167"), Some("under"), Some("s0168")] {
            // Not on a seam: a tie there picks either tile, flat or fused.
            for x in [2.0, 2.4, 2.55, 3.1] {
                let (origin, down) = ([x, 5.0, -20.0], [0.0, -1.0, 0.0]);
                let ray = |w: &World| w.raycast_excluding(origin, down, 20.0, exclude);
                let cast = |w: &World| w.shape_cast(&ball, origin, down, 20.0, exclude);
                assert_eq!(ray(&flat), ray(&memo), "ray at {x} excluding {exclude:?}");
                assert_eq!(cast(&flat), cast(&memo), "cast at {x} excluding {exclude:?}");
            }
        }
        let hit = memo.raycast_excluding([2.0, 5.0, -20.0], [0.0, -1.0, 0.0], 20.0, Some("s0167"));
        assert_eq!(hit.unwrap().tag, "under");
    }

    #[test]
    fn sweeps_on_fused_bodies_answer_as_unfused_ones() {
        let (flat, memo) = (queried(false), queried(true));
        assert_eq!(fused_bodies(&memo), 301);
        let stick = Shape::Cuboid { extents: [0.1, 0.1, 1.2] };
        let posed = |x: f32, y: f32, pitch: f32| ([x, y, -20.0], [(pitch / 2.0).sin(), 0.0, 0.0, (pitch / 2.0).cos()]);
        for x in [2.0, 2.4, 2.55, 3.1] {
            // Falling and turning onto the tiles; tipping in place onto them;
            // already in them, at rest. (A stick spans tiles, so a tie may pick
            // either neighbour: compare where it stops, not the tag.)
            let cases = [
                (posed(x, 3.0, 0.0), posed(x, -1.0, 1.2)),
                (posed(x, 0.6, 0.0), posed(x, 0.6, 0.5)),
                (posed(x, 0.52, 0.0), posed(x, 0.52, 0.0)),
            ];
            for (from, to) in cases {
                // `All`: the unfused tiles are kinematic.
                let sweep = |w: &World| w.sweep(&stick, from, to, SweepFilter::All).map(|h| h.fraction);
                assert!(sweep(&memo).is_some(), "sweep at {x} hits");
                assert_eq!(sweep(&flat), sweep(&memo), "sweep at {x}");
            }
        }
    }

    #[test]
    fn a_sensor_reports_each_fused_body_it_passes() {
        let run = |fused: bool| {
            let mut tiles = floor_tiles(90);
            if !fused {
                tiles = tiles.into_iter().map(separate).collect();
            }
            let probe = Body {
                sensor: true,
                ..crate_at("probe", [0.5, 3.0, -24.0])
            };
            let frame = scene(tiles.into_iter().chain([probe]).collect());
            let mut w = World::new([0.0, -9.81, 0.0]);
            let mut events = Vec::new();
            for _ in 0..90 {
                w.reconcile(&frame);
                w.step_frame(FIXED_DT);
                events.push(sorted_events(w.take_events()));
            }
            events
        };
        let memo = run(true);
        assert_eq!(run(false), memo);
        let starts: Vec<_> = memo.iter().flatten().filter(|e| e.2 && e.3).collect();
        // On the x seam, touching the rows either side in z.
        assert_eq!(starts.len(), 6, "{starts:?}");
    }

    #[test]
    fn only_still_solid_convex_fixed_bodies_fuse_one_compound_per_material_and_cell() {
        let mut bodies = floor_tiles(4);
        bodies[1].friction = 0.9;
        bodies[3].position[0] = 5.0; // the next cell
        bodies.extend([
            Body::kinematic("door".to_string(), Shape::Sphere { radius: 0.5 }).at([0.0, 4.0, 0.0]),
            Body {
                sensor: true,
                ..Body::fixed("zone".to_string(), Shape::Sphere { radius: 0.5 })
            },
            Body {
                velocity: [1.0, 0.0, 0.0],
                ..Body::fixed("belt".to_string(), Shape::Sphere { radius: 0.5 })
            },
        ]);
        // Flat or grouped alike.
        let flat = bodies.drain(..2).collect();
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&with_groups(flat, vec![group("g", bodies)]));
        let compounds: Vec<_> = w.fused.iter().map(|f| f.tags.clone()).collect();
        assert_eq!(compounds, [vec!["s0000", "s0002"], vec!["s0003"], vec!["s0001"]]);
        assert_eq!(w.tags.keys().collect::<Vec<_>>(), ["belt", "door", "zone"]);
        assert_eq!(w.colliders.len(), 6);
    }

    #[test]
    fn a_friction_of_minus_zero_keeps_its_compound() {
        let tiles = |n: usize, friction: f32| -> Vec<Body> {
            floor_tiles(n).into_iter().map(|b| b.with_friction(friction)).collect()
        };
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(tiles(2, 0.0)));
        // Equal by `==`, so not an edit; dropping s0001 rebuilds the compound.
        w.reconcile(&scene(tiles(1, -0.0)));
        assert_eq!(fused_bodies(&w), 1);
        assert!(w.body_transform("s0000").is_some());
    }

    #[test]
    fn a_world_with_fused_bodies_restores_exactly() {
        let frame = with_groups(
            vec![crate_at("a", [0.3, 3.0, -20.2])],
            vec![group("g", floor_tiles(300))],
        );
        let mut w = World::new([0.0, -9.81, 0.0]);
        for _ in 0..50 {
            w.reconcile(&frame);
            w.step_frame(FIXED_DT);
        }
        assert!(!w.fused_touching.is_empty());
        let bytes = w.snapshot();
        let mut restored = World::new([0.0, 0.0, 0.0]);
        restored.restore(&bytes).unwrap();
        // Equal state, not bytes: parry caches a compound's per-part contacts
        // in a hash map, which a JSON round trip may reorder. (A checkpoint
        // clones it, so in-process restores stay byte-exact.)
        let state = |w: &World| serde_json::from_slice::<serde_json::Value>(&w.snapshot()).unwrap();
        assert_eq!(state(&restored), state(&w));
        let checkpoint = w.checkpoint();
        for _ in 0..40 {
            for w in [&mut w, &mut restored] {
                w.reconcile(&frame);
                w.step_frame(FIXED_DT);
            }
            assert_eq!(w.take_events(), restored.take_events());
        }
        assert_eq!(state(&restored), state(&w));
        let mut resumed = World::new([0.0, 0.0, 0.0]);
        resumed.restore_checkpoint(&checkpoint);
        for _ in 0..40 {
            resumed.reconcile(&frame);
            resumed.step_frame(FIXED_DT);
        }
        assert!(resumed.snapshot() == w.snapshot());
    }

    #[test]
    fn a_continuous_sensor_tunnelling_a_fused_wall_reports_it() {
        let run = |fused: bool| {
            let wall = Body::fixed("wall".to_string(), Shape::Cuboid { extents: [0.1, 4.0, 4.0] });
            let wall = if fused { wall } else { separate(wall) };
            let bullet = Body {
                sensor: true,
                continuous: true,
                velocity: [300.0, 0.0, 0.0],
                ..Body::dynamic("bullet".to_string(), Shape::Sphere { radius: 0.1 }).at([-2.0, 0.0, 0.0])
            };
            let frame = scene(vec![bullet, wall]);
            let mut w = World::new([0.0, 0.0, 0.0]);
            let mut events = Vec::new();
            for _ in 0..3 {
                w.reconcile(&frame);
                w.step_frame(FIXED_DT);
                events.extend(w.take_events().into_iter().map(|e| (e.started, e.sensor)));
            }
            events
        };
        assert_eq!(run(false), [(true, true), (false, true)]);
        assert_eq!(run(true), run(false));
    }

    /// A crate resting on tile `s0000` while the tile is a kinematic body for
    /// frames 60..90 (and moves to `moved.1` at frame `moved.0`): the events.
    fn handed_over(moved: Option<(u32, [f32; 3])>) -> Vec<(String, String, bool, bool)> {
        let crate_ = crate_at("crate", [-15.0, 1.5, -25.0]);
        let tile = |frame: u32| {
            let tile = floor_tiles(1).remove(0);
            match moved {
                Some((from, at)) if frame >= from => tile.at(at),
                _ => tile,
            }
        };
        let mut w = World::new([0.0, -9.81, 0.0]);
        let mut events = Vec::new();
        for frame in 0..120 {
            let tile = match (60..90).contains(&frame) {
                true => separate(tile(frame)),
                false => tile(frame),
            };
            w.reconcile(&scene(vec![crate_.clone(), tile]));
            w.step_frame(FIXED_DT);
            events.extend(sorted_events(w.take_events()));
        }
        events
    }

    #[test]
    fn a_resting_contact_survives_its_tile_moving_in_and_out_of_a_compound() {
        let touch = |started| ("crate".to_string(), "s0000".to_string(), started, false);
        assert_eq!(handed_over(None), [touch(true)]);
        // Moved out from under the crate as it leaves or re-joins the
        // compound: one stop.
        for frame in [60, 90] {
            let away = Some((frame, [-10.0, 0.0, -20.0]));
            assert_eq!(handed_over(away), [touch(true), touch(false)], "at {frame}");
        }
    }

    #[test]
    fn an_unready_replacement_keeps_the_fused_body_where_it_was() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&with_groups(vec![], vec![group("g", floor_tiles(2))]));
        teleport_tile(&mut w, 3.0);
        let Shape::Heightfield { geometry, .. } = flat_heightfield(4, 4, 0) else {
            unreachable!()
        };
        let pending = Body::fixed("s0000".to_string(), Shape::Heightfield { geometry, data: None });
        w.reconcile(&with_groups(vec![pending], vec![group("g", floor_tiles(2).split_off(1))]));
        assert_eq!(w.body_transform("s0000").unwrap().0, [0.0, 3.0, 0.0]);
        assert!(w.tags.contains_key("s0000"));
    }

    /// `n` frictionless 1 m tiles in a row along +x from x = 0, fused or
    /// separate bodies.
    fn row(n: i32, fused: bool) -> Vec<Body> {
        (0..n)
            .map(|i| {
                let tile = Body::fixed(format!("t{i:02}"), Shape::Cuboid { extents: [1.0; 3] })
                    .at([i as f32, 0.0, 0.0])
                    .with_friction(0.0);
                if fused { tile } else { separate(tile) }
            })
            .collect()
    }

    /// A body sliding at 5 m/s along +x from `x0` over `tiles`, a 1 m row
    /// along x (`wall`: beside it, pushed against it, without gravity). Once
    /// settled, the worst seam snag: offset along the contact normal from
    /// its settled distance, normal speed, and the speed lost along x by
    /// x = `end` (or after 400 frames).
    fn slide(
        tiles: &[Body],
        shape: &Shape,
        half: [f32; 3],
        wall: bool,
        x0: f32,
        end: f32,
    ) -> [f32; 3] {
        let normal = if wall { 2 } else { 1 };
        let mut start = [x0, 0.0, 0.0];
        start[normal] = 0.5 + half[normal] + 0.001;
        let mut velocity = [5.0, 0.0, 0.0];
        velocity[normal] = if wall { -0.5 } else { 0.0 };
        let slider = Body {
            velocity,
            rotation_locked: matches!(shape, Shape::Capsule { .. }),
            ..Body::dynamic("slider".to_string(), shape.clone()).at(start).with_friction(0.0)
        };
        let mut frame = scene(tiles.iter().cloned().chain([slider]).collect());
        frame.gravity = if wall { [0.0; 3] } else { [0.0, -9.81, 0.0] };
        let mut w = World::new(frame.gravity);
        let (mut settled, mut worst) = (None, [0.0f32; 3]);
        for _ in 0..400 {
            w.reconcile(&frame);
            w.step_frame(FIXED_DT);
            let (p, _) = w.body_transform("slider").unwrap();
            let v = w.body_velocity("slider").unwrap();
            worst[2] = 5.0 - v[0];
            if p[0] > end {
                break;
            }
            let Some(rest) = settled else {
                settled = (p[0] > x0 + 1.0).then_some(p[normal]);
                continue;
            };
            worst[0] = worst[0].max((p[normal] - rest).abs());
            worst[1] = worst[1].max(v[normal].abs());
        }
        worst
    }

    /// Box and capsule sliders, on (`wall` false) and beside the row: name,
    /// shape, half extents, wall.
    fn sliders() -> Vec<(&'static str, Shape, [f32; 3], bool)> {
        let cube = (Shape::Cuboid { extents: [0.8; 3] }, [0.4; 3]);
        let capsule = (Shape::Capsule { half_height: 0.4, radius: 0.3 }, [0.3, 0.7, 0.3]);
        [("box", cube), ("capsule", capsule)]
            .into_iter()
            .flat_map(|(name, (shape, half))| {
                [false, true].map(|wall| (name, shape.clone(), half, wall))
            })
            .collect()
    }

    /// Each snag metric over 8 start offsets: per start (`slide`).
    fn slides(
        tiles: &[Body],
        shape: &Shape,
        half: [f32; 3],
        wall: bool,
        end: f32,
    ) -> Vec<[f32; 3]> {
        (0..8).map(|k| slide(tiles, shape, half, wall, 0.2 + 0.07 * k as f32, end)).collect()
    }

    fn mean(runs: &[[f32; 3]]) -> [f32; 3] {
        [0, 1, 2].map(|i| runs.iter().map(|m| m[i]).sum::<f32>() / runs.len() as f32)
    }

    /// One cell's tiles merge into one box, so a slider crossing them meets
    /// no seam: no offset, no off-surface speed, no speed lost (the box
    /// beside the wall keeps the few mm it settles by, as on separate tiles).
    #[test]
    fn merged_tiles_slide_without_seam_kicks() {
        let tiles = row(16, true);
        for (name, shape, half, wall) in sliders() {
            let merged = slides(&tiles, &shape, half, wall, 11.0);
            eprintln!("seams {name} wall={wall}: merged {:?}", mean(&merged));
            for m in &merged {
                assert!(m.iter().all(|&x| x < 5e-3), "{name} wall={wall}: {m:?}");
            }
        }
    }

    /// Cells are compound boundaries, so a row crossing one (x = 15.5) keeps
    /// that seam, and it snags as the seam between two separate boxes does.
    #[test]
    fn a_cell_boundary_is_one_ordinary_seam() {
        let tiles = row(30, true);
        let halves = [(7.5, 16.0), (22.5, 14.0)].map(|(x, len)| {
            let half = Body::fixed(format!("h{x}"), Shape::Cuboid { extents: [len, 1.0, 1.0] });
            separate(half.at([x, 0.0, 0.0]).with_friction(0.0))
        });
        for (name, shape, half, wall) in sliders() {
            let merged = mean(&slides(&tiles, &shape, half, wall, 25.0));
            let two = mean(&slides(&halves, &shape, half, wall, 25.0));
            eprintln!("cell seam {name} wall={wall}: two boxes {two:?} merged {merged:?}");
            for i in 0..3 {
                assert!(merged[i] <= two[i] * 1.1 + 1e-3, "{name} wall={wall} metric {i}");
            }
        }
    }

    /// A crate resting across three merged tiles touches all three, middle
    /// included, as it does separate ones.
    #[test]
    fn a_body_on_merged_tiles_touches_each_tile_under_it() {
        let run = |fused: bool| {
            let plank = Shape::Cuboid { extents: [2.6, 0.4, 0.6] };
            let crate_ = Body::dynamic("crate".to_string(), plank).at([1.0, 1.0, 0.0]);
            let frame = scene(row(8, fused).into_iter().chain([crate_]).collect());
            let mut w = World::new([0.0, -9.81, 0.0]);
            let mut events = Vec::new();
            for _ in 0..60 {
                w.reconcile(&frame);
                w.step_frame(FIXED_DT);
                events.extend(sorted_events(w.take_events()));
            }
            if fused {
                assert_eq!(w.fused[0].parts.shapes().len(), 8);
                let rapier = w.colliders[w.fused[0].collider].shape().as_compound().unwrap();
                assert_eq!(rapier.shapes().len(), 1);
            }
            events
        };
        let merged = run(true);
        assert_eq!(merged, run(false));
        let touched: Vec<_> = merged.iter().map(|e| e.1.as_str()).collect();
        assert_eq!(touched, ["t00", "t01", "t02"]);
    }

    /// `frames` steps of `body` over 8 x 12 tiles `thick` high (top at
    /// y = 0.5), fused or separate: each step's events.
    fn on_tiles(
        body: &Body,
        thick: f32,
        fused: bool,
        frames: usize,
    ) -> Vec<Vec<(String, String, bool, bool)>> {
        let tiles = (0..8 * 12).map(|i| {
            let shape = Shape::Cuboid { extents: [1.0, thick, 1.0] };
            let tile = Body::fixed(format!("t{i:02}"), shape)
                .at([(i % 8) as f32, 0.5 - thick / 2.0, (i / 8) as f32]);
            if fused { tile } else { separate(tile) }
        });
        let frame = scene(tiles.chain([body.clone()]).collect());
        let mut w = World::new([0.0, -9.81, 0.0]);
        (0..frames)
            .map(|_| {
                w.reconcile(&frame);
                w.step_frame(FIXED_DT);
                sorted_events(w.take_events())
            })
            .collect()
    }

    /// Per-tile contact events on merged tiles match separate tiles: a plank
    /// lying diagonally (its contact patch's bounds cover tiles it misses)
    /// and a crate sunk deep into thin tiles (contact points below them; it
    /// then sinks through, differently, so only its first step compares).
    #[test]
    fn merged_tiles_report_the_tiles_a_contact_patch_meets() {
        let (sin, cos) = std::f32::consts::FRAC_PI_8.sin_cos();
        let plank = Body::dynamic("plank".into(), Shape::Cuboid { extents: [5.0, 0.2, 0.3] })
            .at([3.5, 0.62, 3.5])
            .facing([0.0, sin, 0.0, cos]);
        let sunk = Body::dynamic("sunk".into(), Shape::Cuboid { extents: [1.6, 1.0, 0.6] })
            .at([1.0, 0.35, 10.0]);
        for (body, thick, frames) in [(plank, 1.0, 30), (sunk, 0.04, 1)] {
            let merged = on_tiles(&body, thick, true, frames);
            assert!(merged.iter().flatten().count() >= 3, "{}: {merged:?}", body.tag);
            assert_eq!(merged, on_tiles(&body, thick, false, frames), "{}", body.tag);
        }
    }

    /// A ball landing fast beside a seam touches only the tile it lands on.
    /// (Separate tiles also report a brief contact with the neighbour from
    /// rapier's speculative contact, cm short of it; a merged box doesn't.)
    #[test]
    fn a_fast_landing_names_only_the_tile_under_it() {
        for x in [0.3, 0.35, 0.4, 0.45] {
            let ball = Body::dynamic("ball".into(), Shape::Sphere { radius: 0.25 })
                .at([x, 40.0, 10.0]);
            let starts: Vec<_> = on_tiles(&ball, 1.0, true, 200)
                .into_iter()
                .flatten()
                .filter(|e| e.2)
                .map(|e| e.1)
                .collect();
            assert_eq!(starts, ["t80"], "x = {x}");
        }
    }

    #[test]
    fn boxes_sharing_faces_merge_and_the_rest_pass_through() {
        let at = |x: f32, y: f32, z: f32| Pose::from_translation(Vector::new(x, y, z));
        let cube = SharedShape::cuboid(0.5, 0.5, 0.5);
        // A 4 x 4 floor whose tile (1, 1) is raised, a sphere, a turned box.
        let mut parts: Vec<_> = (0..16)
            .map(|i| (at((i % 4) as f32 - 2.0, 0.0, (i / 4) as f32 - 2.0), cube.clone()))
            .collect();
        parts[5].0.translation.y = 0.5;
        parts.push((at(9.0, 0.0, 0.0), SharedShape::ball(0.5)));
        let turned = Pose::from_parts(Vector::new(9.0, 0.0, 3.0), Rotation::from_rotation_y(0.3));
        parts.push((turned, cube.clone()));
        // Two boxes with a 0.5 mm crack between them stay two.
        parts.push((at(9.0, 0.0, 6.0), cube.clone()));
        parts.push((at(10.0005, 0.0, 6.0), cube.clone()));
        let (shape, merged) = merge_boxes(&parts);
        let shapes = shape.as_compound().unwrap().shapes();
        // Row z = 0; (0, 1); (2..3, 1); the raised tile; rows z = 2..3; ball;
        // turned; the cracked pair.
        assert_eq!(shapes.len(), 9);
        let rows_2_3: Vec<usize> = (8..16).collect();
        let groups =
            [&[0, 1, 2, 3][..], &[4], &[6, 7], &[5], &rows_2_3, &[16], &[17], &[18], &[19]];
        for group in groups {
            let union = group
                .iter()
                .map(|&i| parts[i].1.compute_aabb(&parts[i].0))
                .reduce(|a, b| a.merged(&b))
                .unwrap();
            let (pose, box_) = &shapes[merged[group[0]] as usize];
            assert!(group.iter().all(|&i| merged[i] == merged[group[0]]), "{group:?}");
            assert_eq!(box_.compute_aabb(pose), union, "{group:?}");
        }
        assert_eq!(shapes[merged[17] as usize].0, turned);
    }

    #[test]
    fn a_teleported_tile_leaves_its_merged_box() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&scene(row(8, true)));
        w.queue_command(PhysicsCommand::Teleport {
            tag: "t03".to_string(),
            position: [3.0, 2.0, 0.0],
        });
        w.step_frame(FIXED_DT);
        let rapier = w.colliders[w.fused[0].collider].shape().as_compound().unwrap();
        assert_eq!(rapier.shapes().len(), 3);
        assert_eq!(w.body_transform("t03").unwrap().0, [3.0, 2.0, 0.0]);
        let down = |x: f32| w.raycast([x, 5.0, 0.0], [0.0, -1.0, 0.0], 10.0).map(|h| h.tag);
        assert_eq!([down(2.9), down(4.0)], [Some("t03".into()), Some("t04".into())]);
    }

    /// The equivalence golden: the same bodies declared flat or as a group
    /// give the same events and poses every frame and byte-identical worlds,
    /// through impulses, a teleport of a fused body and a mid-run group edit
    /// — and a recorded memo run seeks and replays bit-exact across that edit.
    #[test]
    fn a_memoized_group_simulates_exactly_like_flat_bodies() {
        use super::super::{Command, Simulatable, Timeline, TimelineLog};

        let before = group("statics", floor_tiles(1500));
        let after = {
            let mut tiles = floor_tiles(1500);
            tiles.remove(7);
            tiles[99].position[1] += 0.5;
            tiles.push(
                Body::fixed("s9999".to_string(), Shape::Sphere { radius: 1.0 }).at([4.0, 1.0, 4.0]),
            );
            group("statics", tiles)
        };
        let dynamics = vec![
            crate_at("a", [0.0, 3.0, 0.0]),
            crate_at("b", [-7.5, 2.0, -18.0]),
            crate_at("c", [4.0, 5.0, 4.0]),
        ];
        // Frames 400..500: a flat fixed box in the group's cells, so a key
        // goes dirty while the group's `Arc` holds still.
        let ledge = Body::fixed("ledge".to_string(), Shape::Cuboid { extents: [2.0, 0.2, 2.0] })
            .at([0.5, 0.6, -18.0]);
        let cmds_at = |frame: u64, memo: bool| {
            let statics = if frame < 300 { &before } else { &after };
            let mut dynamics = dynamics.clone();
            if (400..500).contains(&frame) {
                dynamics.push(ledge.clone());
            }
            let scene = if memo {
                with_groups(dynamics.clone(), vec![statics.clone()])
            } else {
                scene(
                    dynamics
                        .iter()
                        .chain(statics.bodies.iter())
                        .cloned()
                        .collect(),
                )
            };
            let mut cmds = vec![Command::DeclareScene(scene)];
            if frame == 100 {
                cmds.push(Command::Apply(PhysicsCommand::ApplyImpulse {
                    tag: "a".to_string(),
                    impulse: [2.0, 4.0, 0.0],
                }));
            }
            if frame == 200 {
                cmds.push(Command::Apply(PhysicsCommand::Teleport {
                    tag: "s0042".to_string(),
                    position: [0.0, 0.5, 0.0],
                }));
            }
            cmds
        };

        let mut flat = World::new([0.0, -9.81, 0.0]);
        let mut memo = World::new([0.0, -9.81, 0.0]);
        let mut log = TimelineLog::keyframes(30);
        let mut at_290 = Vec::new();
        let mut events = 0;
        for frame in 0..600 {
            if frame == 290 {
                at_290 = memo.snapshot();
            }
            let cmds = cmds_at(frame, true);
            log.record(frame, &memo, &cmds);
            let flat_events = flat.step(&cmds_at(frame, false));
            assert_eq!(flat_events, memo.step(&cmds), "events at {frame}");
            for tag in ["a", "b", "c", "s0042"] {
                assert_eq!(flat.body_transform(tag), memo.body_transform(tag), "{tag} at {frame}");
            }
            if frame % 50 == 0 || frame == 599 {
                assert!(sim_state(&flat) == sim_state(&memo), "diverged at {frame}");
            }
            events += flat_events.len();
            if frame == 450 {
                assert_eq!(fused_bodies(&memo), 1501);
            }
            if frame > 0 && ![300, 400].contains(&frame) {
                let flat_bodies = if (400..500).contains(&frame) { 4 } else { 3 };
                assert_eq!(memo.reconcile_stats().compared, flat_bodies, "frame {frame}");
                assert_eq!(memo.reconcile_stats().written, 0, "frame {frame}");
            }
        }
        // b's landing on a seam: six tiles at once.
        assert!(events >= 6, "{events} events");
        assert_eq!(memo.body_transform("s0042").unwrap().0, [0.0, 0.5, 0.0]);
        assert!(memo.body_transform("s9999").is_some() && memo.body_transform("s0007").is_none());
        assert_eq!((memo.tags.len(), fused_bodies(&memo)), (3, 1500));

        // Seek to before the edit: the restored world holds the old group,
        // so replaying frame 300 must not take the fast path.
        let end = memo.snapshot();
        log.seek(290, &mut memo);
        assert!(memo.snapshot() == at_290, "seek(290) diverged");
        for cmds in log.commands_since(290).to_vec() {
            memo.step(&cmds);
        }
        assert!(
            memo.snapshot() == end,
            "replay across the group edit diverged"
        );
    }

    // ── checkpoint sharing ──────────────────────────────────────────────

    fn shares(a: &World, b: &World) -> bool {
        Arc::ptr_eq(&a.declared, &b.declared)
            && Arc::ptr_eq(&a.tags, &b.tags)
            && Arc::ptr_eq(&a.flat, &b.flat)
            && Arc::ptr_eq(&a.rooms, &b.rooms)
    }

    #[test]
    fn keyframes_of_a_still_scene_share_the_per_body_maps() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        let frame = with_groups(
            vec![crate_at("a", [0.0, 3.0, 0.0])],
            vec![group("statics", floor_tiles(300))],
        );
        w.reconcile(&frame);
        let first = w.checkpoint();
        for _ in 0..30 {
            w.step_frame(FIXED_DT);
            w.reconcile(&frame);
        }
        let second = w.checkpoint();
        assert!(shares(&first.world, &second.world));
        assert!(shares(&first.world, &w));
    }

    #[test]
    fn a_write_after_a_checkpoint_does_not_reach_it() {
        let mut w = World::new([0.0, -9.81, 0.0]);
        let tiles = floor_tiles(20);
        w.reconcile(&scene(tiles.clone()));
        w.step_frame(FIXED_DT);
        let checkpoint = w.checkpoint();
        let before = w.snapshot();

        // Move one tile, drop another, add a crate: every map is written.
        let mut edited = tiles;
        edited[3] = edited[3].clone().at([40.0, 0.0, 0.0]);
        edited.remove(5);
        edited.push(crate_at("crate", [0.0, 3.0, 0.0]));
        w.reconcile(&scene(edited));
        w.step_frame(FIXED_DT);
        assert!(!Arc::ptr_eq(&checkpoint.world.declared, &w.declared));
        assert!(!Arc::ptr_eq(&checkpoint.world.tags, &w.tags));
        assert!(!Arc::ptr_eq(&checkpoint.world.flat, &w.flat));

        assert!(checkpoint.world.snapshot() == before, "the checkpoint changed");
        w.restore_checkpoint(&checkpoint);
        assert!(w.snapshot() == before, "restore diverged");
    }
}

/// The fused room as colliders, on the desktop stand-in room (the
/// `examples/scene-grid` floor, desk and wall) fused on the CPU from the
/// desktop camera's viewpoint.
#[cfg(test)]
mod fused_room_tests {
    use std::sync::Arc;

    use super::*;
    use crate::fused_mesh::tests::{depth_frame_sized, view_projection};
    use crate::fused_mesh::Volume;
    use crate::physics::DEFAULT_GRAVITY;

    const FLOOR: ([f32; 3], [f32; 3]) = ([-3.0, -0.02, -1.5], [3.0, 0.0, 4.5]);
    const DESK: ([f32; 3], [f32; 3]) = ([-0.3, 0.0, 0.7], [0.9, 0.75, 1.3]);
    const WALL: ([f32; 3], [f32; 3]) = ([-3.0, 0.0, 2.375], [3.0, 2.6, 2.425]);
    const BALL: f32 = 0.05;

    /// The stand-in room fused from the desktop camera, looking around it.
    fn stand_in_room() -> RoomMeshes {
        let eye = [0.0, 1.6, -0.6];
        let mut volume = Volume::default();
        let mut chunks = BTreeMap::new();
        for target in [
            [0.3, 0.6, 1.0],
            [-1.2, 0.6, 2.0],
            [1.4, 0.6, 2.0],
            [0.0, 1.2, 2.4],
            [0.0, 0.0, 0.6],
        ] {
            let vp = view_projection(eye, target);
            volume.integrate(&depth_frame_sized(vp, &[FLOOR, DESK, WALL], 160));
            for (key, mesh) in volume.extract_dirty() {
                match mesh {
                    Some(mesh) => chunks.insert(key, Arc::new(mesh)),
                    None => chunks.remove(&key),
                };
            }
        }
        RoomMeshes {
            chunks,
            top_eye_y: volume.top_eye_y,
        }
    }

    fn room_body(room: &Arc<RoomMeshes>) -> Body {
        Body::fixed("room".to_string(), Shape::FusedRoom { room: room.clone() })
            .with_restitution(0.8)
    }

    fn ball(position: [f32; 3], velocity: [f32; 3]) -> Body {
        Body::dynamic("ball".to_string(), Shape::Sphere { radius: BALL })
            .at(position)
            .with_velocity(velocity)
            .with_restitution(0.8)
    }

    fn world_with(bodies: Vec<Body>) -> World {
        let mut w = World::new(DEFAULT_GRAVITY);
        w.reconcile(&PhysicsScene::create(DEFAULT_GRAVITY, bodies));
        w
    }

    fn run(w: &mut World, steps: usize, mut each: impl FnMut(&World)) {
        for _ in 0..steps {
            w.step_fixed();
            each(w);
        }
    }

    #[test]
    fn the_stand_in_room_becomes_chunk_colliders_with_a_floor() {
        let room = Arc::new(stand_in_room());
        let mut w = world_with(vec![room_body(&room)]);
        let (colliders, floor) = w.fused_room_colliders().unwrap();
        assert!(colliders > 10, "{colliders}");
        assert!(colliders <= room.chunks.len() + 1);
        let floor = floor.expect("the floor was seen");
        assert!(floor.abs() < 0.01, "floor at {floor}");
        // A ray at the wall reports the room's tag.
        run(&mut w, 1, |_| {});
        let hit = w.raycast([-1.5, 1.0, 0.0], [0.0, 0.0, 1.0], 5.0).unwrap();
        assert_eq!(hit.tag, "room");
        assert!((hit.position[2] - 2.375).abs() < 0.02, "{:?}", hit.position);
    }

    /// Placement's query: a ray onto the room hits the chunk or floor plane
    /// under it with the room's tag, the point, and the normal facing the
    /// ray; nothing there is a miss.
    #[test]
    fn rays_land_on_the_room_with_a_normal_facing_the_ray() {
        let room = stand_in_room();
        let mut w = world_with(vec![room_body(&Arc::new(room.clone()))]);
        run(&mut w, 1, |_| {});
        let near = |a: [f32; 3], b: [f32; 3], tol: f32| (0..3).all(|i| (a[i] - b[i]).abs() < tol);
        let land = |w: &World, from: [f32; 3], dir: [f32; 3], at: [f32; 3], normal: [f32; 3]| {
            let hit = w.raycast(from, dir, 10.0).expect("a hit");
            assert_eq!(hit.tag, "room");
            assert!(near(hit.position, at, 0.03), "{from:?} hit {:?}, not {at:?}", hit.position);
            assert!(near(hit.normal, normal, 0.1), "{from:?} normal {:?}", hit.normal);
            hit.position
        };
        let (down, ahead) = ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
        // The wall, the desk's top and front, and the floor plane.
        land(&w, [-1.5, 1.0, 0.0], ahead, [-1.5, 1.0, 2.375], [0.0, 0.0, -1.0]);
        let desk = land(&w, [0.3, 1.6, 1.0], down, [0.3, 0.75, 1.0], [0.0, 1.0, 0.0]);
        land(&w, [0.3, 0.4, 0.0], ahead, [0.3, 0.4, 0.7], [0.0, 0.0, -1.0]);
        land(&w, [-1.0, 1.0, 0.5], down, [-1.0, 0.0, 0.5], [0.0, 1.0, 0.0]);
        // Up from the eye there is no ceiling.
        assert!(w.raycast([0.0, 1.6, 0.0], [0.0, 1.0, 0.0], 10.0).is_none());
        // The desk top's chunk re-fused 2 cm higher: the ray finds the new top.
        let top = |p: &[f32; 3]| (p[1] - 0.75).abs() < 0.01 && (p[0] - 0.3).abs() < 0.1 && (p[2] - 1.0).abs() < 0.1;
        let (&key, mesh) = room.chunks.iter().find(|(_, m)| m.positions.iter().any(top)).unwrap();
        let mut raised = (**mesh).clone();
        raised.positions.iter_mut().for_each(|p| p[1] += 0.02);
        let mut next = room.clone();
        next.chunks.insert(key, Arc::new(raised));
        w.reconcile(&PhysicsScene::create(DEFAULT_GRAVITY, vec![room_body(&Arc::new(next))]));
        run(&mut w, 1, |_| {});
        let raised = land(&w, [0.3, 1.6, 1.0], down, [0.3, 0.77, 1.0], [0.0, 1.0, 0.0]);
        assert!((raised[1] - desk[1] - 0.02).abs() < 1e-3, "{desk:?} -> {raised:?}");
    }

    /// Chunks build through `Physics.trimesh`'s shape builder: one build per
    /// chunk collider, each with no density.
    #[test]
    fn room_chunks_use_the_trimesh_builder() {
        let room = Arc::new(stand_in_room());
        let before = crate::physics::trimesh_builds();
        let w = world_with(vec![room_body(&room)]);
        let live = w.rooms.values().next().unwrap();
        assert_eq!(crate::physics::trimesh_builds() - before, live.chunks.len());
        for &handle in live.chunks.values() {
            assert_eq!(w.colliders[handle].density(), 0.0);
        }
    }

    #[test]
    fn a_ball_dropped_over_the_desk_rests_on_it() {
        let room = Arc::new(stand_in_room());
        let mut w = world_with(vec![room_body(&room), ball([0.3, 1.4, 1.0], [0.0; 3])]);
        let mut lowest = f32::INFINITY;
        run(&mut w, 360, |w| {
            lowest = lowest.min(w.body_transform("ball").unwrap().0[1]);
        });
        let ([x, y, z], _) = w.body_transform("ball").unwrap();
        assert!((y - (0.75 + BALL)).abs() < 0.02, "resting at y = {y}");
        assert!((-0.3..0.9).contains(&x) && (0.7..1.3).contains(&z));
        assert!(lowest > 0.7, "fell into the desk: {lowest}");
        let v = w.body_velocity("ball").unwrap();
        assert!(v.iter().all(|c| c.abs() < 0.01), "still moving: {v:?}");
    }

    #[test]
    fn a_ball_rolled_at_the_wall_bounces_off_it() {
        let room = Arc::new(stand_in_room());
        let rolled = ball([-1.5, BALL, 1.2], [0.0, 0.0, 3.0]);
        let thrown = |speed| ball([-1.5, 1.2, 1.2], [0.0, 0.0, speed]).as_continuous();
        // The wall is a surface with no thickness: a thrown ball is swept
        // along each step, at any speed.
        for body in [rolled, thrown(5.0), thrown(6.0), thrown(9.0), thrown(15.0)] {
            let speed = body.velocity[2];
            let mut w = world_with(vec![room_body(&room), body]);
            let (mut nearest, mut back) = (f32::NEG_INFINITY, 0.0f32);
            run(&mut w, 120, |w| {
                nearest = nearest.max(w.body_transform("ball").unwrap().0[2]);
                back = back.min(w.body_velocity("ball").unwrap()[2]);
            });
            assert!(nearest < 2.375, "{speed} m/s went through the wall: z = {nearest}");
            assert!(nearest > 2.375 - BALL - 0.03, "{speed} m/s never reached it: {nearest}");
            assert!(back < -0.4 * speed, "{speed} m/s did not bounce back: vz = {back}");
        }
    }

    #[test]
    fn continuous_reconciles_onto_the_live_body() {
        let swept = ball([0.0, 1.0, 0.0], [0.0; 3]).as_continuous();
        let mut w = world_with(vec![swept]);
        assert!(w.bodies[w.tags["ball"]].is_ccd_enabled());
        w.reconcile(&PhysicsScene::create(DEFAULT_GRAVITY, vec![ball([0.0, 1.0, 0.0], [0.0; 3])]));
        assert!(!w.bodies[w.tags["ball"]].is_ccd_enabled());
    }

    /// Contacts are per body pair: a ball landing on the room starts one, and
    /// a chunk re-meshed under it (old collider out, new one in) says
    /// nothing.
    #[test]
    fn contacts_with_the_room_are_per_body_not_per_chunk() {
        let room = stand_in_room();
        let resting = || ball([0.3, 1.0, 1.0], [0.0; 3]).with_restitution(0.0);
        let still = |room: RoomMeshes| room_body(&Arc::new(room)).with_restitution(0.0);
        let mut w = world_with(vec![still(room.clone()), resting()]);
        let names = |e: &PhysicsEvent| {
            let mut pair = [e.a.clone(), e.b.clone()];
            pair.sort();
            pair == ["ball", "room"]
        };
        run(&mut w, 240, |_| {});
        let landed = w.take_events();
        assert!(landed.iter().all(names), "{landed:?}");
        assert_eq!(
            landed.iter().map(|e| e.started).collect::<Vec<_>>(),
            vec![true],
            "one contact, however many colliders it touched"
        );
        // The desk top's chunk under the ball, moved a millimetre.
        let under_ball = |p: &[f32; 3]| {
            (p[1] - 0.75).abs() < 0.01 && (p[0] - 0.3).abs() < 0.1 && (p[2] - 1.0).abs() < 0.1
        };
        let (&key, mesh) = room
            .chunks
            .iter()
            .find(|(_, m)| m.positions.iter().any(under_ball))
            .unwrap();
        let mut moved = (**mesh).clone();
        moved.positions.iter_mut().for_each(|p| p[0] += 0.001);
        let mut next = room.clone();
        next.chunks.insert(key, Arc::new(moved));
        w.reconcile(&PhysicsScene::create(
            DEFAULT_GRAVITY,
            vec![still(next), resting()],
        ));
        run(&mut w, 2, |_| {});
        assert_eq!(w.take_events(), vec![], "a re-mesh is not a contact");
        // Nor across a snapshot taken between the re-mesh and its step.
        let mut next = room.clone();
        let (&key, mesh) = next.chunks.iter().find(|(_, m)| m.positions.iter().any(under_ball)).unwrap();
        let mut moved = (**mesh).clone();
        moved.positions.iter_mut().for_each(|p| p[0] -= 0.001);
        next.chunks.insert(key, Arc::new(moved));
        w.reconcile(&PhysicsScene::create(DEFAULT_GRAVITY, vec![still(next), resting()]));
        let mut restored = World::new(DEFAULT_GRAVITY);
        restored.restore(&w.snapshot()).unwrap();
        run(&mut restored, 2, |_| {});
        assert_eq!(restored.take_events(), vec![], "a restored re-mesh is not a contact");
        // Lifting the ball off ends the contact, naming the room.
        w.queue_command(PhysicsCommand::Teleport {
            tag: "ball".to_string(),
            position: [0.3, 1.5, 1.0],
        });
        w.step_frame(FIXED_DT);
        let lifted = w.take_events();
        assert!(lifted.len() == 1 && !lifted[0].started && names(&lifted[0]), "{lifted:?}");
    }

    /// A world with a room snapshots to JSON and resumes bit-exact.
    #[test]
    fn a_world_with_a_room_restores_bit_exact() {
        let room = Arc::new(stand_in_room());
        let scene = PhysicsScene::create(
            DEFAULT_GRAVITY,
            vec![room_body(&room), ball([0.3, 1.4, 1.0], [0.5, 0.0, 0.0])],
        );
        let mut live = world_with(scene.bodies.clone());
        run(&mut live, 30, |_| {});
        let mut restored = World::new(DEFAULT_GRAVITY);
        restored.restore(&live.snapshot()).unwrap();
        for w in [&mut live, &mut restored] {
            w.reconcile(&scene);
            run(w, 60, |_| {});
        }
        assert_eq!(live.snapshot(), restored.snapshot());
    }

    /// Despawning a room right after a re-mesh reports nothing about it.
    #[test]
    fn a_despawned_room_reports_no_contacts() {
        let room = stand_in_room();
        let resting = || ball([0.3, 1.0, 1.0], [0.0; 3]);
        let mut w = world_with(vec![room_body(&Arc::new(room.clone())), resting()]);
        run(&mut w, 240, |_| {});
        let mut next = room.clone();
        // The desk top's chunk under the ball.
        let under_ball = |p: &[f32; 3]| {
            (p[1] - 0.75).abs() < 0.01 && (p[0] - 0.3).abs() < 0.1 && (p[2] - 1.0).abs() < 0.1
        };
        let (&key, mesh) = next.chunks.iter().find(|(_, m)| m.positions.iter().any(under_ball)).unwrap();
        let mut moved = (**mesh).clone();
        moved.positions.iter_mut().for_each(|p| p[0] += 0.001);
        next.chunks.insert(key, Arc::new(moved));
        w.reconcile(&PhysicsScene::create(
            DEFAULT_GRAVITY,
            vec![room_body(&Arc::new(next)), resting()],
        ));
        w.reconcile(&PhysicsScene::create(DEFAULT_GRAVITY, vec![resting()]));
        let _ = w.take_events();
        run(&mut w, 2, |_| {});
        let events = w.take_events();
        assert!(events.iter().all(|e| e.a != "room" && e.b != "room"), "{events:?}");
    }

    #[test]
    fn teleporting_the_room_is_refused() {
        let mut w = world_with(vec![room_body(&Arc::new(stand_in_room()))]);
        w.queue_command(PhysicsCommand::Teleport {
            tag: "room".to_string(),
            position: [0.0, 1.0, 0.0],
        });
        w.step_frame(FIXED_DT);
        assert_eq!(w.body_transform("room").unwrap().0, [0.0; 3]);
        assert_eq!(w.take_command_warnings().len(), 1);
    }

    /// Nobody saw the floor behind the desk; the floor plane is there anyway.
    #[test]
    fn a_ball_dropped_on_unseen_floor_lands_on_the_floor_plane() {
        let room = Arc::new(stand_in_room());
        let behind_desk = [0.3, 0.3, 1.8];
        let mut w = world_with(vec![room_body(&room), ball(behind_desk, [0.0; 3])]);
        run(&mut w, 1, |_| {});
        let probe = w.raycast_excluding([0.3, 0.2, 1.8], [0.0, -1.0, 0.0], 1.0, Some("ball"));
        assert!(probe.is_some_and(|hit| hit.position[1].abs() < 0.01));
        run(&mut w, 240, |_| {});
        let y = w.body_transform("ball").unwrap().0[1];
        assert!((y - BALL).abs() < 0.01, "y = {y}");
    }

    /// A head that rises later admits the floor it saw while low: the same
    /// chunks with a higher head re-run floor detection, rebuilding nothing.
    #[test]
    fn a_higher_head_admits_the_floor() {
        let room = stand_in_room();
        let low = RoomMeshes {
            top_eye_y: Some(0.5),
            ..room.clone()
        };
        let mut w = world_with(vec![room_body(&Arc::new(low))]);
        assert_eq!(w.fused_room_colliders().unwrap().1, None);
        let before = w.rooms["room"].chunks.clone();
        w.reconcile(&PhysicsScene::create(
            DEFAULT_GRAVITY,
            vec![room_body(&Arc::new(room))],
        ));
        let floor = w.fused_room_colliders().unwrap().1.expect("a floor");
        assert!(floor.abs() < 0.01, "{floor}");
        // Only the floor's chunks changed (their level triangles went to the
        // plane); the rest kept their colliders.
        let after = &w.rooms["room"].chunks;
        assert!(before.iter().any(|(k, h)| after.get(k) == Some(h)));
    }

    /// A re-fused room after a checkpoint copies `rooms`; the checkpoint keeps
    /// the old colliders and restores byte-exact.
    #[test]
    fn a_re_fuse_after_a_checkpoint_does_not_reach_it() {
        let room = stand_in_room();
        let mut w = world_with(vec![room_body(&Arc::new(room.clone()))]);
        let checkpoint = w.checkpoint();
        let before = w.snapshot();
        let mut next = room;
        let key = *next.chunks.keys().find(|k| k[1] > 0).unwrap();
        let mut moved = (*next.chunks[&key]).clone();
        moved.positions.iter_mut().for_each(|p| p[0] += 0.01);
        next.chunks.insert(key, Arc::new(moved));
        w.reconcile(&PhysicsScene::create(
            DEFAULT_GRAVITY,
            vec![room_body(&Arc::new(next))],
        ));
        assert!(!Arc::ptr_eq(&checkpoint.world.rooms, &w.rooms));
        assert_ne!(checkpoint.world.rooms["room"].chunks, w.rooms["room"].chunks);
        assert!(checkpoint.world.snapshot() == before, "the checkpoint changed");
        w.restore_checkpoint(&checkpoint);
        assert!(w.snapshot() == before, "restore diverged");
    }

    /// Re-declaring the same room is free; a re-fused chunk rebuilds only
    /// that chunk's collider.
    #[test]
    fn only_re_meshed_chunks_rebuild() {
        let room = stand_in_room();
        let mut w = world_with(vec![room_body(&Arc::new(room.clone()))]);
        let before = w.rooms["room"].chunks.clone();
        let (&key, mesh) = room.chunks.iter().find(|(k, _)| k[1] > 0).unwrap();
        let mut next = room.clone();
        let mut moved = (**mesh).clone();
        moved.positions.iter_mut().for_each(|p| p[0] += 0.01);
        next.chunks.insert(key, Arc::new(moved));
        w.reconcile(&PhysicsScene::create(
            DEFAULT_GRAVITY,
            vec![room_body(&Arc::new(next))],
        ));
        let after = &w.rooms["room"].chunks;
        assert_eq!(before.len(), after.len());
        for (k, handle) in &before {
            assert_eq!(*k == key, after[k] != *handle, "chunk {k:?}");
        }
    }

    /// Max |vertical velocity| of a ball resting on the floor over 2 s, after
    /// 2 s to settle.
    fn resting_jitter(w: &mut World) -> f32 {
        let resting = Body::dynamic("ball".to_string(), Shape::Sphere { radius: BALL })
            .at([-1.13, 0.3, 0.37]);
        let mut scene = PhysicsScene::create(DEFAULT_GRAVITY, vec![resting]);
        scene.bodies.extend(w.declared.values().cloned());
        w.reconcile(&scene);
        run(w, 120, |_| {});
        let mut jitter = 0.0f32;
        run(w, 120, |w| {
            jitter = jitter.max(w.body_velocity("ball").unwrap()[1].abs())
        });
        jitter
    }

    /// The floor plane's point: on the fused floor's ripple a resting ball
    /// keeps rolling.
    #[test]
    fn the_floor_plane_stills_a_resting_ball() {
        // The CPU fixture fuses a floor flat to a millimetre; give it the few
        // millimetres of ripple a depth sensor's floor has.
        let mut room = stand_in_room();
        for mesh in room.chunks.values_mut() {
            let mesh = Arc::make_mut(mesh);
            for p in mesh.positions.iter_mut().filter(|p| p[1].abs() < 0.02) {
                p[1] += 0.004 * (p[0] * 37.0).sin() * (p[2] * 23.0).cos();
            }
        }
        // Planed: the room as `Physics.fusedRoom` builds it.
        let mut planed = world_with(vec![room_body(&Arc::new(room.clone()))]);
        // Unsnapped: the same chunks as raw meshes, on an untagged body.
        let mut raw = World::new(DEFAULT_GRAVITY);
        let body = raw.bodies.insert(RigidBodyBuilder::fixed().build());
        for mesh in room.chunks.values() {
            let Some(collider) = room::chunk_collider(mesh, None).and_then(chunk_trimesh) else {
                continue;
            };
            raw.colliders
                .insert_with_parent(collider.restitution(0.8), body, &mut raw.bodies);
        }
        let (with, without) = (resting_jitter(&mut planed), resting_jitter(&mut raw));
        eprintln!("resting ball max |vy|: floor plane {with:.6} m/s, raw {without:.6} m/s");
        assert!(with <= without, "floor plane {with} > raw {without}");
        assert!(with < 1e-3, "floor plane jitter {with}");
    }
}
