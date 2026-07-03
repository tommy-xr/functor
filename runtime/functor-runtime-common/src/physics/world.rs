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
    pub fn step_frame(&mut self, real_dt: f32) -> u32 {
        self.accumulator += real_dt.max(0.0);
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
        steps
    }

    /// Advance exactly one fixed step.
    pub fn step_fixed(&mut self) {
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
            &(),
        );
        self.frame += 1;
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
            .sensor(body.sensor);
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
