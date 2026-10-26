//! "Make printable": rebuild any triangle soup as a closed solid. Game meshes
//! are rarely watertight — open edges, seams, overlapping shells, zero-thickness
//! cards — so instead of repairing them we resample: every point within
//! `offset` of the surface is solid, as is everything the outside can't reach,
//! and the boundary is extracted from that voxel field.

use std::collections::{HashMap, VecDeque};

use crate::Mesh;

/// Largest voxel grid built (lattice points). Each point costs ~4 bytes (one
/// f32, reused from distance to signed field, plus a bit), so this caps the
/// field near 1 GB.
const MAX_CELLS: usize = 250_000_000;

/// Remesh `soup` (mm) as a closed solid sampled every `voxel` mm, thickening
/// every surface by `offset` mm so cards and thin features survive.
/// The offset is raised to at least ¾ voxel, so a thickened sheet always
/// contains a lattice point and can't vanish between samples.
pub fn printable(soup: &Mesh, voxel: f64, offset: f64) -> Result<Mesh, String> {
    let grid = voxelize(soup, voxel, offset.max(0.75 * voxel))?;
    Ok(marching_tets(&grid))
}

/// The finest voxel, starting from `voxel`, whose grid fits under the cap.
pub fn fitting_voxel(soup: &Mesh, voxel: f64, offset: f64) -> f64 {
    let Some(b) = soup.bounds() else { return voxel };
    let mut h = voxel;
    loop {
        let pad = 2.0 * (offset.max(0.75 * h) + 4.0 * h);
        let cells: f64 = (0..3)
            .map(|i| (b.max[i] - b.min[i] + pad) / h + 2.0)
            .product();
        if cells <= MAX_CELLS as f64 {
            return h;
        }
        h *= 1.25;
    }
}

struct Grid {
    n: [usize; 3],
    origin: [f64; 3],
    h: f64,
    /// Signed field per lattice point: negative inside the solid.
    f: Vec<f32>,
}

impl Grid {
    fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        x + self.n[0] * (y + self.n[1] * z)
    }

    fn pos(&self, x: usize, y: usize, z: usize) -> [f64; 3] {
        [
            self.origin[0] + x as f64 * self.h,
            self.origin[1] + y as f64 * self.h,
            self.origin[2] + z as f64 * self.h,
        ]
    }
}

fn voxelize(m: &Mesh, h: f64, offset: f64) -> Result<Grid, String> {
    let b = m.bounds().ok_or("the model has no geometry")?;
    let band = offset + 2.0 * h;
    let pad = band + 2.0 * h;
    let origin = b.min.map(|x| x - pad);
    let n: [usize; 3] =
        std::array::from_fn(|i| ((b.max[i] - b.min[i] + 2.0 * pad) / h).ceil() as usize + 1);
    let cells = n[0].saturating_mul(n[1]).saturating_mul(n[2]);
    if cells > MAX_CELLS {
        return Err(format!(
            "a {h} mm voxel grid would need {} M cells (limit {} M): use a coarser voxel or a smaller size",
            cells / 1_000_000,
            MAX_CELLS / 1_000_000
        ));
    }
    let mut grid = Grid {
        n,
        origin,
        h,
        // Squared distance to the nearest triangle (within the band), turned
        // into the signed field in place below.
        f: vec![(band * band) as f32; cells],
    };
    for t in &m.triangles {
        let [a, bb, c] = t.map(|i| m.vertices[i as usize]);
        let lo: [usize; 3] = std::array::from_fn(|i| {
            ((a[i].min(bb[i]).min(c[i]) - band - origin[i]) / h)
                .floor()
                .max(0.0) as usize
        });
        let hi: [usize; 3] = std::array::from_fn(|i| {
            (((a[i].max(bb[i]).max(c[i]) + band - origin[i]) / h).ceil() as usize).min(n[i] - 1)
        });
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let dd = dist2_tri(grid.pos(x, y, z), a, bb, c) as f32;
                    let i = grid.idx(x, y, z);
                    if dd < grid.f[i] {
                        grid.f[i] = dd;
                    }
                }
            }
        }
    }
    // Flood fill the outside through points farther than `offset` from the
    // surface; whatever it can't reach is solid.
    let off2 = (offset * offset) as f32;
    let mut outside = vec![0u64; cells.div_ceil(64)];
    let is_out = |bits: &[u64], i: usize| bits[i / 64] >> (i % 64) & 1 == 1;
    let mut queue = VecDeque::from([0usize]);
    outside[0] |= 1;
    while let Some(i) = queue.pop_front() {
        let (x, y, z) = (i % n[0], (i / n[0]) % n[1], i / (n[0] * n[1]));
        let neighbours = [
            (x > 0).then(|| i - 1),
            (x + 1 < n[0]).then(|| i + 1),
            (y > 0).then(|| i - n[0]),
            (y + 1 < n[1]).then(|| i + n[0]),
            (z > 0).then(|| i - n[0] * n[1]),
            (z + 1 < n[2]).then(|| i + n[0] * n[1]),
        ];
        for j in neighbours.into_iter().flatten() {
            if !is_out(&outside, j) && grid.f[j] >= off2 {
                outside[j / 64] |= 1 << (j % 64);
                queue.push_back(j);
            }
        }
    }
    // Keep the sign strictly away from zero so no lattice point sits on the
    // surface (which would make degenerate triangles).
    let eps = 1e-4 * h as f32;
    for (i, f) in grid.f.iter_mut().enumerate() {
        let v = f.sqrt() - offset as f32;
        *f = if is_out(&outside, i) {
            v.max(eps)
        } else {
            v.min(-eps)
        };
    }
    Ok(grid)
}

