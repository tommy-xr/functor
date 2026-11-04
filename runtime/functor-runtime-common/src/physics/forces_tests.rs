//! `Physics.explode` (a radial impulse command) and `Physics.forceVolume` (a
//! declared per-step field).

use super::{
    remove_world, with_world, Blast, Body, Command, Falloff, ForceVolume, PhysicsCommand,
    PhysicsScene, Shape, Simulatable, SteppedPhysics, World, DEFAULT_WORLD, FIXED_DT,
};

fn ball(tag: &str, at: [f32; 3]) -> Body {
    Body::dynamic(tag.to_string(), Shape::Sphere { radius: 0.5 })
        .at(at)
        .with_mass(1.0)
}

fn weightless(bodies: Vec<Body>) -> PhysicsScene {
    PhysicsScene::create([0.0; 3], bodies)
}

fn blast(radius: f32, strength: f32, falloff: Falloff, lift: f32) -> PhysicsCommand {
    PhysicsCommand::Explode(Blast {
        center: [0.0; 3],
        radius,
        strength,
        falloff,
        lift,
    })
}

fn velocity(w: &World, tag: &str) -> [f32; 3] {
    w.body_velocity(tag).unwrap()
}

fn speed(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn sleeping(w: &World, tag: &str) -> bool {
    let dump: serde_json::Value = serde_json::from_str(&w.dump()).unwrap();
    dump["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["tag"] == tag)
        .unwrap()["sleeping"]
        .as_bool()
        .unwrap()
}

/// Balls whose nearest points are 1.5, 2.5 and 4.5 from the blast, radius 4.
fn exploded(falloff: Falloff) -> World {
    let mut w = World::new([0.0; 3]);
    let scene = weightless(vec![
        ball("near", [2.0, 0.0, 0.0]),
        ball("far", [-3.0, 0.0, 0.0]),
        ball("out", [0.0, 0.0, 5.0]),
    ]);
    w.step(&[
        Command::DeclareScene(scene),
        Command::Apply(blast(4.0, 10.0, falloff, 0.0)),
    ]);
    assert!(w.take_command_warnings().is_empty());
    w
}

#[test]
fn explode_pushes_outward_with_linear_falloff() {
    let w = exploded(Falloff::Linear);
    let (near, far) = (velocity(&w, "near"), velocity(&w, "far"));
    // impulse = strength * (1 - d / r) on a 1 kg ball, straight out.
    assert!((near[0] - 10.0 * (1.0 - 1.5 / 4.0)).abs() < 1e-3, "{near:?}");
    assert!((far[0] + 10.0 * (1.0 - 2.5 / 4.0)).abs() < 1e-3, "{far:?}");
    assert!(near[1].abs() < 1e-4 && near[2].abs() < 1e-4, "{near:?}");
    assert_eq!(velocity(&w, "out"), [0.0; 3], "outside the radius");
}

#[test]
fn quadratic_falloff_concentrates_the_push() {
    let w = exploded(Falloff::Quadratic);
    let near = velocity(&w, "near")[0];
    let far = velocity(&w, "far")[0];
    assert!((near - 10.0 * 0.625f32.powi(2)).abs() < 1e-3, "{near}");
    assert!((far + 10.0 * 0.375f32.powi(2)).abs() < 1e-3, "{far}");
}

#[test]
fn lift_tilts_a_level_push_upward() {
    let mut w = World::new([0.0; 3]);
    w.step(&[
        Command::DeclareScene(weightless(vec![ball("a", [2.0, 0.0, 0.0])])),
        Command::Apply(blast(4.0, 10.0, Falloff::Linear, 1.0)),
    ]);
    let v = velocity(&w, "a");
    assert!(v[1] > 0.0 && (v[0] - v[1]).abs() < 1e-3, "45 degrees up: {v:?}");
    assert!((speed(v) - 6.25).abs() < 1e-3, "lift redirects, not adds: {v:?}");
}

#[test]
fn explode_wakes_sleepers_and_ignores_fixed_and_kinematic_bodies() {
    let ground = Body::fixed(
        "ground".to_string(),
        Shape::Cuboid {
            extents: [20.0, 0.2, 20.0],
        },
    );
    let crate_ = Body::dynamic(
        "crate".to_string(),
        Shape::Cuboid {
            extents: [1.0, 1.0, 1.0],
        },
    )
    .at([1.0, 0.6, 0.0]);
    let post = Body::kinematic("post".to_string(), Shape::Sphere { radius: 0.3 }).at([-1.0, 1.0, 0.0]);
    let scene = PhysicsScene::create([0.0, -9.81, 0.0], vec![ground, crate_, post]);
    let mut w = World::new([0.0, -9.81, 0.0]);
    w.step(&[Command::DeclareScene(scene.clone())]);
    for _ in 0..300 {
        w.step(&[]);
    }
    assert!(sleeping(&w, "crate"), "the crate settles to sleep");
    let still = |w: &World| (w.body_transform("ground"), w.body_transform("post"));
    let before = still(&w);

    w.step(&[
        Command::DeclareScene(scene),
        Command::Apply(blast(3.0, 8.0, Falloff::Linear, 0.5)),
    ]);
    assert!(w.take_command_warnings().is_empty());
    assert!(!sleeping(&w, "crate"), "the blast wakes it");
    let v = velocity(&w, "crate");
    assert!(v[0] > 1.0 && v[1] > 0.0, "pushed out and up: {v:?}");
    assert_eq!(still(&w), before, "fixed and kinematic bodies don't move");
}

fn fan(force: [f32; 3], turbulence: f32, drag: f32) -> ForceVolume {
    ForceVolume {
        tag: "fan".to_string(),
        center: [0.0; 3],
        size: [4.0, 4.0, 4.0],
        force,
        turbulence,
        drag,
    }
}

fn with_volume(bodies: Vec<Body>, volume: ForceVolume) -> PhysicsScene {
    PhysicsScene {
        volumes: vec![volume],
        ..weightless(bodies)
    }
}

#[test]
fn a_force_volume_accelerates_only_bodies_inside_it() {
    let mut w = World::new([0.0; 3]);
    let scene = with_volume(
        vec![ball("in", [0.0; 3]), ball("out", [5.0, 0.0, 0.0])],
        fan([0.0, 0.0, 2.0], 0.0, 0.0),
    );
    w.step(&[Command::DeclareScene(scene)]);
    for _ in 1..30 {
        w.step(&[]);
    }
    // 2 N on 1 kg for 30 steps, still inside (it moves 0.25 m).
    let v = velocity(&w, "in");
    assert!((v[2] - 2.0 * 30.0 * FIXED_DT).abs() < 1e-3, "{v:?}");
    assert_eq!(velocity(&w, "out"), [0.0; 3]);

    // Undeclared, it stops pushing.
    w.step(&[Command::DeclareScene(weightless(vec![
        ball("in", [0.0; 3]),
        ball("out", [5.0, 0.0, 0.0]),
    ]))]);
    let coasting = velocity(&w, "in");
    w.step(&[]);
    assert_eq!(velocity(&w, "in"), coasting);
}

#[test]
fn turbulence_is_seeded_per_body_and_drag_slows_without_reversing() {
    let run = || {
        let mut w = World::new([0.0; 3]);
        let scene = with_volume(
            vec![ball("a", [-0.5, 0.0, 0.0]), ball("b", [0.5, 0.0, 0.0])],
            fan([0.0; 3], 3.0, 0.0),
        );
        w.step(&[Command::DeclareScene(scene)]);
        for _ in 0..20 {
            w.step(&[]);
        }
        w
    };
    let (w, again) = (run(), run());
    assert!(w.snapshot() == again.snapshot(), "gusts are deterministic");
    let (a, b) = (velocity(&w, "a"), velocity(&w, "b"));
    assert!(speed(a) > 0.01 && speed(b) > 0.01, "{a:?} {b:?}");
    assert_ne!(a, b, "each body gusts on its own seed");

    let mut w = World::new([0.0; 3]);
    let moving = ball("m", [-1.0, 0.0, 0.0]).with_velocity([4.0, 0.0, 0.0]);
    w.step(&[Command::DeclareScene(with_volume(vec![moving], fan([0.0; 3], 0.0, 2.0)))]);
    for _ in 0..10 {
        w.step(&[]);
    }
    let v = velocity(&w, "m")[0];
    assert!(v > 0.0 && v < 4.0 * 0.8, "drag 2/s over 1/6 s: {v}");
}

/// A blast and a gusty volume, recorded live, rewind and replay byte-exact.
#[test]
fn explosions_and_volumes_replay_byte_identically() {
    remove_world(DEFAULT_WORLD);
    let mut sp = SteppedPhysics::new();
    let scene = PhysicsScene {
        gravity: [0.0, -9.81, 0.0],
        ..with_volume(
            vec![
                Body::fixed(
                    "ground".to_string(),
                    Shape::Cuboid {
                        extents: [20.0, 0.2, 20.0],
                    },
                ),
                ball("a", [0.5, 0.6, 0.0]),
                ball("b", [-0.8, 0.6, 0.3]),
            ],
            fan([1.0, 0.0, 0.0], 2.0, 0.5),
        )
    };
    let snapshot = || with_world(DEFAULT_WORLD, |w| w.snapshot()).unwrap();
    let (mut snap_5, mut snap_20) = (Vec::new(), Vec::new());
    for t in 0..40 {
        if t == 5 {
            snap_5 = snapshot();
        }
        if t == 20 {
            snap_20 = snapshot();
        }
        if t == 10 {
            with_world(DEFAULT_WORLD, |w| {
                w.queue_command(blast(3.0, 4.0, Falloff::Quadratic, 0.3))
            });
        }
        sp.advance(&scene, FIXED_DT);
    }
    let end = snapshot();
    // A seek replays the recorded blast from the log.
    assert!(sp.seek_to_frame(20).is_empty());
    assert!(snapshot() == snap_20, "seek replays the recorded blast");
    assert!(sp.rewind_to_frame(5).is_empty());
    assert!(snapshot() == snap_5, "rewind restores the volume and bodies");
    for _ in 5..10 {
        sp.advance(&scene, FIXED_DT);
    }
    // The blast is in the truncated future: re-issue it, as the game would.
    with_world(DEFAULT_WORLD, |w| {
        w.queue_command(blast(3.0, 4.0, Falloff::Quadratic, 0.3))
    });
    for _ in 10..40 {
        sp.advance(&scene, FIXED_DT);
    }
    assert!(snapshot() == end, "replaying the same inputs reproduces the run");
    remove_world(DEFAULT_WORLD);
}
