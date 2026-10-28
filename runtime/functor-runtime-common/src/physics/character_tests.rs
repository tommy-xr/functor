//! `Physics.character` + `PhysicsCommand::Move`: a kinematic capsule walking
//! trimesh, box and heightfield worlds.

use std::sync::Arc;

use super::{
    remove_world, with_world, Body, CharacterSettings, PhysicsCommand, PhysicsScene, Shape,
    SteppedPhysics, TriMeshData, World, DEFAULT_WORLD, FIXED_DT,
};

const GRAVITY: f32 = -9.81;
/// Capsule: half-height 0.6 + radius 0.3, so the centre stands 0.9 above the
/// feet (plus the offset).
const STAND: f32 = 0.9 + SETTINGS.offset;
const SETTINGS: CharacterSettings = CharacterSettings {
    step_height: 0.35,
    max_slope: std::f32::consts::FRAC_PI_4,
    snap: 0.3,
    offset: 0.02,
};

fn player(at: [f32; 3]) -> Body {
    let capsule = Shape::Capsule {
        half_height: 0.6,
        radius: 0.3,
    };
    Body::character("player".to_string(), capsule, SETTINGS).at(at)
}

/// A hexahedron from its 8 corners (corner `i` is `+x` if bit 1, `+y` if bit
/// 2, `+z` if bit 4), faces wound outward. Collapsed corners make zero-area
/// triangles, which are dropped, so a wedge is a hexahedron too.
fn hexa(corners: [[f32; 3]; 8]) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let quads = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    let triangles = quads
        .iter()
        .flat_map(|[a, b, c, d]| [[*a, *c, *b], [*a, *d, *c]])
        .collect();
    (corners.to_vec(), triangles)
}

fn aabb(min: [f32; 3], max: [f32; 3]) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    hexa(std::array::from_fn(|i| {
        let pick = |bit, axis: usize| if i & bit == 0 { min[axis] } else { max[axis] };
        [pick(1, 0), pick(2, 1), pick(4, 2)]
    }))
}

/// A ramp rising `angle` along +x from (x0, 0), `run` long, z in [-5, 5].
fn wedge(x0: f32, run: f32, angle: f32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let top = run * angle.tan();
    hexa(std::array::from_fn(|i| {
        let x = if i & 1 == 0 { x0 } else { x0 + run };
        let y = if i & 2 != 0 && i & 1 != 0 { top } else { 0.0 };
        let z = if i & 4 == 0 { -5.0 } else { 5.0 };
        [x, y, z]
    }))
}

/// Merge pieces into one `Physics.trimesh`, as a level would be.
fn level(pieces: &[(Vec<[f32; 3]>, Vec<[u32; 3]>)]) -> Body {
    let (mut positions, mut triangles) = (Vec::new(), Vec::new());
    for (p, t) in pieces {
        let base = positions.len() as u32;
        positions.extend_from_slice(p);
        triangles.extend(t.iter().map(|tri| tri.map(|i| i + base)));
    }
    let mesh = Arc::new(TriMeshData::new(positions, triangles).unwrap());
    Body::fixed("level".to_string(), Shape::TriMesh { mesh })
}

/// The floor y = 0 as a slab, x and z in [-20, 20].
fn floor() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    aabb([-20.0, -1.0, -20.0], [20.0, 0.0, 20.0])
}

/// A world holding `bodies`, stepped once so queries see them.
fn world(bodies: Vec<Body>) -> World {
    let mut w = World::new([0.0, GRAVITY, 0.0]);
    w.reconcile(&PhysicsScene::create([0.0, GRAVITY, 0.0], bodies));
    w.step_fixed();
    w
}

/// The game's side of a character: steer horizontally at `velocity` (m/s),
/// keep a vertical speed that gravity grows and the ground resets, and move
/// by both each step. Returns the centre and grounded flag per step.
fn walk(w: &mut World, velocity: [f32; 2], steps: usize) -> Vec<([f32; 3], bool)> {
    let mut vy = 0.0;
    (0..steps)
        .map(|_| {
            let grounded = w.grounded("player").unwrap();
            vy = if grounded { 0.0 } else { vy } + GRAVITY * FIXED_DT;
            let translation = [velocity[0] * FIXED_DT, vy * FIXED_DT, velocity[1] * FIXED_DT];
            w.queue_command(PhysicsCommand::Move {
                tag: "player".to_string(),
                translation,
            });
            w.step_frame(FIXED_DT);
            assert!(w.take_command_warnings().is_empty());
            (w.body_transform("player").unwrap().0, w.grounded("player").unwrap())
        })
        .collect()
}

