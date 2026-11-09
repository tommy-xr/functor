//! Bodies placed at full poses (`Physics.atPose`) and given spin
//! (`PhysicsCommand::SetAngularVelocity`): a held prop's lifecycle.

use super::{
    Body, Command, PhysicsCommand, PhysicsScene, Shape, Simulatable, Timeline, TimelineLog, World, FIXED_DT,
};
use crate::MappedTrackingPose;

type Pose = ([f32; 3], [f32; 4]);

/// A pose as `Scene.atPose` takes it, placed as `Physics.atPose` places it.
fn pose(at: [f32; 3], forward: [f32; 3], up: [f32; 3]) -> Pose {
    MappedTrackingPose { position: at, forward, up }
        .to_position_rotation()
        .unwrap()
}

/// Yawed `deg` about +Y at `at`: forward swings from +Z toward +X.
fn yawed(at: [f32; 3], deg: f32) -> Pose {
    let (s, c) = deg.to_radians().sin_cos();
    pose(at, [s, 0.0, c], [0.0, 1.0, 0.0])
}

const STICK: Shape = Shape::Cuboid { extents: [0.04, 0.04, 0.7] };

fn stick(kinematic: bool, (p, q): Pose) -> Body {
    let body = if kinematic {
        Body::kinematic("stick".to_string(), STICK)
    } else {
        Body::dynamic("stick".to_string(), STICK)
    };
    body.at(p).facing(q)
}

fn floor() -> Body {
    Body::fixed("floor".to_string(), Shape::Cuboid { extents: [10.0, 0.2, 10.0] }).at([0.0, -0.1, 0.0])
}

fn scene(gravity: [f32; 3], bodies: Vec<Body>) -> PhysicsScene {
    PhysicsScene::create(gravity, bodies)
}

/// The angle between two rotations, in degrees.
fn turn_deg(a: [f32; 4], b: [f32; 4]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    2.0 * dot.abs().min(1.0).acos().to_degrees()
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt()
}

// A stick swung like a bat, its end in the hand: redeclared kinematic at
// each step's pose, it moves there over the step with that velocity (turning
// included), so it shoves the box it sweeps through.
#[test]
fn a_kinematic_body_redeclared_at_each_pose_shoves_what_it_sweeps() {
    let pivot = [0.0, 0.06, 0.0];
    let swing = |deg: f32| {
        let (s, c) = deg.to_radians().sin_cos();
        let (_, q) = yawed(pivot, deg);
        ([pivot[0] + 0.35 * s, pivot[1], pivot[2] + 0.35 * c], q)
    };
    let crate_ = || {
        Body::dynamic("box".to_string(), Shape::Cuboid { extents: [0.1, 0.1, 0.1] })
            .at([0.5, 0.05, 0.0])
            .with_mass(0.3)
    };
    let g = [0.0, -9.81, 0.0];
    let mut w = World::new(g);
    w.reconcile(&scene(g, vec![floor(), crate_(), stick(true, swing(0.0))]));
    for _ in 0..30 {
        w.step_frame(FIXED_DT);
    }
    let rest = w.body_transform("box").unwrap().0;
    // 6° a step at 60 Hz: 360°/s, the tip at ~4.4 m/s.
    let rate = 6.0f32;
    let mut spun = 0.0f32;
    for i in 1..=25 {
        w.reconcile(&scene(g, vec![floor(), crate_(), stick(true, swing(rate * i as f32))]));
        w.step_frame(FIXED_DT);
        if i == 5 {
            spun = w.body_angular_velocity("stick").unwrap()[1];
        }
    }
    let expected = (rate * 60.0).to_radians();
    assert!((spun - expected).abs() < 0.01 * expected, "stick spins at {spun} rad/s, not {expected}");
    // At 90° the tip moves along -Z: the box is knocked that way.
    let (moved, _) = w.body_transform("box").unwrap();
    assert!(rest[2] - moved[2] > 0.1, "box not shoved: {rest:?} -> {moved:?}");
}

// Held (kinematic at a moving pose), then let go (dynamic at the last held
// pose): no jump at the swap, and it carries on from there at the hand's
// speed, still turned as held.
#[test]
fn dynamic_to_kinematic_to_dynamic_keeps_the_pose() {
    let g = [0.0, -9.81, 0.0];
    let mut w = World::new(g);
    let spawn = yawed([0.0, 2.0, 0.0], 0.0);
    w.reconcile(&scene(g, vec![stick(false, spawn)]));
    for _ in 0..10 {
        w.step_frame(FIXED_DT);
    }
    let mut held = spawn;
    for i in 1..=5 {
        held = pose([0.1 * i as f32, 1.5, 0.0], [0.6, 0.0, 0.8], [-0.2, 0.9, 0.15]);
        w.reconcile(&scene(g, vec![stick(true, held)]));
        w.step_frame(FIXED_DT);
    }
    let (p, q) = w.body_transform("stick").unwrap();
    assert!(dist(p, held.0) < 1e-5 && turn_deg(q, held.1) < 0.01, "held at {p:?} {q:?}");

    w.reconcile(&scene(g, vec![stick(false, held)]));
    let (p, q) = w.body_transform("stick").unwrap();
    assert!(dist(p, held.0) < 1e-5 && turn_deg(q, held.1) < 0.01, "jumped at the swap: {p:?}");
    for _ in 0..3 {
        w.reconcile(&scene(g, vec![stick(false, held)]));
        w.step_frame(FIXED_DT);
    }
    // The hand moved 10 cm a step: 6 m/s along x, for three steps.
    let (fell, q) = w.body_transform("stick").unwrap();
    assert!(fell[1] < held.0[1] - 1e-3 && (fell[0] - held.0[0] - 0.3).abs() < 1e-3, "{fell:?}");
    assert!(turn_deg(q, held.1) < 0.01, "turned while falling freely");
}

