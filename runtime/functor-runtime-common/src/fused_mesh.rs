//! The fused room: environment depth frames folded into a truncated signed
//! distance volume (TSDF), whose surface is extracted as triangle meshes — so
//! the room fills in as the wearer looks around, and furniture that moves is
//! carved away again (the approach Lasertag and Hauntify use on Quest 3).
//!
//! Deterministic and IO-free: [`Volume::integrate`] folds one depth frame in,
//! [`Volume::extract_dirty`] re-meshes only the chunks whose surface changed.
//! `crate::fused_room` reads the frames back from the GPU and runs this off
//! the render thread.
//!
//! The volume is a hash of 8³-voxel chunks keyed by integer position, so only
//! space near observed surfaces costs memory. Each voxel keeps a weighted
//! average of its signed distance to the surface in front of or behind it,
//! measured along the depth camera's view axis and clamped to ±[`TRUNCATION`]; voxels a ray passes through on its way to a
//! surface read as free space, so a surface that is no longer there decays
//! within about [`MAX_WEIGHT`] observations.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use cgmath::{InnerSpace, Matrix4, SquareMatrix, Vector3, Vector4};

use crate::scene3d::MeshData;

/// Voxel edge, metres.
pub const VOXEL: f32 = 0.08;
/// Voxels along a chunk edge.
const CHUNK: i32 = 8;
const CHUNK_VOXELS: usize = (CHUNK * CHUNK * CHUNK) as usize;
/// Signed distances clamp to ±this; voxels store them divided by it.
const TRUNCATION: f32 = 3.0 * VOXEL;
/// The observations a voxel's average remembers. Lower forgets moved
/// furniture faster; higher smooths more depth noise.
const MAX_WEIGHT: f32 = 32.0;
/// Surfaces beyond this are too noisy to fuse, metres. Farther depth still
/// clears the free space in front of it.
const MAX_RANGE: f32 = 4.0;
/// How far (÷ [`TRUNCATION`]) a voxel may drift from the value its chunk was
/// last meshed with before the chunk re-meshes: about 1 cm of surface.
const REMESH_DRIFT: f32 = 0.05;
/// Pixel stride when finding which chunks a frame's surfaces reach: 4 pixels
/// of a 320-wide image span under a voxel at [`MAX_RANGE`].
const ALLOCATE_STRIDE: usize = 4;

/// A chunk's integer position: chunk `k` spans voxels `k * 8 .. k * 8 + 8`.
pub type ChunkKey = [i32; 3];

/// One depth image: `depth[y * width + x]` is window depth in `[0, 1]` under
/// `view_projection` (world → clip, GL conventions), row 0 at the bottom; 1.0
/// (or more) means no surface.
#[derive(Clone, Debug)]
pub struct DepthFrame {
    pub width: usize,
    pub height: usize,
    pub depth: Vec<f32>,
    pub view_projection: Matrix4<f32>,
}

impl DepthFrame {
    /// Decode an RGBA8 readback whose RGB bytes carry 24-bit window depth,
    /// most significant first (`crate::fused_room`'s pack pass).
    pub fn from_rgba8(
        width: usize,
        height: usize,
        rgba: &[u8],
        view_projection: Matrix4<f32>,
    ) -> DepthFrame {
        let depth = rgba
            .chunks_exact(4)
            .map(|p| {
                let packed = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                packed as f32 / 16_777_215.0
            })
            .collect();
        DepthFrame {
            width,
            height,
            depth,
            view_projection,
        }
    }
}

/// Signed distances (÷ [`TRUNCATION`], negative behind a surface) and their
/// weights (0 = never observed), plus the distances the chunk's mesh was last
/// marked from.
struct Chunk {
    sdf: [f32; CHUNK_VOXELS],
    weight: [f32; CHUNK_VOXELS],
    meshed: [f32; CHUNK_VOXELS],
}

/// The fused volume.
#[derive(Default)]
pub struct Volume {
    chunks: HashMap<ChunkKey, Chunk>,
    /// Chunks whose mesh is out of date.
    dirty: BTreeSet<ChunkKey>,
    /// Depth frames integrated so far.
    pub fused_frames: u64,
}

