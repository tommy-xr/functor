//! Indexed triangle meshes: primitive generators plus the measurements that
//! verify a mesh is printable (closed, positive volume).

use std::collections::{HashMap, HashSet};
use std::f64::consts::{PI, TAU};

use cgmath::{BaseFloat, InnerSpace, Matrix4, Point3, SquareMatrix, Transform, Vector3};

/// The unit outward normal of counter-clockwise triangle `a b c` (zero when
/// degenerate) — shared by STL export and the renderer so they cannot drift.
pub fn face_normal<S: BaseFloat>(a: Vector3<S>, b: Vector3<S>, c: Vector3<S>) -> Vector3<S> {
    let n = (b - a).cross(c - a);
    if n.magnitude2() > S::zero() {
        n.normalize()
    } else {
        n
    }
}

/// Counter-clockwise triangles (seen from outside) over shared vertices, in mm.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Mesh {
    /// Append `other` transformed by `xform`. A mirroring transform flips
    /// winding, so triangles are reversed to keep them facing outward.
    pub fn append(&mut self, other: &Mesh, xform: Matrix4<f64>) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices.iter().map(|&v| {
            let p = xform.transform_point(Point3::from(v));
            [p.x, p.y, p.z]
        }));
        let mirrored = xform.determinant() < 0.0;
        self.triangles
            .extend(other.triangles.iter().map(|&[a, b, c]| {
                let [a, b, c] = [a, b, c].map(|i| i + base);
                if mirrored {
                    [a, c, b]
                } else {
                    [a, b, c]
                }
            }));
    }

    /// Append `other` as is.
    pub fn extend(&mut self, other: &Mesh) {
        self.append(other, Matrix4::identity());
    }

    /// Signed volume (divergence theorem); positive when facing outward.
    pub fn volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|&[a, b, c]| {
                let [a, b, c] = [a, b, c].map(|i| Vector3::from(self.vertices[i as usize]));
                a.dot(b.cross(c))
            })
            .sum::<f64>()
            / 6.0
    }

    /// `None` for an empty mesh.
    pub fn bounds(&self) -> Option<Bounds> {
        let first = *self.vertices.first()?;
        Some(self.vertices.iter().fold(
            Bounds {
                min: first,
                max: first,
            },
            |b, v| Bounds {
                min: std::array::from_fn(|i| b.min[i].min(v[i])),
                max: std::array::from_fn(|i| b.max[i].max(v[i])),
            },
        ))
    }

    /// Edges not shared by exactly two oppositely wound triangles. Zero means
    /// the mesh is closed and consistently oriented.
    pub fn non_manifold_edges(&self) -> usize {
        // Undirected edge -> (faces using it, +1/-1 per winding direction).
        let mut edges: HashMap<(u32, u32), (u32, i32)> = HashMap::new();
        for &[a, b, c] in &self.triangles {
            for (u, v) in [(a, b), (b, c), (c, a)] {
                let (key, dir) = if u < v { ((u, v), 1) } else { ((v, u), -1) };
                let edge = edges.entry(key).or_default();
                edge.0 += 1;
                edge.1 += dir;
            }
        }
        edges
            .values()
            .filter(|&&(faces, dir)| faces != 2 || dir != 0)
            .count()
    }

    /// Vertices at the same f32 position as an earlier vertex. STL stores
    /// positions, not indices, so readers join such vertices: a nonzero count
    /// means separate pieces touch at a point, edge, or face — not a solid.
    pub fn shared_positions(&self) -> usize {
        let mut seen = HashSet::new();
        self.vertices
            .iter()
            .filter(|v| !seen.insert(v.map(|x| (x as f32).to_bits())))
            .count()
    }

    /// The f32 positions (as bits) more than one vertex rounds to, each with
    /// all its vertices: the points behind [`Self::shared_positions`].
    pub(crate) fn shared_points(&self) -> HashMap<[u32; 3], Vec<[f64; 3]>> {
        let key = |v: &[f64; 3]| v.map(|x| (x as f32).to_bits());
        let mut seen = HashSet::new();
        let shared: HashSet<_> = self
            .vertices
            .iter()
            .map(key)
            .filter(|k| !seen.insert(*k))
            .collect();
        let mut points: HashMap<_, Vec<_>> = HashMap::new();
        for v in self.vertices.iter().filter(|v| shared.contains(&key(v))) {
            points.entry(key(v)).or_default().push(*v);
        }
        points
    }

    /// Why slicers would not read this mesh as one closed solid, each
    /// sentence naming `subject` (e.g. "`shell`"); empty when it prints.
    /// The one check behind `functor export` and `Solid.check`.
    pub fn print_problems(&self, subject: &str) -> Vec<String> {
        if self.triangles.is_empty() {
            return vec![format!("{subject} is empty")];
        }
        // Overflowed coordinates would also read as shared (equal infinities).
        let overflows = |x: &f64| !(*x as f32).is_finite();
        if self.vertices.iter().flatten().any(overflows) {
            return vec![format!(
                "{subject} is too large to export: coordinates must fit in 32-bit floats"
            )];
        }
        let mut problems = Vec::new();
        let shared = self.shared_positions();
        if shared > 0 {
            problems.push(format!(
                "{subject} would not slice as a solid: {shared} vertices of separate pieces \
land on the same point, joining them at a corner or edge. Overlap the pieces in the \
`Solid.group` slightly (or move them apart) instead of letting them touch exactly."
            ));
        }
        let open = self.non_manifold_edges();
        if open > 0 {
            problems.push(format!(
                "{subject} would not slice as a solid: {open} edges are open or shared by \
more than two faces"
            ));
        }
        problems
    }
}