fn last(trace: &[([f32; 3], bool)]) -> [f32; 3] {
    trace.last().unwrap().0
}

#[test]
fn a_character_walks_across_a_trimesh_floor() {
    let mut w = world(vec![level(&[floor()]), player([0.0, STAND, 0.0])]);
    let trace = walk(&mut w, [2.0, 0.0], 120);
    let p = last(&trace);
    assert!((p[0] - 4.0).abs() < 0.05, "walked 2 m/s for 2 s: {p:?}");
    assert!((p[1] - STAND).abs() < 0.01, "stays on the floor: {p:?}");
    assert!(trace[1..].iter().all(|(_, g)| *g), "grounded throughout");
}

#[test]
fn a_character_climbs_a_step_up_to_step_height_and_not_a_taller_one() {
    let climb = |rise: f32| {
        let step = aabb([2.0, -1.0, -5.0], [6.0, rise, 5.0]);
        let mut w = world(vec![level(&[floor(), step]), player([0.0, STAND, 0.0])]);
        last(&walk(&mut w, [2.0, 0.0], 150))
    };
    let p = climb(0.3);
    assert!(p[0] > 4.0, "walked on across the tread: {p:?}");
    assert!((p[1] - (0.3 + STAND)).abs() < 0.01, "stands on the tread: {p:?}");
    let p = climb(0.45);
    assert!(p[0] < 2.0 - 0.3 + 0.01, "blocked at the riser: {p:?}");
    assert!((p[1] - STAND).abs() < 0.01, "still on the floor: {p:?}");
}

#[test]
fn a_character_slides_along_a_wall() {
    let wall = aabb([2.0, -1.0, -20.0], [3.0, 3.0, 20.0]);
    let mut w = world(vec![level(&[floor(), wall]), player([0.0, STAND, 0.0])]);
    let p = last(&walk(&mut w, [2.0, 2.0], 120));
    assert!(p[0] < 2.0 - 0.3 && p[0] > 2.0 - 0.3 - 0.05, "held at the wall: {p:?}");
    assert!(p[2] > 3.5, "kept sliding along it: {p:?}");
}

#[test]
fn a_character_stays_grounded_down_a_walkable_slope_and_slides_down_a_steep_one() {
    // 30°: walk down from the top; standing still holds.
    let slope = 30f32.to_radians();
    let top = 6.0 * slope.tan();
    let start = [5.0, top + STAND / slope.cos(), 0.0];
    let mut w = world(vec![level(&[floor(), wedge(0.0, 6.0, slope)]), player(start)]);
    let held = last(&walk(&mut w, [0.0, 0.0], 60));
    assert!((held[0] - start[0]).abs() < 0.01, "no slide on 30°: {held:?}");
    let trace = walk(&mut w, [-1.5, 0.0], 300);
    assert!(trace.iter().all(|(_, g)| *g), "grounded all the way down");
    let p = last(&trace);
    assert!(p[0] < 0.0 && (p[1] - STAND).abs() < 0.01, "reached the floor: {p:?}");

    // 60°: standing on it slides down, and is not grounded.
    let slope = 60f32.to_radians();
    let start = [1.5, 1.5 * slope.tan() + STAND / slope.cos(), 0.0];
    let mut w = world(vec![level(&[floor(), wedge(0.0, 3.0, slope)]), player(start)]);
    let trace = walk(&mut w, [0.0, 0.0], 30);
    assert!(!trace[5].1, "not grounded on 60°");
    let p = last(&trace);
    assert!(p[1] < start[1] - 0.5, "slid down: {p:?}");
}

/// A zero-thickness wall, from either side and at any speed: every move is
/// swept, so nothing tunnels (sweeps are two-sided).
#[test]
fn a_character_does_not_tunnel_through_a_thin_trimesh_wall() {
    // The plane x = 2, facing -x.
    let wall = (
        vec![[2.0, -1.0, -5.0], [2.0, -1.0, 5.0], [2.0, 4.0, 5.0], [2.0, 4.0, -5.0]],
        vec![[0, 1, 2], [0, 2, 3]],
    );
    for (from, speed) in [(0.0, 1.5), (0.0, 50.0), (4.0, -1.5), (4.0, -50.0)] {
        let mut w = world(vec![level(&[floor(), wall.clone()]), player([from, STAND, 0.0])]);
        let trace = walk(&mut w, [speed, 0.0], 120);
        let side = |x: f32| (x - 2.0).signum();
        assert!(
            trace.iter().all(|(p, _)| side(p[0]) == side(from)),
            "crossed from x={from} at {speed} m/s: {:?}",
            last(&trace)
        );
    }
}