fn spin(angular_velocity: [f32; 3]) -> PhysicsCommand {
    PhysicsCommand::SetAngularVelocity { tag: "stick".to_string(), angular_velocity }
}

fn near3(a: [f32; 3], b: [f32; 3]) -> bool {
    dist(a, b) < 1e-4
}

// A release: declared dynamic at the held pose and given both velocities in
// one frame. Neither is a declaration, so redeclaring the same body, or a
// changed or rebuilt one, keeps them; a later command replaces the spin.
#[test]
fn a_released_spin_lasts_until_replaced() {
    let zero = [0.0; 3];
    let held = yawed([0.0, 1.0, 0.0], 30.0);
    let mut w = World::new(zero);
    w.reconcile(&scene(zero, vec![stick(true, held)]));
    w.step_frame(FIXED_DT);

    let (v, omega) = ([1.0, 2.0, -3.0], [0.0, 9.0, 0.0]);
    w.reconcile(&scene(zero, vec![stick(false, held)]));
    w.queue_command(PhysicsCommand::SetVelocity { tag: "stick".to_string(), velocity: v });
    w.queue_command(spin(omega));
    w.step_frame(FIXED_DT);
    for _ in 0..29 {
        w.reconcile(&scene(zero, vec![stick(false, held)]));
        w.step_frame(FIXED_DT);
    }
    assert!(w.take_command_warnings().is_empty());
    assert!(near3(w.body_angular_velocity("stick").unwrap(), omega));
    assert!(near3(w.body_velocity("stick").unwrap(), v));
    // 30 steps at 9 rad/s about +Y, from 30° of yaw.
    let (_, q) = w.body_transform("stick").unwrap();
    let turned = 30.0 + (9.0f32 * 30.0 / 60.0).to_degrees();
    assert!(turn_deg(q, yawed(zero, turned).1) < 0.5, "turned {}°", turn_deg(q, held.1));

    // A changed material, then a rebuild (a new mass), keep the spin.
    w.reconcile(&scene(zero, vec![stick(false, held).with_friction(0.9)]));
    w.reconcile(&scene(zero, vec![stick(false, held).with_friction(0.9).with_mass(2.0)]));
    w.step_frame(FIXED_DT);
    assert!(near3(w.body_angular_velocity("stick").unwrap(), omega));

    for replacement in [[3.0, 0.0, -1.0], zero] {
        w.queue_command(spin(replacement));
        for _ in 0..5 {
            w.step_frame(FIXED_DT);
        }
        assert!(near3(w.body_angular_velocity("stick").unwrap(), replacement));
    }
}

#[test]
fn spinning_a_kinematic_or_upright_body_warns() {
    let mut w = World::new([0.0; 3]);
    let held = yawed([0.0, 1.0, 0.0], 0.0);
    w.reconcile(&scene([0.0; 3], vec![stick(true, held)]));
    w.queue_command(spin([0.0, 1.0, 0.0]));
    w.step_frame(FIXED_DT);
    w.reconcile(&scene([0.0; 3], vec![stick(false, held).as_upright()]));
    w.queue_command(spin([0.0, 1.0, 0.0]));
    w.step_frame(FIXED_DT);
    let warnings = w.take_command_warnings();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("non-dynamic"), "{}", warnings[0]);
    assert!(warnings[1].contains("upright"), "{}", warnings[1]);
    assert_eq!(w.body_angular_velocity("stick").unwrap(), [0.0; 3]);
}

/// A held stick swung toward the floor, released spinning at frame 20, then
/// tumbling on the floor: each frame's recorded input.
fn release_at(frame: u64) -> Vec<Command> {
    let g = [0.0, -9.81, 0.0];
    let held = |f: u64| yawed([0.0, 1.0 - 0.02 * f as f32, 0.0], 7.0 * f as f32);
    let body = if frame < 20 { stick(true, held(frame)) } else { stick(false, held(20)) };
    let mut cmds = vec![Command::DeclareScene(scene(g, vec![floor(), body]))];
    if frame == 20 {
        cmds.push(Command::Apply(PhysicsCommand::SetVelocity {
            tag: "stick".to_string(),
            velocity: [0.5, 1.0, 0.0],
        }));
        cmds.push(Command::Apply(spin([2.0, 7.0, 0.0])));
    }
    cmds
}

// Rewound to mid-flight and replayed, the release reproduces the same pose
// and spin bit for bit.
#[test]
fn a_rewound_release_replays_its_pose_and_spin() {
    const FRAMES: u64 = 60;
    let mut live = World::new([0.0; 3]);
    let mut tl = TimelineLog::keyframes(8);
    for f in 0..FRAMES {
        let cmds = release_at(f);
        tl.record(f, &live, &cmds);
        live.step(&cmds);
        if f == 22 {
            let flying = live.body_angular_velocity("stick").unwrap();
            assert!(near3(flying, [2.0, 7.0, 0.0]), "in flight it spins as released: {flying:?}");
        }
    }
    let end = (live.body_transform("stick"), live.body_angular_velocity("stick"));

    for from in [18, 21, 37] {
        let mut replayed = World::new([0.0; 3]);
        tl.seek(from, &mut replayed);
        for cmds in tl.commands_since(from).to_vec() {
            replayed.step(&cmds);
        }
        assert!(Simulatable::snapshot(&replayed) == Simulatable::snapshot(&live), "from {from}");
        assert_eq!((replayed.body_transform("stick"), replayed.body_angular_velocity("stick")), end);
    }
}