/// Triangles covering a planar polygon (3D corners in order, either winding),
/// as indices into `corners`: ear clipping in the polygon's plane, so concave
/// outlines work. Repeated corners and zero-area triangles are dropped — they
/// would print as stray rods.
pub fn triangulate_polygon(corners: &[[f64; 3]]) -> Vec<[usize; 3]> {
    // Drop consecutive repeats (incl. last == first).
    let mut ids: Vec<usize> = (0..corners.len()).collect();
    ids.dedup_by(|b, a| corners[*a] == corners[*b]);
    while ids.len() > 1 && corners[ids[0]] == corners[ids[ids.len() - 1]] {
        ids.pop();
    }
    // Newell's normal; project out its largest axis.
    let mut n = [0.0; 3];
    for (k, &i) in ids.iter().enumerate() {
        let (a, b) = (corners[i], corners[ids[(k + 1) % ids.len()]]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let drop = (0..3)
        .max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()))
        .unwrap();
    let (u, v) = ((drop + 1) % 3, (drop + 2) % 3);
    let sign = n[drop].signum();
    let p = |i: usize| [corners[i][u], corners[i][v]];
    // Twice the signed area of a, b, c, positive when it turns the polygon's way.
    let turn = |a: usize, b: usize, c: usize| {
        let (a, b, c) = (p(a), p(b), p(c));
        sign * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]))
    };
    let scale = n.iter().map(|x| x.abs()).fold(0.0, f64::max);
    let eps = 1e-12 * scale.max(f64::MIN_POSITIVE);

    let mut out = Vec::new();
    while ids.len() > 3 {
        let m = ids.len();
        let ear = (0..m).find(|&k| {
            let (a, b, c) = (ids[(k + m - 1) % m], ids[k], ids[(k + 1) % m]);
            turn(a, b, c) > eps
                && ids.iter().all(|&q| {
                    q == a
                        || q == b
                        || q == c
                        || turn(a, b, q) < 0.0
                        || turn(b, c, q) < 0.0
                        || turn(c, a, q) < 0.0
                })
        });
        let Some(k) = ear else { break };
        out.push([ids[(k + m - 1) % m], ids[k], ids[(k + 1) % m]]);
        ids.remove(k);
    }
    // Whatever is left (a final triangle, or a self-crossing outline) fans.
    out.extend(ids.windows(2).skip(1).map(|w| [ids[0], w[0], w[1]]));
    out.retain(|&[a, b, c]| turn(a, b, c).abs() > eps);
    out
}

