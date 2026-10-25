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

pub(crate) fn cylinder(radius: f64, height: f64, segments: usize) -> Mesh {
    let n = segments as u32;
    let h = height / 2.0;
    // Bottom ring 0..n, top ring n..2n, then bottom and top cap centers.
    let mut vertices: Vec<_> = ring(radius, -h, segments)
        .chain(ring(radius, h, segments))
        .collect();
    vertices.extend([[0.0, 0.0, -h], [0.0, 0.0, h]]);
    let (bottom, top) = (2 * n, 2 * n + 1);
    let mut triangles = Vec::with_capacity(4 * segments);
    for i in 0..n {
        let j = (i + 1) % n;
        triangles.push([i, j, n + j]);
        triangles.push([i, n + j, n + i]);
        triangles.push([bottom, j, i]);
        triangles.push([top, n + i, n + j]);
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