#[test]
fn a_character_walks_on_boxes_and_heightfields() {
    let ground = Body::fixed(
        "ground".to_string(),
        Shape::Cuboid {
            extents: [40.0, 1.0, 40.0],
        },
    )
    .at([0.0, -0.5, 0.0]);
    let crate_ = Body::fixed(
        "crate".to_string(),
        Shape::Cuboid {
            extents: [2.0, 0.3, 2.0],
        },
    )
    .at([3.0, 0.15, 0.0]);
    let mut w = world(vec![ground, crate_, player([0.0, STAND, 0.0])]);
    let p = last(&walk(&mut w, [2.0, 0.0], 90));
    assert!((p[1] - (0.3 + STAND)).abs() < 0.01, "stepped onto the box: {p:?}");

    // A flat 20 m heightfield at half of 0..10 m: y = 5.
    let terrain = Shape::Heightfield {
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
            samples: vec![u16::MAX / 2; 16],
            width: 4,
            height: 4,
            revision: 1,
        })),
    };
    let terrain = Body::fixed("terrain".to_string(), terrain);
    let mut w = world(vec![terrain, player([0.0, 5.0 + STAND + 0.2, 0.0])]);
    let trace = walk(&mut w, [2.0, 0.0], 90);
    let p = last(&trace);
    assert!((p[1] - (5.0 + STAND)).abs() < 0.01 && p[0] > 2.5, "{p:?}");
    assert!(trace.last().unwrap().1);
}

#[test]
fn moves_on_non_characters_warn_and_settings_are_read_per_move() {
    let mut w = world(vec![level(&[floor()]), player([0.0, STAND, 0.0])]);
    w.queue_command(PhysicsCommand::Move {
        tag: "level".to_string(),
        translation: [1.0, 0.0, 0.0],
    });
    w.step_frame(FIXED_DT);
    assert_eq!(
        w.take_command_warnings(),
        ["physics move on \"level\", which is not a Physics.character, has no effect"]
    );
    assert_eq!(w.grounded("level"), None);
    assert_eq!(w.grounded("player"), Some(false), "no move yet");
}

/// The recorded drive: moves are commands, grounded is world state, so a
/// rewind lands bit-exact and the branch replays bit-exact.
#[test]
fn character_moves_replay_and_rewind_bit_exact() {
    remove_world(DEFAULT_WORLD);
    let step = aabb([2.0, -1.0, -5.0], [6.0, 0.3, 5.0]);
    let scene = PhysicsScene::create(
        [0.0, GRAVITY, 0.0],
        vec![level(&[floor(), step]), player([0.0, STAND + 0.5, 0.0])],
    );
    let snapshot = || with_world(DEFAULT_WORLD, |w| w.snapshot()).unwrap();
    let run = |sp: &mut SteppedPhysics, frames: std::ops::Range<u64>| {
        for f in frames {
            let x = if f % 50 < 40 { 2.0 } else { -1.0 };
            with_world(DEFAULT_WORLD, |w| {
                w.queue_command(PhysicsCommand::Move {
                    tag: "player".to_string(),
                    translation: [x * FIXED_DT, -0.05, 0.3 * FIXED_DT],
                })
            });
            sp.advance(&scene, FIXED_DT);
        }
    };
    let mut sp = SteppedPhysics::new();
    run(&mut sp, 0..60);
    let at_60 = snapshot();
    run(&mut sp, 60..150);
    let end = snapshot();
    assert!(with_world(DEFAULT_WORLD, |w| w.grounded("player")).flatten().unwrap());

    assert!(sp.rewind_to_frame(60).is_empty());
    assert!(snapshot() == at_60, "a rewind restores bit-exact");
    run(&mut sp, 60..150);
    assert!(snapshot() == end, "the branch replays bit-exact");

    // A fresh run of the same inputs matches too.
    remove_world(DEFAULT_WORLD);
    let mut fresh = SteppedPhysics::new();
    run(&mut fresh, 0..150);
    assert!(snapshot() == end, "two runs agree bit-exact");
    remove_world(DEFAULT_WORLD);
}