pub(crate) fn boxed([x, y, z]: [f64; 3]) -> Mesh {
    let (x, y, z) = (x / 2.0, y / 2.0, z / 2.0);
    let vertices = (0..8)
        .map(|i| {
            let pick = |bit: usize, h: f64| if i & bit == 0 { -h } else { h };
            [pick(1, x), pick(2, y), pick(4, z)]
        })
        .collect();
    // Vertex i has x from bit 0, y from bit 1, z from bit 2.
    // Two triangles per face, in face order -z, +z, -y, +y, -x, +x.
    let triangles = vec![
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    Mesh {
        vertices,
        triangles,
    }
}

/// Ring vertices around Z at height `z`, starting on +X, counter-clockwise.
pub(crate) fn ring(radius: f64, z: f64, segments: usize) -> impl Iterator<Item = [f64; 3]> {
    (0..segments).map(move |i| {
        let a = TAU * i as f64 / segments as f64;
        [radius * a.cos(), radius * a.sin(), z]
    })
}

/// A Z-axis frustum from `bottom` radius at -h/2 to `top` at +h/2. A zero
/// radius ends in an apex vertex instead of a ring (a pointed cone).
pub(crate) fn cone(bottom: f64, top: f64, height: f64, segments: usize) -> Mesh {
    let n = segments as u32;
    let h = height / 2.0;
    let mut vertices = Vec::new();
    // Each end: the first vertex index and whether it is a ring (else an apex).
    let mut end = |r: f64, z: f64| {
        let first = vertices.len() as u32;
        if r > 0.0 {
            vertices.extend(ring(r, z, segments));
            vertices.push([0.0, 0.0, z]); // cap center
        } else {
            vertices.push([0.0, 0.0, z]);
        }
        (first, r > 0.0)
    };
    let (lo, lo_ring) = end(bottom, -h);
    let (hi, hi_ring) = end(top, h);
    let mut triangles = Vec::with_capacity(4 * segments);
    for i in 0..n {
        let j = (i + 1) % n;
        match (lo_ring, hi_ring) {
            (true, true) => {
                triangles.push([lo + i, lo + j, hi + j]);
                triangles.push([lo + i, hi + j, hi + i]);
            }
            (true, false) => triangles.push([lo + i, lo + j, hi]),
            (false, true) => triangles.push([lo, hi + j, hi + i]),
            (false, false) => unreachable!("one radius is positive"),
        }
        if lo_ring {
            triangles.push([lo + n, lo + j, lo + i]);
        }
        if hi_ring {
            triangles.push([hi + n, hi + i, hi + j]);
        }
    }
    Mesh {
        vertices,
        triangles,
    }
}

/// A UV sphere: `segments` around, `segments / 2` bands pole to pole.
pub(crate) fn sphere(radius: f64, segments: usize) -> Mesh {
    let n = segments as u32;
    let bands = (segments / 2).max(2);
    // Rings 1..bands-1 (pole-free), then the south and north poles.
    let mut vertices: Vec<[f64; 3]> = (1..bands)
        .flat_map(|k| {
            let polar = PI * k as f64 / bands as f64;
            ring(radius * polar.sin(), -radius * polar.cos(), segments)
        })
        .collect();
    let (south, north) = (vertices.len() as u32, vertices.len() as u32 + 1);
    vertices.extend([[0.0, 0.0, -radius], [0.0, 0.0, radius]]);
    let rings = bands as u32 - 1;
    let mut triangles = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        triangles.push([south, j, i]);
        for r in 0..rings - 1 {
            let (lo, hi) = (r * n, (r + 1) * n);
            triangles.push([lo + i, lo + j, hi + j]);
            triangles.push([lo + i, hi + j, hi + i]);
        }
        let last = (rings - 1) * n;
        triangles.push([north, last + i, last + j]);
    }
    Mesh {
        vertices,
        triangles,
    }
}

#[cfg(test)]
mod tests {
    fn polygon_area(corners: &[[f64; 3]]) -> f64 {
        super::triangulate_polygon(corners)
            .iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (corners[a], corners[b], corners[c]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn polygons_triangulate_concave_and_drop_degenerates() {
        let flat = |pts: &[[f64; 2]]| pts.iter().map(|&[x, y]| [x, y, 0.0]).collect::<Vec<_>>();
        // A U: area 7, where a fan would cover 11.
        let u = flat(&[
            [0.0, 0.0],
            [3.0, 0.0],
            [3.0, 3.0],
            [2.0, 3.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 3.0],
            [0.0, 3.0],
        ]);
        assert!((polygon_area(&u) - 7.0).abs() < 1e-9);
        // Either winding.
        let mut reversed = u.clone();
        reversed.reverse();
        assert!((polygon_area(&reversed) - 7.0).abs() < 1e-9);
        // A square with a repeated corner and a collinear midpoint: 5 corners, 3 triangles.
        let square = flat(&[
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.0],
            [2.0, 0.0],
            [2.0, 2.0],
            [0.0, 2.0],
        ]);
        assert_eq!(super::triangulate_polygon(&square).len(), 3);
        assert!((polygon_area(&square) - 4.0).abs() < 1e-9);
        // Degenerate: a "triangle" with a repeated corner, and a line.
        assert!(
            super::triangulate_polygon(&flat(&[[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]])).is_empty()
        );
        assert!(
            super::triangulate_polygon(&flat(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]])).is_empty()
        );
    }
}
