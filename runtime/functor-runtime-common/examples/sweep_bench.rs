//! `sweep_bench` — one `World::sweep` (nonlinear: turning while it moves)
//! against one linear `World::shape_cast`, in a vr-lab-sized world.
//!
//! The world is a floor, a table, a wall and a basket (fixed boxes, fused as
//! the runtime fuses them). Each case sweeps a held stick one 60 Hz step's
//! worth of hand motion: clear, into the wall, and turning in place into it;
//! then clear and turning down into flat terrain (the segmented path: parry
//! has no turning cast against a heightfield).
//! Reports the min and median per call over batches. Timings are desktop;
//! the Quest estimate is ×4.
//!
//! ```sh
//! cargo run -q --release -p functor_runtime_common --example sweep_bench
//! ```

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use functor_runtime_common::asset::pipelines::HeightmapData;
use functor_runtime_common::terrain::{TerrainGeometry, TerrainSource};

use functor_runtime_common::physics::{Body, PhysicsScene, Shape, SweepFilter, World, FIXED_DT};
use functor_runtime_common::MappedTrackingPose;

const BATCHES: usize = 50;
const CALLS: usize = 2000;

fn cuboid(tag: &str, at: [f32; 3], size: [f32; 3]) -> Body {
    Body::fixed(tag.to_string(), Shape::Cuboid { extents: size }).at(at)
}

fn pose(at: [f32; 3], forward: [f32; 3]) -> ([f32; 3], [f32; 4]) {
    MappedTrackingPose { position: at, forward, up: [0.0, 1.0, 0.0] }
        .to_position_rotation()
        .unwrap()
}

/// Min and median microseconds per call of `f`.
fn time(mut f: impl FnMut() -> bool) -> (f64, f64) {
    let mut per_call: Vec<f64> = (0..BATCHES)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..CALLS {
                black_box(f());
            }
            start.elapsed().as_secs_f64() * 1e6 / CALLS as f64
        })
        .collect();
    per_call.sort_by(f64::total_cmp);
    (per_call[0], per_call[BATCHES / 2])
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("warning: a debug build; run with --release");
    }
    let mut bodies = vec![
        cuboid("floor", [0.0, -0.05, 0.0], [30.0, 0.1, 30.0]),
        cuboid("table", [0.0, 0.45, 4.6], [1.2, 0.9, 0.6]),
        cuboid("wall", [0.65, 1.0, 4.6], [0.1, 2.0, 1.6]),
    ];
    for i in 0..5 {
        bodies.push(cuboid(&format!("basket-{i}"), [-0.2 + 0.3 * i as f32, 0.17, 7.1], [0.03, 0.35, 0.6]));
    }
    let world = |bodies: Vec<Body>| {
        let mut w = World::new([0.0, -9.81, 0.0]);
        w.reconcile(&PhysicsScene::create([0.0, -9.81, 0.0], bodies));
        w.step_frame(FIXED_DT);
        w
    };
    let boxes = world(bodies);
    // Terrain has no turning cast, so its sweeps take the segmented path.
    let terrain = world(vec![Body::fixed(
        "terrain".to_string(),
        Shape::Heightfield {
            geometry: TerrainGeometry {
                source: TerrainSource { locator: "flat.png".to_string(), while_pending: Vec::new() },
                width: 30.0,
                depth: 30.0,
                min_height: 0.0,
                max_height: 1.0,
            },
            data: Some(Arc::new(HeightmapData::flat())),
        },
    )]);

    let stick = Shape::Cuboid { extents: [0.04, 0.04, 0.6] };
    // One step of a 1.2 m/s hand: 2 cm. The stick lies along +Z unless it
    // turns, so the linear cast sweeps the same shape.
    let z = [0.0, 0.0, 1.0];
    let cases = [
        ("clear", &boxes, [0.0, 1.2, 4.4], [0.02, 1.2, 4.4], z, z),
        ("into the wall", &boxes, [0.565, 1.2, 4.4], [0.585, 1.2, 4.4], z, z),
        ("turning in place", &boxes, [0.35, 1.2, 4.4], [0.35, 1.2, 4.4], z, [1.0, 0.0, 0.2]),
        ("terrain: clear", &terrain, [0.0, 1.2, 0.0], [0.02, 1.2, 0.0], z, z),
        ("terrain: turn, clear", &terrain, [0.0, 1.2, 0.0], [0.0, 1.2, 0.0], z, [0.0, -1.0, 0.2]),
        ("terrain: turning", &terrain, [0.0, 0.2, 0.0], [0.0, 0.2, 0.0], z, [0.0, -1.0, 0.2]),
    ];
    println!("sweep_bench: one call, a 0.6 m stick, vr-lab-sized world ({CALLS} calls x {BATCHES} batches)\n");
    println!("  {:<18} {:>10} {:>10} {:>10} {:>10}  hits", "case", "sweep min", "med", "cast min", "med");
    for (name, w, a, b, fa, fb) in cases {
        let (from, to) = (pose(a, fa), pose(b, fb));
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let sweep = || w.sweep(&stick, from, to, SweepFilter::Fixed).is_some();
        // The linear cast can't turn and needs a direction: along the centre's
        // motion, or straight up when the centre doesn't move.
        let (dir, max) = if len > 0.0 { (d, len) } else { ([0.0, 1.0, 0.0], 1.0e-3) };
        let cast = || w.shape_cast(&stick, a, dir, max, None).is_some();
        let hits = (sweep(), cast());
        let ((s_min, s_med), (c_min, c_med)) = (time(sweep), time(cast));
        println!(
            "  {name:<18} {s_min:>8.2}us {s_med:>8.2}us {c_min:>8.2}us {c_med:>8.2}us  sweep {} / cast {}",
            hits.0, hits.1
        );
    }
}