fn voxel_index(x: i32, y: i32, z: i32) -> usize {
    (x + CHUNK * (y + CHUNK * z)) as usize
}

/// The chunk holding the voxel nearest `p`.
fn chunk_of(p: Vector3<f32>) -> ChunkKey {
    let voxel = (p / VOXEL).map(|c| c.floor() as i32);
    [voxel.x, voxel.y, voxel.z].map(|c| c.div_euclid(CHUNK))
}

/// The world position of chunk `key`'s voxel `(x, y, z)`. Voxels are the
/// lattice points a chunk's cells span, so its cells reach its +1
/// neighbours' first voxels. The lattice sits half a voxel off whole
/// multiples, so a surface at a round height — the stage floor at y = 0 —
/// falls between voxels instead of on them, where a near-zero distance
/// would flip sign with every depth texel.
fn voxel_position(key: ChunkKey, x: i32, y: i32, z: i32) -> Vector3<f32> {
    Vector3::new(
        (key[0] * CHUNK + x) as f32 + 0.5,
        (key[1] * CHUNK + y) as f32 + 0.5,
        (key[2] * CHUNK + z) as f32 + 0.5,
    ) * VOXEL
}

fn unproject(inverse: &Matrix4<f32>, ndc: Vector4<f32>) -> Vector3<f32> {
    let p = inverse * ndc;
    p.truncate() / p.w
}

/// Whether some of chunk `key` may be inside the depth camera's frustum and
/// within range of `eye`.
fn chunk_in_view(view_projection: &Matrix4<f32>, eye: Vector3<f32>, key: ChunkKey) -> bool {
    let size = CHUNK as f32 * VOXEL;
    let min = voxel_position(key, 0, 0, 0);
    let center = min + Vector3::new(size, size, size) * 0.5;
    if (center - eye).magnitude() > MAX_RANGE + size {
        return false;
    }
    // Culled only when all eight corners are outside the same clip plane.
    let mut outside = [true; 5];
    for corner in 0..8 {
        let offset = Vector3::new(
            (corner & 1) as f32,
            ((corner >> 1) & 1) as f32,
            ((corner >> 2) & 1) as f32,
        ) * size;
        let c = view_projection * (min + offset).extend(1.0);
        for (plane, out) in [c.x > c.w, c.x < -c.w, c.y > c.w, c.y < -c.w, c.w <= 0.0]
            .into_iter()
            .enumerate()
        {
            outside[plane] &= out;
        }
    }
    !outside.contains(&true)
}

/// The surface depth at image point `(u, v)` (pixels): bilinear between the
/// four nearest pixels where they agree, so a voxel's distance does not step
/// with the pixel it lands in; the nearest pixel across a depth edge, where
/// blending would invent a surface in between.
fn sample(surface_w: &[f32], width: usize, height: usize, u: f32, v: f32) -> f32 {
    let nearest = surface_w[v as usize * width + u as usize];
    let (x, y) = ((u - 0.5).max(0.0), (v - 0.5).max(0.0));
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let corners = [
        surface_w[y0 * width + x0],
        surface_w[y0 * width + x1],
        surface_w[y1 * width + x0],
        surface_w[y1 * width + x1],
    ];
    let (low, high) = corners
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &w| {
            (lo.min(w), hi.max(w))
        });
    // NaN corners (no surface) leave `low`/`high` unmoved; count them out.
    if corners.iter().any(|w| w.is_nan()) || high - low > TRUNCATION {
        return nearest;
    }
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let top = corners[0] + (corners[1] - corners[0]) * fx;
    let bottom = corners[2] + (corners[3] - corners[2]) * fx;
    top + (bottom - top) * fy
}

