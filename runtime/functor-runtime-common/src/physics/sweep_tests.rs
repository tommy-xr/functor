//! `World::sweep`: a shape swept between two poses, rotating as it moves.

use super::{Body, PhysicsScene, RigidPose, Shape, SweepFilter, SweepHit, World, FIXED_DT};
use crate::MappedTrackingPose;

const X: [f32; 3] = [1.0, 0.0, 0.0];
const Y: [f32; 3] = [0.0, 1.0, 0.0];
const Z: [f32; 3] = [0.0, 0.0, 1.0];

/// A pose as `Scene.atPose` takes it: local +Z along `forward`, +Y along `up`.
fn pose(at: [f32; 3], forward: [f32; 3], up: [f32; 3]) -> RigidPose {
    MappedTrackingPose {
        position: at,
        forward,
        up,
    }
    .to_position_rotation()
    .unwrap()
}

fn upright(at: [f32; 3]) -> RigidPose {
    pose(at, Z, Y)
}

fn cuboid(x: f32, y: f32, z: f32) -> Shape {
    Shape::Cuboid { extents: [x, y, z] }
}

/// A fixed slab whose -x face is at `face`, `thick` deep in x.
fn wall(tag: &str, face: f32, thick: f32) -> Body {
    Body::fixed(tag.to_string(), cuboid(thick, 4.0, 4.0)).at([face + thick / 2.0, 0.0, 0.0])
}

fn stepped(bodies: Vec<Body>) -> World {
    let mut w = World::new([0.0; 3]);
    w.reconcile(&PhysicsScene::create([0.0; 3], bodies));
    w.step_frame(FIXED_DT);
    w
}

