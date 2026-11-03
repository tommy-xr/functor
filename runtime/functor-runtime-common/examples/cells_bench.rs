//! `Scene.cells` portal-walk cost from outside a building: one graph of a
//! row of rooms, each with a window onto the street, walked from an eye on
//! the street facing them. Per window count: the walk's time (min of 5
//! rounds, each the mean of many walks) and the cells it reached.
//!
//! ```sh
//! cargo run -q --release -p functor_runtime_common --example cells_bench
//! ```

use std::time::Instant;

use cgmath::{perspective, Deg, Matrix4, Point3, Vector3};
use functor_runtime_common::cells::{Cell, Cells, CellsData, Plane, Portal, OUTSIDE};

/// `n` 4 × 3 × 4 rooms along X, a doorway between neighbours and a window in
/// each south wall (z = 0) onto the outside.
fn row(n: usize) -> Cells {
    let room = |x: f32| Cell {
        planes: vec![
            Plane {
                n: [1.0, 0.0, 0.0],
                d: -x,
            },
            Plane {
                n: [-1.0, 0.0, 0.0],
                d: x + 4.0,
            },
            Plane {
                n: [0.0, 1.0, 0.0],
                d: 0.0,
            },
            Plane {
                n: [0.0, -1.0, 0.0],
                d: 3.0,
            },
            Plane {
                n: [0.0, 0.0, 1.0],
                d: 0.0,
            },
            Plane {
                n: [0.0, 0.0, -1.0],
                d: 4.0,
            },
        ],
    };
    let mut portals = Vec::new();
    for i in 0..n {
        let x = 4.0 * i as f32;
        portals.push(Portal {
            from: OUTSIDE,
            to: i as u32,
            polygon: vec![
                [x + 1.0, 1.0, 0.0],
                [x + 3.0, 1.0, 0.0],
                [x + 3.0, 2.0, 0.0],
                [x + 1.0, 2.0, 0.0],
            ],
        });
        if i + 1 < n {
            let x = x + 4.0;
            let door = vec![[x, 0.0, 1.5], [x, 0.0, 2.5], [x, 2.0, 2.5], [x, 2.0, 1.5]];
            let mut back = door.clone();
            back.reverse();
            portals.push(Portal {
                from: i as u32,
                to: i as u32 + 1,
                polygon: door,
            });
            portals.push(Portal {
                from: i as u32 + 1,
                to: i as u32,
                polygon: back,
            });
        }
    }
    Cells::new(CellsData {
        cells: (0..n).map(|i| room(4.0 * i as f32)).collect(),
        portals,
        ..CellsData::default()
    })
    .unwrap()
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("cells_bench: a debug build; pass --release for real numbers");
    }
    println!(
        "{:>8} {:>6} {:>6} {:>10} {:>8} {:>8} {:>5}",
        "windows", "away", "min", "us/walk", "visible", "portals", "cut"
    );
    for n in [4, 16, 64, 256] {
        let width = 4.0 * n as f32;
        // On the street facing the middle of the row: near, and far with
        // and without a minimum portal size.
        for (away, min) in [(10.0, 0.0), (100.0, 0.0), (100.0, 0.02)] {
            let cells = row(n).with_min_portal(min).unwrap();
            let eye = [width / 2.0, 1.6, -away];
            let clip = perspective(Deg(90.0), 1.0, 0.1, 1000.0)
                * Matrix4::look_at_rh(
                    Point3::from(eye),
                    Point3::new(width / 2.0, 1.6, 0.0),
                    Vector3::unit_y(),
                );
            let walk = cells.visible(eye.into(), &clip);
            let visible = walk
                .cells
                .as_ref()
                .map_or(n, |v| v.iter().filter(|&&s| s).count());
            let iters = 200_000 / n;
            let best = (0..5)
                .map(|_| {
                    let start = Instant::now();
                    for _ in 0..iters {
                        std::hint::black_box(
                            cells.visible(std::hint::black_box(eye.into()), &clip),
                        );
                    }
                    start.elapsed().as_secs_f64() * 1e6 / iters as f64
                })
                .fold(f64::INFINITY, f64::min);
            println!(
                "{n:>8} {away:>6} {min:>6} {best:>10.2} {visible:>8} {:>8} {:>5}",
                walk.portals, walk.cut
            );
        }
    }
}