impl Volume {
    /// Fold `frame` into the volume: every voxel in view within
    /// [`TRUNCATION`] behind the surface its ray hits moves its average
    /// toward its distance from that surface, and every voxel in front of it
    /// toward free space. Chunks whose surface may have moved are marked for
    /// [`Volume::extract_dirty`].
    pub fn integrate(&mut self, frame: &DepthFrame) {
        let (width, height) = (frame.width, frame.height);
        let Some(inverse) = frame.view_projection.invert() else {
            return;
        };
        if frame.depth.len() != width * height {
            return;
        }
        // The camera centre is the point clip space sends to infinity.
        let eye = unproject(&inverse, Vector4::new(0.0, 0.0, 1.0, 0.0));

        // Each pixel's surface depth along the view axis (clip `w`; NaN:
        // none), and the chunks the in-range surfaces' truncation bands reach.
        // Comparing view-axis depths rather than ray lengths keeps a voxel and
        // the pixel it lands in consistent across the pixel's footprint.
        let mut surface_w = vec![f32::NAN; width * height];
        let mut reached = HashSet::new();
        for y in 0..height {
            for x in 0..width {
                let d = frame.depth[y * width + x];
                if !(d < 1.0) {
                    continue;
                }
                let ndc = Vector4::new(
                    (x as f32 + 0.5) / width as f32 * 2.0 - 1.0,
                    (y as f32 + 0.5) / height as f32 * 2.0 - 1.0,
                    d * 2.0 - 1.0,
                    1.0,
                );
                let homogeneous = inverse * ndc;
                // `ndc` has w = 1, so the point's clip w is the inverse's.
                let w = 1.0 / homogeneous.w;
                if !(w > 0.0) {
                    continue;
                }
                surface_w[y * width + x] = w;
                if w <= MAX_RANGE && x % ALLOCATE_STRIDE == 0 && y % ALLOCATE_STRIDE == 0 {
                    let surface = homogeneous.truncate() * w;
                    let dir = (surface - eye).normalize();
                    for t in [-TRUNCATION, 0.0, TRUNCATION] {
                        reached.insert(chunk_of(surface + dir * t));
                    }
                }
            }
        }
        for key in reached {
            self.chunks.entry(key).or_insert_with(|| Chunk {
                sdf: [0.0; CHUNK_VOXELS],
                weight: [0.0; CHUNK_VOXELS],
                meshed: [0.0; CHUNK_VOXELS],
            });
        }

        let mut changed = Vec::new();
        for (&key, chunk) in self.chunks.iter_mut() {
            if !chunk_in_view(&frame.view_projection, eye, key) {
                continue;
            }
            // Whether the mesh no longer matches: a voxel first observed, or
            // one whose sign or value moved from what was meshed.
            let mut stale = false;
            for z in 0..CHUNK {
                for y in 0..CHUNK {
                    for x in 0..CHUNK {
                        let p = voxel_position(key, x, y, z);
                        let clip = frame.view_projection * p.extend(1.0);
                        if clip.w <= 0.0 {
                            continue;
                        }
                        let u = (clip.x / clip.w * 0.5 + 0.5) * width as f32;
                        let v = (clip.y / clip.w * 0.5 + 0.5) * height as f32;
                        if !(u >= 0.0 && v >= 0.0 && u < width as f32 && v < height as f32) {
                            continue;
                        }
                        let surface = sample(&surface_w, width, height, u, v);
                        let sdf = surface - clip.w;
                        // NaN (no surface) fails this too: unknown, untouched.
                        // A surface out of range only clears space in front.
                        if !(sdf >= -TRUNCATION) || (surface > MAX_RANGE && sdf < TRUNCATION) {
                            continue;
                        }
                        let i = voxel_index(x, y, z);
                        let (old, weight) = (chunk.sdf[i], chunk.weight[i]);
                        // An incremental mean: an unchanged observation keeps
                        // the value bit-exact, so a still view stays still.
                        let new = old + ((sdf / TRUNCATION).min(1.0) - old) / (weight + 1.0);
                        chunk.sdf[i] = new;
                        chunk.weight[i] = (weight + 1.0).min(MAX_WEIGHT);
                        let meshed = chunk.meshed[i];
                        stale |= weight == 0.0
                            || (meshed < 0.0) != (new < 0.0)
                            || (new - meshed).abs() > REMESH_DRIFT;
                    }
                }
            }
            if stale {
                chunk.meshed = chunk.sdf;
                changed.push(key);
            }
        }
        // A chunk's cells also read its +1 neighbours' first voxels.
        for [x, y, z] in changed {
            for corner in 0..8 {
                self.dirty
                    .insert([x - (corner & 1), y - ((corner >> 1) & 1), z - (corner >> 2)]);
            }
        }
        self.fused_frames += 1;
    }

