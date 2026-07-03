//! Determinism goldens (docs/physics.md, "Test harness").
//!
//! These are the executable form of the determinism contract: same build, same
//! declared-scene history, same fixed steps → **byte-identical snapshots**.
//! They run under plain `cargo test -p functor_runtime_common` (no GL, no
//! dylibs), so CI exercises them on every platform.
//!
//! The scripted scenario deliberately includes a despawn *and* a respawn:
//! Rapier arena handles depend on the full insert/remove history, and this is
//! the fine-print requirement most likely to regress silently.

use super::*;

/// The declared scene as a pure function of the frame number: a ground slab and
/// three stacked crates; crate "c" despawns at frame 30 and respawns offset at
/// frame 60 (exercising arena-slot reuse); crate "b" gets teleported at frame
/// 90 (exercising the divergence rule mid-replay).
fn scene_at(frame: u64) -> PhysicsScene {
    let ground = Body::fixed(
        "ground".to_string(),
        Shape::Cuboid {
            extents: [20.0, 0.2, 20.0],
        },
    );
    let a = Body::dynamic(
        "a".to_string(),
        Shape::Cuboid {
            extents: [1.0, 1.0, 1.0],
        },
    )
    .at([0.0, 2.0, 0.0]);
    let b_pos = if frame < 90 {
        [0.2, 3.5, 0.1]
    } else {
        [3.0, 4.0, 0.0]
    };
    let b = Body::dynamic(
        "b".to_string(),
        Shape::Cuboid {
            extents: [1.0, 1.0, 1.0],
        },
    )
    .at(b_pos);
    let c = Body::dynamic("c".to_string(), Shape::Sphere { radius: 0.5 }).at(if frame < 30 {
        [-0.1, 5.0, 0.2]
    } else {
        [1.5, 6.0, -0.5]
    });

    let mut bodies = vec![ground, a, b];
    if !(30..60).contains(&frame) {
        bodies.push(c);
    }
    PhysicsScene::create([0.0, -9.81, 0.0], bodies)
}

const FRAMES: u64 = 120;

/// Drive a world through the scripted scenario up to (excluding) `to`.
fn run(world: &mut World, from: u64, to: u64) {
    for f in from..to {
        world.reconcile(&scene_at(f));
        world.step_fixed();
    }
}

#[test]
fn determinism_golden_two_worlds_stay_byte_identical() {
    let mut a = World::new([0.0, -9.81, 0.0]);
    let mut b = World::new([0.0, -9.81, 0.0]);
    for f in 0..FRAMES {
        let scene = scene_at(f);
        a.reconcile(&scene);
        b.reconcile(&scene);
        a.step_fixed();
        b.step_fixed();
        assert!(a.snapshot() == b.snapshot(), "worlds diverged at frame {f}");
    }
    // Sanity: the scenario actually simulated something (crates fell and hit
    // the ground), so byte-equality above wasn't comparing static worlds.
    let (pos, _) = a.body_transform("a").unwrap();
    assert!(pos[1] < 2.0 && pos[1] > 0.0, "unexpected rest pose {pos:?}");
}

#[test]
fn restore_golden_snapshot_plus_replay_matches_live_run() {
    // Snapshot at frame 20 — *before* the despawn (30), respawn (60), and
    // teleport (90) — so the replayed window exercises removal, arena-slot
    // reuse, and the divergence rule against serde-restored arenas.
    let mut live = World::new([0.0, -9.81, 0.0]);
    run(&mut live, 0, 20);
    let checkpoint = live.snapshot();
    run(&mut live, 20, FRAMES);

    // Restore the checkpoint and replay the same declared scenes forward: this
    // is exactly what the Timeline's `seek` does (Phase 1 of the rewind seam),
    // and it must land byte-identical to the live run.
    let mut resumed = World::new([0.0, 0.0, 0.0]);
    resumed.restore(&checkpoint).unwrap();
    run(&mut resumed, 20, FRAMES);

    assert!(
        resumed.snapshot() == live.snapshot(),
        "restored+replayed world diverged from the live run"
    );
}