fn near(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

fn fraction(hit: Option<SweepHit>) -> f32 {
    hit.expect("a hit").fraction
}

#[test]
fn a_rotated_box_stops_where_its_turned_extent_meets_the_wall() {
    let w = stepped(vec![wall("wall", 1.0, 0.2)]);
    let stick = cuboid(0.2, 0.4, 1.0);
    let diag = [
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    // (forward, up) and the box's half-extent along world x when posed so.
    let cases = [
        (Z, Y, 0.1),                      // as authored
        (X, Y, 0.5),                      // yawed: its length across
        (Z, X, 0.2),                      // rolled: its height across
        (Y, [0.0, 0.0, -1.0], 0.1),       // pitched straight up
        (diag, Y, (0.1 + 0.5) * diag[0]), // yawed 45°
    ];
    for (forward, up, half) in cases {
        let (from, to) = (
            pose([0.0; 3], forward, up),
            pose([2.0, 0.0, 0.0], forward, up),
        );
        let hit = w
            .sweep(&stick, from, to, SweepFilter::Fixed)
            .expect("hits the wall");
        let want = (1.0 - half) / 2.0;
        assert!(
            near(hit.fraction, want, 2.0e-3),
            "{forward:?}/{up:?}: {} vs {want}",
            hit.fraction
        );
        assert_eq!(hit.tag, "wall");
        assert!(near(hit.position[0], 1.0, 1.0e-2), "{hit:?}");
        assert!(hit.normal[0] < -0.99, "{hit:?}");
    }
}

#[test]
fn an_unrotated_sweep_matches_shape_cast() {
    let w = stepped(vec![
        wall("wall", 1.0, 0.2),
        Body::fixed("floor".into(), cuboid(8.0, 0.2, 8.0)).at([0.0, -1.1, 0.0]),
    ]);
    let shapes = [
        Shape::Sphere { radius: 0.1 },
        cuboid(0.2, 0.3, 0.4),
        Shape::Capsule {
            half_height: 0.2,
            radius: 0.05,
        },
    ];
    for shape in shapes {
        for d in [[2.0f32, 0.0, 0.0], [1.5, -1.0, 0.3], [0.0, -2.0, 0.0]] {
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let linear = w
                .shape_cast(&shape, [0.0; 3], d, len, None)
                .expect("linear hit");
            let swept = w
                .sweep(&shape, upright([0.0; 3]), upright(d), SweepFilter::All)
                .expect("swept hit");
            assert_eq!(swept.tag, linear.tag);
            assert!(
                near(swept.fraction * len, linear.distance, 2.0e-3),
                "{shape:?} {d:?}: {swept:?} vs {linear:?}"
            );
        }
    }
}

#[test]
fn rotating_in_place_into_a_wall_hits_where_a_linear_cast_misses() {
    let w = stepped(vec![wall("wall", 1.0, 0.2)]);
    // 1.6 m long along local +Z, its centre 0.5 m from the wall.
    let stick = cuboid(0.05, 0.05, 1.6);
    let centre = [0.5, 0.0, 0.0];
    let nudge = [0.5, 0.01, 0.0];
    // The centre barely moves: a linear cast along it sees nothing.
    assert!(w
        .shape_cast(&stick, centre, [0.0, 1.0, 0.0], 0.01, None)
        .is_none());
    let hit = w
        .sweep(
            &stick,
            pose(centre, Z, Y),
            pose(nudge, X, Y),
            SweepFilter::Fixed,
        )
        .expect("the far end swings into the wall");
    // At the hit, the stick's far corner is at the face: 0.5 + 0.8 sin θ + 0.025 cos θ.
    let theta = hit.fraction * std::f32::consts::FRAC_PI_2;
    let reach = 0.5 + 0.8 * theta.sin() + 0.025 * theta.cos();
    assert!(
        near(reach, 1.0, 5.0e-3),
        "fraction {} reaches x {reach}",
        hit.fraction
    );
    assert!(hit.normal[0] < -0.99, "{hit:?}");
}

#[test]
fn a_thin_wall_is_not_tunnelled() {
    let w = stepped(vec![wall("thin", 1.0, 0.01)]);
    // A small ball crossing it in one long sweep.
    let ball = Shape::Sphere { radius: 0.05 };
    let f = fraction(w.sweep(
        &ball,
        upright([0.0; 3]),
        upright([5.0, 0.0, 0.0]),
        SweepFilter::Fixed,
    ));
    assert!(near(f * 5.0, 0.95, 2.0e-3), "{f}");
    // A fast half turn swinging a stick's end through it.
    let stick = cuboid(0.02, 0.02, 1.6);
    let hit = w.sweep(
        &stick,
        pose([0.6, 0.0, 0.0], Z, Y),
        pose([0.6, 0.0, 0.0], [0.0, 0.0, -1.0], Y),
        SweepFilter::Fixed,
    );
    assert!(
        hit.as_ref()
            .is_some_and(|h| h.fraction > 0.0 && h.fraction < 0.5),
        "{hit:?}"
    );
}

#[test]
fn a_start_overlap_holds_against_pressing_and_lets_go_moving_out() {
    let w = stepped(vec![wall("wall", 1.0, 0.2)]);
    let block = cuboid(0.2, 0.2, 0.2);
    // 2 cm into the wall.
    let inside = [0.92, 0.0, 0.0];
    let out = w.sweep(
        &block,
        upright(inside),
        upright([0.5, 0.0, 0.0]),
        SweepFilter::Fixed,
    );
    assert_eq!(out, None);
    let along = w.sweep(
        &block,
        upright(inside),
        upright([0.92, 0.3, 0.0]),
        SweepFilter::Fixed,
    );
    assert_eq!(along, None, "sliding along the face doesn't press into it");
    let hit = w
        .sweep(
            &block,
            upright(inside),
            upright([1.0, 0.0, 0.0]),
            SweepFilter::Fixed,
        )
        .expect("pressing in");
    assert_eq!((hit.fraction, hit.tag.as_str()), (0.0, "wall"));
    assert!(hit.normal[0] < -0.99, "{hit:?}");
    // Flush on the face and turning either way: one edge presses in.
    for turn in [0.01f32, -0.01] {
        let (s, c) = (turn / 2.0).sin_cos();
        let to = ([0.91, 0.0, 0.0], [0.0, s, 0.0, c]);
        let hit = w.sweep(&block, upright([0.91, 0.0, 0.0]), to, SweepFilter::Fixed);
        assert_eq!(hit.map(|h| h.fraction), Some(0.0), "turning {turn}");
    }
    // A stick turned into the wall: turning further presses, turning back frees it.
    let stick = cuboid(0.05, 0.05, 1.6);
    let into = pose([0.5, 0.0, 0.0], [0.8, 0.0, 0.6], Y);
    let deeper = pose([0.5, 0.0, 0.0], X, Y);
    let back = pose([0.5, 0.0, 0.0], Z, Y);
    assert_eq!(
        fraction(w.sweep(&stick, into, deeper, SweepFilter::Fixed)),
        0.0
    );
    assert_eq!(w.sweep(&stick, into, back, SweepFilter::Fixed), None);
}

#[test]
fn no_motion_asks_whether_the_pose_overlaps() {
    let w = stepped(vec![wall("wall", 1.0, 0.2)]);
    let block = cuboid(0.2, 0.2, 0.2);
    let hit = w.sweep(
        &block,
        upright([0.95, 0.0, 0.0]),
        upright([0.95, 0.0, 0.0]),
        SweepFilter::Fixed,
    );
    let hit = hit.expect("overlapping");
    assert_eq!((hit.fraction, hit.tag.as_str()), (0.0, "wall"));
    assert!(hit.normal[0] < -0.99, "{hit:?}");
    let clear = upright([0.5, 0.0, 0.0]);
    assert_eq!(w.sweep(&block, clear, clear, SweepFilter::Fixed), None);
    // Turned so it reaches the wall, in place.
    let turned = pose([0.5, 0.0, 0.0], X, Y);
    let stick = cuboid(0.05, 0.05, 1.6);
    assert_eq!(
        fraction(w.sweep(&stick, turned, turned, SweepFilter::Fixed)),
        0.0
    );
}

#[test]
fn the_filter_picks_which_bodies_count() {
    let mut sensor = Body::fixed("sensor".into(), cuboid(0.1, 1.0, 1.0)).at([0.3, 0.0, 0.0]);
    sensor.sensor = true;
    let w = stepped(vec![
        sensor,
        Body::kinematic("kinematic".into(), cuboid(0.1, 1.0, 1.0)).at([0.5, 0.0, 0.0]),
        Body::dynamic("dynamic".into(), cuboid(0.1, 1.0, 1.0)).at([0.7, 0.0, 0.0]),
        wall("wall", 1.0, 0.2),
    ]);
    let ball = Shape::Sphere { radius: 0.05 };
    let (from, to) = (upright([0.0; 3]), upright([2.0, 0.0, 0.0]));
    let tag = |filter| w.sweep(&ball, from, to, filter).map(|h| h.tag);
    assert_eq!(tag(SweepFilter::Fixed).as_deref(), Some("wall"));
    assert_eq!(tag(SweepFilter::NotKinematic).as_deref(), Some("dynamic"));
    assert_eq!(tag(SweepFilter::All).as_deref(), Some("kinematic"));
    // Still, too: an overlap test sees only what the filter lets through.
    let at = upright([0.5, 0.0, 0.0]);
    assert_eq!(w.sweep(&ball, at, at, SweepFilter::Fixed), None);
    assert_eq!(
        w.sweep(&ball, at, at, SweepFilter::All)
            .map(|h| h.tag)
            .as_deref(),
        Some("kinematic")
    );
}

#[test]
fn a_fused_wall_reports_its_own_tag() {
    // Two fixed boxes in one cell fuse into one compound.
    let w = stepped(vec![wall("near", 1.0, 0.2), wall("far", 3.0, 0.2)]);
    let ball = Shape::Sphere { radius: 0.1 };
    let hit = w
        .sweep(
            &ball,
            upright([2.0, 0.0, 0.0]),
            upright([4.0, 0.0, 0.0]),
            SweepFilter::Fixed,
        )
        .unwrap();
    assert_eq!(hit.tag, "far");
    assert!(near(hit.fraction, 0.45, 2.0e-3), "{hit:?}");
    let still = upright([1.05, 0.0, 0.0]);
    assert_eq!(
        w.sweep(&ball, still, still, SweepFilter::Fixed)
            .unwrap()
            .tag,
        "near"
    );
}