    /// Re-mesh every chunk marked since the last call: `Some(mesh)` replaces
    /// that chunk's mesh, `None` removes it (no surface there now).
    pub fn extract_dirty(&mut self) -> Vec<(ChunkKey, Option<MeshData>)> {
        std::mem::take(&mut self.dirty)
            .into_iter()
            .map(|key| {
                let mesh = Some(self.extract_chunk(key)).filter(|m| !m.triangles.is_empty());
                (key, mesh)
            })
            .collect()
    }

    /// Marching cubes over chunk `key`'s 8³ cells. A cell with an unobserved
    /// corner (or one in a missing neighbour chunk) is skipped. Triangles wind
    /// counter-clockwise seen from free space; vertices on a shared edge are
    /// shared, and a neighbouring chunk computes the identical position.
    fn extract_chunk(&self, key: ChunkKey) -> MeshData {
        // The chunk's voxels plus its +1 neighbours' first layers: 9³.
        const N: i32 = CHUNK + 1;
        let mut grid = [f32::NAN; (N * N * N) as usize];
        let at = |x: i32, y: i32, z: i32| (x + N * (y + N * z)) as usize;
        for corner in 0..8 {
            let offset = [corner & 1, (corner >> 1) & 1, corner >> 2];
            let Some(chunk) =
                self.chunks
                    .get(&[key[0] + offset[0], key[1] + offset[1], key[2] + offset[2]])
            else {
                continue;
            };
            let range = |o: i32| if o == 0 { 0..CHUNK } else { 0..1 };
            for z in range(offset[2]) {
                for y in range(offset[1]) {
                    for x in range(offset[0]) {
                        let i = voxel_index(x, y, z);
                        // The snapshot, not the live value: a chunk that
                        // re-meshes reads its neighbours' boundary voxels as
                        // their own meshes did, so shared faces still meet.
                        if chunk.weight[i] > 0.0 {
                            grid[at(
                                x + offset[0] * CHUNK,
                                y + offset[1] * CHUNK,
                                z + offset[2] * CHUNK,
                            )] = chunk.meshed[i];
                        }
                    }
                }
            }
        }

        let table = cube_table();
        let mut mesh = MeshData {
            positions: Vec::new(),
            triangles: Vec::new(),
        };
        // The vertex on each grid edge, by (lower voxel, axis).
        let mut edge_vertex = vec![u32::MAX; grid.len() * 3];
        for z in 0..CHUNK {
            for y in 0..CHUNK {
                for x in 0..CHUNK {
                    let mut values = [0.0; 8];
                    let mut config = 0;
                    for (corner, value) in values.iter_mut().enumerate() {
                        let c = corner as i32;
                        *value = grid[at(x + (c & 1), y + ((c >> 1) & 1), z + (c >> 2))];
                        if *value < 0.0 {
                            config |= 1 << corner;
                        }
                    }
                    if values.iter().any(|v| v.is_nan()) {
                        continue;
                    }
                    for triangle in &table[config] {
                        let indices = triangle.map(|edge| {
                            let (a, b) = EDGES[edge as usize];
                            let axis = (a ^ b).trailing_zeros() as usize;
                            let corner = a as i32;
                            let (lx, ly, lz) =
                                (x + (corner & 1), y + (corner >> 1 & 1), z + (corner >> 2));
                            let slot = &mut edge_vertex[at(lx, ly, lz) * 3 + axis];
                            if *slot == u32::MAX {
                                let (va, vb) = (values[a], values[b]);
                                let mut p = voxel_position(key, lx, ly, lz);
                                p[axis] += va / (va - vb) * VOXEL;
                                *slot = mesh.positions.len() as u32;
                                mesh.positions.push(p.into());
                            }
                            *slot
                        });
                        mesh.triangles.push(indices);
                    }
                }
            }
        }
        mesh
    }
}