/// Marching tetrahedra (six tets per cell around the shared 0–7 diagonal): on a
/// padded grid this always yields a closed 2-manifold, with no lookup table.
/// Vertices are shared per lattice edge.
fn marching_tets(g: &Grid) -> Mesh {
    const C: [[usize; 3]; 8] = [
        [0, 0, 0],
        [1, 0, 0],
        [0, 1, 0],
        [1, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [0, 1, 1],
        [1, 1, 1],
    ];
    const T: [[usize; 4]; 6] = [
        [0, 1, 3, 7],
        [0, 1, 5, 7],
        [0, 2, 3, 7],
        [0, 2, 6, 7],
        [0, 4, 5, 7],
        [0, 4, 6, 7],
    ];
    let n = g.n;
    let mut out = Mesh::default();
    let mut edge_vertex: HashMap<(usize, usize), u32> = HashMap::new();
    for z in 0..n[2] - 1 {
        for y in 0..n[1] - 1 {
            for x in 0..n[0] - 1 {
                let ids = C.map(|c| g.idx(x + c[0], y + c[1], z + c[2]));
                let fs = ids.map(|i| g.f[i]);
                if fs.iter().all(|&v| v > 0.0) || fs.iter().all(|&v| v < 0.0) {
                    continue;
                }
                let ps = C.map(|c| g.pos(x + c[0], y + c[1], z + c[2]));
                // The surface crossing on lattice edge p–q.
                let mut cross = |p: usize, q: usize, out: &mut Mesh| -> u32 {
                    let key = (ids[p].min(ids[q]), ids[p].max(ids[q]));
                    *edge_vertex.entry(key).or_insert_with(|| {
                        let t = (fs[p] / (fs[p] - fs[q])).clamp(0.02, 0.98) as f64;
                        out.vertices.push(std::array::from_fn(|k| {
                            ps[p][k] + t * (ps[q][k] - ps[p][k])
                        }));
                        out.vertices.len() as u32 - 1
                    })
                };
                for tet in T {
                    let inside: Vec<usize> = tet.into_iter().filter(|&k| fs[k] < 0.0).collect();
                    let outside: Vec<usize> = tet.into_iter().filter(|&k| fs[k] > 0.0).collect();
                    let tris: Vec<[u32; 3]> = match inside.len() {
                        1 | 3 => {
                            let (lone, others) = if inside.len() == 1 {
                                (inside[0], &outside)
                            } else {
                                (outside[0], &inside)
                            };
                            let v: Vec<u32> =
                                others.iter().map(|&o| cross(lone, o, &mut out)).collect();
                            vec![[v[0], v[1], v[2]]]
                        }
                        2 => {
                            let (i0, i1, o0, o1) = (inside[0], inside[1], outside[0], outside[1]);
                            let p = cross(i0, o0, &mut out);
                            let q = cross(i0, o1, &mut out);
                            let r = cross(i1, o1, &mut out);
                            let s = cross(i1, o0, &mut out);
                            vec![[p, q, r], [p, r, s]]
                        }
                        _ => continue,
                    };
                    // Orient each triangle to face away from the solid corners.
                    let solid: [f64; 3] = std::array::from_fn(|k| {
                        inside.iter().map(|&i| ps[i][k]).sum::<f64>() / inside.len() as f64
                    });
                    for t in tris {
                        let [a, b, c] = t.map(|i| out.vertices[i as usize]);
                        let normal = cross3(sub(b, a), sub(c, a));
                        out.triangles.push(if dot(normal, sub(a, solid)) < 0.0 {
                            [t[0], t[2], t[1]]
                        } else {
                            t
                        });
                    }
                }
            }
        }
    }
    out
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Squared distance from `p` to triangle `abc` (Ericson, Real-Time Collision
/// Detection §5.1.5).
fn dist2_tri(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let at = |q: [f64; 3]| dot(sub(p, q), sub(p, q));
    if d1 <= 0.0 && d2 <= 0.0 {
        return at(a);
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return at(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return at(std::array::from_fn(|k| a[k] + v * ab[k]));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return at(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return at(std::array::from_fn(|k| a[k] + w * ac[k]));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub(c, b);
        return at(std::array::from_fn(|k| b[k] + w * bc[k]));
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    at(std::array::from_fn(|k| a[k] + ab[k] * v + ac[k] * w))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Solid;

    fn closed(m: &Mesh) {
        assert!(!m.triangles.is_empty());
        assert_eq!(m.non_manifold_edges(), 0);
        assert_eq!(m.shared_positions(), 0);
        assert!(m.volume() > 0.0);
    }

    #[test]
    fn an_open_box_becomes_a_closed_cup() {
        // A 10 mm cube missing its top face: a typical "not watertight" mesh.
        let mut open = Solid::Box { size: [10.0; 3] }.mesh().unwrap();
        open.triangles
            .retain(|t| !t.iter().all(|&i| open.vertices[i as usize][2] > 0.0));
        assert!(open.non_manifold_edges() > 0);
        let cup = printable(&open, 0.2, 0.3).unwrap();
        closed(&cup);
        // The outside flows in through the opening: walls ~0.6 mm thick over
        // ~500 mm² of surface, not a filled 1000 mm³ cube.
        assert!(
            cup.volume() > 200.0 && cup.volume() < 600.0,
            "{}",
            cup.volume()
        );
    }

    #[test]
    fn a_zero_thickness_card_gains_thickness() {
        let card = Mesh {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 10.0, 0.0],
                [0.0, 10.0, 0.0],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        };
        let solid = printable(&card, 0.2, 0.3).unwrap();
        closed(&solid);
        let b = solid.bounds().unwrap();
        assert!((b.max[2] - b.min[2] - 0.6).abs() < 0.2, "{:?}", b);
    }

    #[test]
    fn simplify_shrinks_a_remesh_and_keeps_it_closed() {
        let ball = Solid::Sphere { radius: 10.0 }.mesh().unwrap();
        let dense = printable(&ball, 0.2, 0.15).unwrap();
        let lean = crate::simplify(&dense, 0.05).unwrap();
        closed(&lean);
        assert!(
            lean.triangles.len() * 3 < dense.triangles.len(),
            "{} -> {}",
            dense.triangles.len(),
            lean.triangles.len()
        );
        assert!((lean.volume() - dense.volume()).abs() < 0.01 * dense.volume());
    }

    #[test]
    fn oversized_grids_are_refused() {
        let big = Solid::Box { size: [1000.0; 3] }.mesh().unwrap();
        assert!(printable(&big, 0.1, 0.05)
            .unwrap_err()
            .contains("voxel grid"));
    }
}
