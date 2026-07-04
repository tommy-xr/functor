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

/// The frame's full command list — the declared scene, plus (at frame 45) an
/// impulse kicking crate "a", exercising the Phase 3 command path: queued,
/// applied after reconcile, replayed like any other input.
fn commands_at(frame: u64) -> Vec<Command> {
    let mut cmds = vec![Command::DeclareScene(scene_at(frame))];
    if frame == 45 {
        cmds.push(Command::Apply(PhysicsCommand::ApplyImpulse {
            tag: "a".to_string(),
            impulse: [0.0, 4.0, 1.5],
        }));
    }
    cmds
}

/// Drive a world through the scripted scenario up to (excluding) `to`, via
/// the `Simulatable` seam (the same path the Timeline replays through).
fn run(world: &mut World, from: u64, to: u64) {
    for f in from..to {
        world.step(&commands_at(f));
    }
}

#[test]
fn determinism_golden_two_worlds_stay_byte_identical() {
    let mut a = World::new([0.0, -9.81, 0.0]);
    let mut b = World::new([0.0, -9.81, 0.0]);
    for f in 0..FRAMES {
        let cmds = commands_at(f);
        a.step(&cmds);
        b.step(&cmds);
        assert!(
            World::snapshot(&a) == World::snapshot(&b),
            "worlds diverged at frame {f}"
        );
    }
    // Sanity: the scenario actually simulated something (crates fell and hit
    // the ground), so byte-equality above wasn't comparing static worlds.
    let (pos, _) = a.body_transform("a").unwrap();
    assert!(pos[1] < 2.0 && pos[1] > 0.0, "unexpected rest pose {pos:?}");
}

/// Drive a `Timeline` + sim through the scripted scenario — including the
/// frame-45 impulse, so every `seek` in the goldens below also proves that
/// recorded commands re-apply during replay.
fn drive<T: Timeline<World>>(tl: &mut T, sim: &mut World, frames: u64) {
    for f in 0..frames {
        let cmds = commands_at(f);
        tl.record(f, sim, &cmds);
        sim.step(&cmds);
    }
}

#[test]
fn strategy_equivalence_golden_keyframes_match_snapshot_ring_for_every_seek() {
    // Record the same run through both cadences (keyframes-every-16 leaves
    // most frames snapshot-less; the every-frame ring is the O(1) oracle).
    let mut kf = TimelineLog::keyframes(16);
    let mut kf_sim = World::new([0.0, -9.81, 0.0]);
    drive(&mut kf, &mut kf_sim, FRAMES);

    let mut ring = TimelineLog::snapshot_ring();
    let mut ring_sim = World::new([0.0, -9.81, 0.0]);
    drive(&mut ring, &mut ring_sim, FRAMES);

    // Every recorded frame must seek byte-identical under both strategies —
    // this is both rewind-correctness and a determinism check (the doc's
    // trait contract: seek(K) == restore-earlier + re-step).
    for k in 0..FRAMES {
        kf.seek(k, &mut kf_sim);
        ring.seek(k, &mut ring_sim);
        assert!(
            Simulatable::snapshot(&kf_sim) == Simulatable::snapshot(&ring_sim),
            "strategies disagree at seek({k})"
        );
    }
}

#[test]
fn replay_golden_replayonly_seek_to_end_matches_live_run() {
    let mut live = World::new([0.0, -9.81, 0.0]);
    let mut tl = TimelineLog::replay_only();
    drive(&mut tl, &mut live, FRAMES);
    let live_end = Simulatable::snapshot(&live);

    // The replay-only cadence holds a single frame-0 snapshot; seeking the
    // last frame replays the entire command log through fresh arenas.
    let mut replayed = World::new([0.0, 0.0, 0.0]);
    tl.seek(FRAMES - 1, &mut replayed);
    // seek lands on the *pre-step* state of the last frame; finish with the
    // log's own final entry to compare against the live end state.
    for cmds in tl.commands_since(FRAMES - 1).to_vec() {
        replayed.step(&cmds);
    }

    assert!(
        Simulatable::snapshot(&replayed) == live_end,
        "ReplayOnly seek-to-end diverged from the live run"
    );
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