/// A cube cell's corner `i` sits at `(i & 1, i >> 1 & 1, i >> 2 & 1)`; its 12
/// edges, as corner pairs differing in one bit.
const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Triangles (as edge triples) for each of the 256 inside/outside corner
/// configurations (bit `i` set = corner `i` inside), generated once instead
/// of transcribing the classic table.
///
/// Each cube face pairs its sign-changing edges into segments that cut off
/// the inside corners, so a face shared by two cells is cut the same way from
/// both sides and the surface is watertight across cells. The segments close
/// into loops, each wound to face the outside and fanned into triangles.
fn cube_table() -> &'static [Vec<[u8; 3]>; 256] {
    static TABLE: OnceLock<[Vec<[u8; 3]>; 256]> = OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(cube_triangles))
}

fn cube_triangles(config: usize) -> Vec<[u8; 3]> {
    let inside = |corner: usize| config & (1 << corner) != 0;
    let edge_of = |a: usize, b: usize| {
        EDGES
            .iter()
            .position(|&e| e == (a.min(b), a.max(b)))
            .expect("adjacent corners")
    };
    // Each crossed edge's two neighbours along the surface, one per face.
    let mut links: [Vec<usize>; 12] = Default::default();
    for axis in 0..3 {
        for side in 0..2 {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let corners =
                [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(i, j)| side << axis | i << u | j << v);
            // Sign changes around the face, in order; they alternate between
            // entering and leaving the inside.
            let crossings: Vec<(usize, bool)> = (0..4)
                .filter(|&k| inside(corners[k]) != inside(corners[(k + 1) % 4]))
                .map(|k| {
                    (
                        edge_of(corners[k], corners[(k + 1) % 4]),
                        inside(corners[k]),
                    )
                })
                .collect();
            // Pair each entry into an inside run with the exit after it.
            let first_entry = crossings.iter().position(|&(_, from_inside)| !from_inside);
            if let Some(start) = first_entry {
                for pair in (0..crossings.len()).step_by(2) {
                    let a = crossings[(start + pair) % crossings.len()].0;
                    let b = crossings[(start + pair + 1) % crossings.len()].0;
                    links[a].push(b);
                    links[b].push(a);
                }
            }
        }
    }

    let midpoint = |edge: usize| {
        let (a, b) = EDGES[edge];
        let p = |c: usize| Vector3::new((c & 1) as f32, (c >> 1 & 1) as f32, (c >> 2 & 1) as f32);
        (p(a) + p(b)) * 0.5
    };
    // The gradient of the trilinear interpolant of ±1 corner values: it
    // points from inside to outside.
    let gradient = |q: Vector3<f32>| {
        (0..8).fold(Vector3::new(0.0, 0.0, 0.0), |g, c| {
            let s = if inside(c) { -1.0 } else { 1.0 };
            let (fx, fy, fz) = (
                if c & 1 != 0 { q.x } else { 1.0 - q.x },
                if c & 2 != 0 { q.y } else { 1.0 - q.y },
                if c & 4 != 0 { q.z } else { 1.0 - q.z },
            );
            let sign = |bit: usize| if c & bit != 0 { 1.0 } else { -1.0 };
            g + Vector3::new(sign(1) * fy * fz, sign(2) * fx * fz, sign(4) * fx * fy) * s
        })
    };

    let mut triangles = Vec::new();
    let mut visited = [false; 12];
    for start in 0..12 {
        if visited[start] || links[start].is_empty() {
            continue;
        }
        let mut cycle = vec![start];
        visited[start] = true;
        let (mut previous, mut current) = (start, links[start][0]);
        while current != start {
            visited[current] = true;
            cycle.push(current);
            let next = if links[current][0] == previous {
                links[current][1]
            } else {
                links[current][0]
            };
            (previous, current) = (current, next);
        }
        // Newell's normal of the loop against the field's gradient.
        let points: Vec<_> = cycle.iter().map(|&e| midpoint(e)).collect();
        let normal = (0..points.len()).fold(Vector3::new(0.0, 0.0, 0.0), |n, i| {
            n + points[i].cross(points[(i + 1) % points.len()])
        });
        let outward = points
            .iter()
            .fold(Vector3::new(0.0, 0.0, 0.0), |g, &p| g + gradient(p));
        if normal.dot(outward) < 0.0 {
            cycle.reverse();
        }
        for k in 1..cycle.len() - 1 {
            triangles.push([cycle[0], cycle[k], cycle[k + 1]].map(|e| e as u8));
        }
    }
    triangles
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use cgmath::{Deg, Point3};
    use std::collections::HashMap;

    /// A camera at `eye` looking at `target`, 90° square, 0.1–20 m.
    pub(crate) fn view_projection(eye: [f32; 3], target: [f32; 3]) -> Matrix4<f32> {
        cgmath::perspective(Deg(90.0), 1.0, 0.1, 20.0)
            * Matrix4::look_at_rh(Point3::from(eye), Point3::from(target), Vector3::unit_y())
    }

    /// Ray-cast a depth frame of `boxes` (min, max corners) from
    /// `view_projection`, as the depth sensor would see them.
    fn depth_frame(view_projection: Matrix4<f32>, boxes: &[([f32; 3], [f32; 3])]) -> DepthFrame {
        depth_frame_sized(view_projection, boxes, 96)
    }

    pub(crate) fn depth_frame_sized(
        view_projection: Matrix4<f32>,
        boxes: &[([f32; 3], [f32; 3])],
        size: usize,
    ) -> DepthFrame {
        let inverse = view_projection.invert().unwrap();
        let mut depth = vec![1.0; size * size];
        for y in 0..size {
            for x in 0..size {
                let ndc = |z: f32| {
                    Vector4::new(
                        (x as f32 + 0.5) / size as f32 * 2.0 - 1.0,
                        (y as f32 + 0.5) / size as f32 * 2.0 - 1.0,
                        z,
                        1.0,
                    )
                };
                let near = unproject(&inverse, ndc(-1.0));
                let dir = (unproject(&inverse, ndc(1.0)) - near).normalize();
                // Slab test per box; keep the nearest entry.
                let hit = boxes
                    .iter()
                    .filter_map(|(min, max)| {
                        let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
                        for a in 0..3 {
                            let (p, q) = ((min[a] - near[a]) / dir[a], (max[a] - near[a]) / dir[a]);
                            t0 = t0.max(p.min(q));
                            t1 = t1.min(p.max(q));
                        }
                        (t0 <= t1).then_some(t0)
                    })
                    .fold(f32::INFINITY, f32::min);
                if hit.is_finite() {
                    let clip = view_projection * (near + dir * hit).extend(1.0);
                    depth[y * size + x] = clip.z / clip.w * 0.5 + 0.5;
                }
            }
        }
        DepthFrame {
            width: size,
            height: size,
            depth,
            view_projection,
        }
    }

    fn all_vertices(
        volume: &mut Volume,
        meshes: &mut HashMap<ChunkKey, MeshData>,
    ) -> Vec<[f32; 3]> {
        for (key, mesh) in volume.extract_dirty() {
            match mesh {
                Some(mesh) => meshes.insert(key, mesh),
                None => meshes.remove(&key),
            };
        }
        meshes.values().flat_map(|m| m.positions.clone()).collect()
    }

    /// A closed, consistently wound surface.
    fn assert_watertight(mesh: &MeshData) {
        let solid = functor_solid::Mesh {
            vertices: mesh.positions.iter().map(|p| p.map(f64::from)).collect(),
            triangles: mesh.triangles.clone(),
        };
        assert_eq!(solid.non_manifold_edges(), 0);
    }

    /// A volume holding `sdf` (metres) at every voxel of chunk 0.
    fn volume_of(sdf: impl Fn(Vector3<f32>) -> f32) -> Volume {
        let mut chunk = Chunk {
            sdf: [0.0; CHUNK_VOXELS],
            weight: [1.0; CHUNK_VOXELS],
            meshed: [0.0; CHUNK_VOXELS],
        };
        for z in 0..CHUNK {
            for y in 0..CHUNK {
                for x in 0..CHUNK {
                    chunk.sdf[voxel_index(x, y, z)] = sdf(voxel_position([0; 3], x, y, z));
                }
            }
        }
        chunk.meshed = chunk.sdf;
        let mut volume = Volume::default();
        volume.chunks.insert([0; 3], chunk);
        volume
    }

    #[test]
    fn marching_cubes_closes_a_sphere_facing_out() {
        let center = Vector3::new(0.3, 0.29, 0.31);
        let volume = volume_of(|p| (p - center).magnitude() - 0.2);
        let mesh = volume.extract_chunk([0; 3]);
        assert!(mesh.triangles.len() > 50, "{}", mesh.triangles.len());
        assert_watertight(&mesh);
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|i| Vector3::from(mesh.positions[i as usize]));
            let normal = (b - a).cross(c - a);
            assert!(
                normal.dot((a + b + c) / 3.0 - center) > 0.0,
                "inward triangle"
            );
        }
        for p in &mesh.positions {
            let r = (Vector3::from(*p) - center).magnitude();
            assert!((r - 0.2).abs() < 0.02, "vertex at radius {r}");
        }
    }

    /// Noise exercises the ambiguous face configurations: as long as the
    /// chunk's boundary is outside, every surface must still close.
    #[test]
    fn marching_cubes_closes_noise() {
        let mut seed = 12345u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let values: Vec<f32> = (0..CHUNK_VOXELS).map(|_| noise()).collect();
        let volume = volume_of(|p| {
            let v = (p / VOXEL).map(|c| c.floor() as i32);
            if [v.x, v.y, v.z].iter().any(|&c| c == 0 || c == CHUNK - 1) {
                1.0
            } else {
                values[voxel_index(v.x, v.y, v.z)]
            }
        });
        let mesh = volume.extract_chunk([0; 3]);
        assert!(mesh.triangles.len() > 100);
        assert_watertight(&mesh);
    }

    #[test]
    fn a_wall_fuses_to_its_surface() {
        let vp = view_projection([0.0, 1.5, 0.0], [0.0, 1.5, 1.0]);
        let wall = ([-3.0, -1.0, 2.03], [3.0, 4.0, 2.2]);
        let mut volume = Volume::default();
        volume.integrate(&depth_frame(vp, &[wall]));
        let vertices = all_vertices(&mut volume, &mut HashMap::new());
        assert!(vertices.len() > 100, "{}", vertices.len());
        for p in vertices {
            assert!((p[2] - 2.03).abs() < 0.01, "vertex off the wall: {p:?}");
        }
        assert_eq!(volume.fused_frames, 1);
    }

    /// Seen at a grazing angle the floor's depth changes fast from pixel to
    /// pixel; sampling it between pixels keeps the fused floor flat.
    #[test]
    fn a_grazing_floor_fuses_flat() {
        let vp = cgmath::perspective(Deg(90.0), 4.0 / 3.0, 0.1, 100.0)
            * Matrix4::look_at_rh(
                Point3::new(0.0, 1.6, -0.6),
                Point3::new(0.0, 1.6, 0.4),
                Vector3::unit_y(),
            );
        let floor = ([-3.0, -0.2, -1.5], [3.0, 0.0, 4.5]);
        let mut volume = Volume::default();
        volume.integrate(&depth_frame_sized(vp, &[floor], 320));
        let vertices = all_vertices(&mut volume, &mut HashMap::new());
        assert!(vertices.len() > 1000, "{}", vertices.len());
        for p in vertices {
            assert!(p[1].abs() < 0.006, "vertex off the floor: {p:?}");
        }
    }

    #[test]
    fn a_still_view_re_meshes_nothing() {
        let vp = view_projection([0.0, 1.5, 0.0], [0.3, 1.0, 1.0]);
        let frame = depth_frame(vp, &[([-3.0, -1.0, 2.03], [3.0, 4.0, 2.2])]);
        let mut volume = Volume::default();
        volume.integrate(&frame);
        assert!(!volume.extract_dirty().is_empty());
        volume.integrate(&frame);
        assert!(volume.extract_dirty().is_empty());
    }

    #[test]
    fn a_removed_box_decays_away() {
        let vp = view_projection([0.0, 1.0, 0.0], [0.0, 0.6, 1.0]);
        // Free space is only carved in front of a surface: the floor is what
        // the box's lower half was hiding.
        let wall = ([-3.0, -1.0, 3.03], [3.0, 4.0, 3.2]);
        let floor = ([-3.0, -0.2, -1.0], [3.0, 0.01, 3.03]);
        let box_ = ([-0.25, 0.01, 1.33], [0.25, 0.5, 1.83]);
        let near_box = |vertices: &[[f32; 3]]| {
            vertices
                .iter()
                .filter(|p| p[1] > 0.05 && p[2] < 2.5)
                .count()
        };
        let mut volume = Volume::default();
        let mut meshes = HashMap::new();
        let with_box = depth_frame(vp, &[wall, floor, box_]);
        for _ in 0..40 {
            volume.integrate(&with_box);
        }
        let vertices = all_vertices(&mut volume, &mut meshes);
        assert!(
            near_box(&vertices) > 20,
            "the box fused: {}",
            near_box(&vertices)
        );
        // The box's front face sits where it was observed.
        assert!(vertices
            .iter()
            .any(|p| (p[2] - 1.33).abs() < 0.01 && p[0].abs() < 0.2));

        // Carved within 2 × MAX_WEIGHT frames of free space — about two
        // seconds at the shells' cadence; voxels just above the floor, where
        // the free space in front of a surface is thin, go last.
        let without = depth_frame(vp, &[wall, floor]);
        let frames = (1..=2 * MAX_WEIGHT as usize)
            .find(|_| {
                volume.integrate(&without);
                near_box(&all_vertices(&mut volume, &mut meshes)) == 0
            })
            .expect("the box should decay away");
        eprintln!("decayed in {frames} frames");
        assert!(frames > 1, "a single frame should not erase it: {frames}");
        // The wall it hid fills in.
        assert!(all_vertices(&mut volume, &mut meshes)
            .iter()
            .any(|p| (p[2] - 3.03).abs() < 0.01 && p[0].abs() < 0.2));
    }

    /// Depth beyond fusing range still carves: a box removed from in front of
    /// a far wall decays.
    #[test]
    fn a_far_wall_still_clears_a_removed_box() {
        let vp = view_projection([0.0, 1.0, 0.0], [0.0, 0.9, 1.0]);
        let far_wall = ([-9.0, -2.0, 6.03], [9.0, 6.0, 6.2]);
        let box_ = ([-0.25, 0.75, 1.83], [0.25, 1.25, 2.33]);
        let mut volume = Volume::default();
        let mut meshes = HashMap::new();
        for _ in 0..40 {
            volume.integrate(&depth_frame(vp, &[far_wall, box_]));
        }
        assert!(!all_vertices(&mut volume, &mut meshes).is_empty());
        let without = depth_frame(vp, &[far_wall]);
        for _ in 0..2 * MAX_WEIGHT as usize {
            volume.integrate(&without);
        }
        // Nothing fused at the box, nor at the out-of-range wall.
        assert_eq!(
            all_vertices(&mut volume, &mut meshes),
            Vec::<[f32; 3]>::new()
        );
    }

    #[test]
    fn depth_round_trips_through_rgba8() {
        let d: f32 = 0.987_654_3;
        let v = (d * 16_777_215.0).round() as u32;
        let rgba = [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255];
        let frame = DepthFrame::from_rgba8(1, 1, &rgba, Matrix4::identity());
        assert!((frame.depth[0] - d).abs() < 1e-7);
    }
}
