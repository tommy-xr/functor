//! The offline baker behind `functor bake`: linear illumination for each
//! texel of an unwrapped mesh. Lights use the engine's falloff
//! (`light::distance_falloff`, `cone_falloff`), shadow rays go through a parry
//! trimesh BVH (`castShadows` lights only, as at runtime), and ambient light
//! is darkened by occlusion. Each chart is then dilated into its padding.
//!
//! Deterministic: a texel's samples come from `(seed, texel, sample)` and its
//! value from no other texel, so the bytes don't depend on the thread count.

use std::sync::atomic::{AtomicUsize, Ordering};

use rapier3d::parry::math::Vector;
use rapier3d::parry::query::{Ray, RayCast};
use rapier3d::parry::shape::TriMesh;

use super::{unwrap, UnwrapOptions, Unwrapped};
use crate::light::{cone_falloff, distance_falloff, radiance, Light};
use crate::model::ModelSource;

/// Bumped when the same inputs would bake different texels.
pub const BAKE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct BakeOptions {
    pub unwrap: UnwrapOptions,
    /// Ambient-occlusion rays per texel; 0 leaves ambient light unoccluded.
    pub ao_samples: u32,
    /// How far an occlusion ray looks for geometry.
    pub ao_distance: f32,
    pub seed: u64,
    /// Worker threads (at least 1); the output doesn't depend on it.
    pub threads: usize,
}

/// `size`² texels of linear RGB light, row 0 first. `filled` marks texels a
/// chart sampled or dilation reached; the rest are background (black).
#[derive(Debug, Clone)]
pub struct Baked {
    pub size: u32,
    pub light: Vec<[f32; 3]>,
    pub filled: Vec<bool>,
}

impl Baked {
    /// RGBA8, clamped at 1; alpha is 255 on filled texels, 0 on background.
    pub fn rgba8(&self) -> Vec<u8> {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.light
            .iter()
            .zip(&self.filled)
            .flat_map(|(c, &f)| [byte(c[0]), byte(c[1]), byte(c[2]), if f { 255 } else { 0 }])
            .collect()
    }
}

/// Unwrap a validated static mesh and bake `lights` into it. Returns the
/// unwrapped mesh (`uvs2` filled, no lightmap) and its lightmap.
pub fn bake_mesh(
    source: &ModelSource,
    lights: &[Light],
    options: &BakeOptions,
) -> Result<(ModelSource, Baked), String> {
    let unwrapped = unwrap(&source.positions, &source.triangles, options.unwrap)?;
    let baked = bake(
        &source.positions,
        &source.normals,
        &source.triangles,
        &unwrapped,
        lights,
        options,
    )?;
    Ok((unwrapped_source(source, &unwrapped), baked))
}

/// `source` on `unwrapped`'s vertex set: per-vertex fields copied through
/// `source`, its `uvs2` and triangles, the same groups, no lightmap (one
/// baked for the old `uvs2` wouldn't fit).
pub fn unwrapped_source(source: &ModelSource, unwrapped: &Unwrapped) -> ModelSource {
    fn copy<T: Copy>(values: &[T], map: &[u32]) -> Vec<T> {
        if values.is_empty() {
            return Vec::new();
        }
        map.iter().map(|&v| values[v as usize]).collect()
    }
    ModelSource {
        positions: copy(&source.positions, &unwrapped.source),
        normals: copy(&source.normals, &unwrapped.source),
        uvs: copy(&source.uvs, &unwrapped.source),
        uvs2: unwrapped.uvs2.clone(),
        triangles: unwrapped.triangles.clone(),
        lightmap: None,
        ..source.clone()
    }
}

/// Where a texel samples the mesh: an input triangle and barycentric weights.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sample {
    pub texel: u32,
    pub triangle: u32,
    pub weights: [f32; 3],
}

/// Bake `lights` into `unwrapped`, the unwrap of `triangles` over
/// `positions` (with one normal per position).
pub fn bake(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    triangles: &[[u32; 3]],
    unwrapped: &Unwrapped,
    lights: &[Light],
    options: &BakeOptions,
) -> Result<Baked, String> {
    if !(options.ao_distance.is_finite() && options.ao_distance > 0.0) {
        return Err(format!(
            "the occlusion distance must be a finite number above 0, got {}",
            options.ao_distance
        ));
    }
    let scene = Scene::new(positions, normals, triangles, unwrapped, lights, options)?;
    let samples = rasterize(unwrapped);

    // Texels in chunks, claimed in turn by each worker; each lands at its
    // sample's index, so the merge is order-free.
    const CHUNK: usize = 256;
    let chunks = samples.len().div_ceil(CHUNK);
    let next = AtomicUsize::new(0);
    let mut light = vec![[0.0f32; 3]; samples.len()];
    let done: Vec<Vec<(usize, Vec<[f32; 3]>)>> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..options.threads.max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let chunk = next.fetch_add(1, Ordering::Relaxed);
                        if chunk >= chunks {
                            return out;
                        }
                        let range = chunk * CHUNK..((chunk + 1) * CHUNK).min(samples.len());
                        out.push((range.start, samples[range].iter().map(|s| scene.light_at(s)).collect()));
                    }
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().expect("a bake worker panicked")).collect()
    });
    for (start, values) in done.into_iter().flatten() {
        light[start..start + values.len()].copy_from_slice(&values);
    }

    let size = unwrapped.size;
    let mut baked = Baked {
        size,
        light: vec![[0.0; 3]; (size * size) as usize],
        filled: vec![false; (size * size) as usize],
    };
    for (s, value) in samples.iter().zip(light) {
        baked.light[s.texel as usize] = value;
        baked.filled[s.texel as usize] = true;
    }
    dilate(&mut baked, unwrapped, options.unwrap.padding);
    Ok(baked)
}

/// Each texel whose centre lies in one of its chart's triangles (the lowest
/// on a shared edge). A chart too thin to hold a texel centre samples its
/// first triangle's centroid instead, so it isn't black.
pub(crate) fn rasterize(u: &Unwrapped) -> Vec<Sample> {
    let size = u.size as usize;
    let mut taken = vec![false; size * size];
    let mut samples = Vec::new();
    let mut first = vec![u32::MAX; u.charts.len()];
    let mut sampled = vec![false; u.charts.len()];
    for (t, tri) in u.triangles.iter().enumerate() {
        let chart = u.chart_of[t] as usize;
        first[chart] = first[chart].min(t as u32);
        let rect = u.charts[chart];
        let p = tri.map(|v| u.uvs2[v as usize].map(|x| x as f64 * u.size as f64));
        let area = cross(p[0], p[1], p[2]);
        if area == 0.0 {
            continue;
        }
        // Texel centres (x + 0.5) inside the triangle's bounds and its chart.
        let span = |k: usize, lo: u32, len: u32| {
            let min = p.iter().map(|q| q[k]).fold(f64::INFINITY, f64::min);
            let max = p.iter().map(|q| q[k]).fold(f64::NEG_INFINITY, f64::max);
            let from = ((min - 0.5).ceil().max(lo as f64)) as u32;
            let to = ((max - 0.5).floor().min((lo + len) as f64 - 1.0)) as i64;
            from as i64..=to
        };
        for y in span(1, rect.y, rect.height) {
            for x in span(0, rect.x, rect.width) {
                let c = [x as f64 + 0.5, y as f64 + 0.5];
                let w = [cross(p[1], p[2], c), cross(p[2], p[0], c), cross(p[0], p[1], c)]
                    .map(|w| w / area);
                let texel = y as usize * size + x as usize;
                if w.iter().all(|&w| w >= -1e-9) && !taken[texel] {
                    taken[texel] = true;
                    samples.push(Sample {
                        texel: texel as u32,
                        triangle: t as u32,
                        weights: w.map(|w| w as f32),
                    });
                    sampled[chart] = true;
                }
            }
        }
    }
    for (chart, rect) in u.charts.iter().enumerate() {
        if sampled[chart] || first[chart] == u32::MAX {
            continue;
        }
        let t = first[chart];
        let centroid = |k: usize| {
            let sum: f64 = u.triangles[t as usize]
                .iter()
                .map(|&v| u.uvs2[v as usize][k] as f64 * u.size as f64)
                .sum();
            sum / 3.0
        };
        let clamp = |v: f64, lo: u32, len: u32| (v.floor() as i64).clamp(lo as i64, (lo + len - 1) as i64) as usize;
        let texel = clamp(centroid(1), rect.y, rect.height) * size + clamp(centroid(0), rect.x, rect.width);
        samples.push(Sample {
            texel: texel as u32,
            triangle: t,
            weights: [1.0 / 3.0; 3],
        });
    }
    samples
}

/// Twice the signed area of `a b c`.
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Fill each chart's empty texels, out to `padding` past its rect, with the
/// mean of their filled neighbours, a ring at a time, so filtering at chart
/// edges never reaches background. A ring reads only earlier rings.
fn dilate(baked: &mut Baked, u: &Unwrapped, padding: u32) {
    let size = baked.size as i64;
    for rect in &u.charts {
        let (x0, y0) = (rect.x as i64 - padding as i64, rect.y as i64 - padding as i64);
        let (x1, y1) = (
            (rect.x + rect.width + padding) as i64,
            (rect.y + rect.height + padding) as i64,
        );
        let neighbours = |i: usize| {
            let (x, y) = (i as i64 % size, i as i64 / size);
            NEIGHBOURS
                .iter()
                .map(move |(dx, dy)| (x + dx, y + dy))
                .filter(move |&(x, y)| x >= x0 && y >= y0 && x < x1 && y < y1)
                .map(move |(x, y)| (y * size + x) as usize)
        };
        let mut queued = vec![false; ((x1 - x0) * (y1 - y0)) as usize];
        let mut queue = |i: usize, queued: &mut [bool]| {
            let k = ((i as i64 / size - y0) * (x1 - x0) + i as i64 % size - x0) as usize;
            !std::mem::replace(&mut queued[k], true)
        };
        let mut ring: Vec<usize> = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * size + x) as usize;
                if !baked.filled[i] && neighbours(i).any(|j| baked.filled[j]) && queue(i, &mut queued) {
                    ring.push(i);
                }
            }
        }
        while !ring.is_empty() {
            let values: Vec<[f32; 3]> = ring
                .iter()
                .map(|&i| {
                    let filled: Vec<usize> = neighbours(i).filter(|&j| baked.filled[j]).collect();
                    let sum = filled.iter().fold([0.0f32; 3], |acc, &j| {
                        [0, 1, 2].map(|k| acc[k] + baked.light[j][k])
                    });
                    sum.map(|c| c / filled.len() as f32)
                })
                .collect();
            for (&i, value) in ring.iter().zip(values) {
                baked.light[i] = value;
                baked.filled[i] = true;
            }
            let mut next = Vec::new();
            for &i in &ring {
                for j in neighbours(i) {
                    if !baked.filled[j] && queue(j, &mut queued) {
                        next.push(j);
                    }
                }
            }
            ring = next;
        }
    }
}

const NEIGHBOURS: [(i64, i64); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Points per texel, on a jittered 2 × 2 grid: shadow edges and occlusion
/// average over the texel instead of stepping at its centre.
const SUBSAMPLES: usize = 4;

/// A point a texel samples: its position, shading normal, ray origin, and
/// face normal (on the shading normal's side), about which occlusion looks.
#[derive(Clone, Copy)]
struct Point {
    p: Vector,
    n: Vector,
    origin: Vector,
    face: Vector,
}

/// The mesh as rays see it, plus what shading a sample needs.
struct Scene<'a> {
    mesh: TriMesh,
    positions: &'a [[f32; 3]],
    normals: &'a [[f32; 3]],
    triangles: &'a [[u32; 3]],
    /// Each triangle's corners in atlas texels.
    texels: Vec<[[f64; 2]; 3]>,
    size: u32,
    lights: &'a [Light],
    /// How far ray origins sit off the surface, against self-hits.
    bias: f32,
    /// Longer than any path through the scene (directional shadow rays).
    far: f32,
    ao_samples: u32,
    ao_distance: f32,
    seed: u64,
}

fn v(p: [f32; 3]) -> Vector {
    Vector::new(p[0], p[1], p[2])
}

impl<'a> Scene<'a> {
    fn new(
        positions: &'a [[f32; 3]],
        normals: &'a [[f32; 3]],
        triangles: &'a [[u32; 3]],
        unwrapped: &Unwrapped,
        lights: &'a [Light],
        options: &BakeOptions,
    ) -> Result<Self, String> {
        // A parry trimesh BVH, as `Physics.trimesh` will build; one shared
        // builder can serve both once that lands.
        let mesh = TriMesh::new(positions.iter().map(|&p| v(p)).collect(), triangles.to_vec())
            .map_err(|e| format!("cannot build the ray mesh: {e:?}"))?;
        let (lo, hi) = positions.iter().fold(
            (Vector::splat(f32::INFINITY), Vector::splat(f32::NEG_INFINITY)),
            |(lo, hi), &p| (lo.min(v(p)), hi.max(v(p))),
        );
        let diagonal = (hi - lo).length();
        Ok(Scene {
            mesh,
            positions,
            normals,
            triangles,
            texels: unwrapped
                .triangles
                .iter()
                .map(|t| t.map(|v| unwrapped.uvs2[v as usize].map(|x| x as f64 * unwrapped.size as f64)))
                .collect(),
            size: unwrapped.size,
            lights,
            bias: (diagonal * 1e-4).max(1e-4),
            far: diagonal * 2.0 + 1.0,
            ao_samples: options.ao_samples,
            ao_distance: options.ao_distance,
            seed: options.seed,
        })
    }

    /// Whether anything lies within `max` of `origin` along unit `dir`.
    fn blocked(&self, origin: Vector, dir: Vector, max: f32) -> bool {
        max > 0.0 && self.mesh.intersects_local_ray(&Ray::new(origin, dir), max)
    }

    /// Sub-sample `j` of texel `s`, on its triangle (a point past an edge
    /// moves onto it).
    fn point(&self, s: &Sample, j: usize, h: u64) -> Point {
        let tri = self.triangles[s.triangle as usize];
        let [ta, tb, tc] = self.texels[s.triangle as usize];
        let (x, y) = ((s.texel % self.size) as f64, (s.texel / self.size) as f64);
        let at = [
            x + ((j & 1) as f64 + unit(hash(h, 2 * j as u64 + 2)) as f64) / 2.0,
            y + ((j >> 1) as f64 + unit(hash(h, 2 * j as u64 + 3)) as f64) / 2.0,
        ];
        let area = cross(ta, tb, tc);
        let weights = if area == 0.0 {
            s.weights
        } else {
            let w = [cross(tb, tc, at), cross(tc, ta, at), cross(ta, tb, at)].map(|w| (w / area).max(0.0));
            let sum: f64 = w.iter().sum();
            w.map(|w| (w / sum) as f32)
        };
        let mix = |values: &[[f32; 3]]| {
            (0..3).fold(Vector::ZERO, |acc, k| acc + v(values[tri[k] as usize]) * weights[k])
        };
        let p = mix(self.positions);
        let [a, b, c] = tri.map(|i| v(self.positions[i as usize]));
        let mut face = (b - a).cross(c - a).normalize_or_zero();
        let mut n = mix(self.normals).normalize_or_zero();
        if n == Vector::ZERO {
            n = face;
        }
        // The authored normal picks the lit side.
        if face.dot(n) < 0.0 {
            face = -face;
        }
        if face == Vector::ZERO {
            face = n;
        }
        Point { p, n, origin: p + face * self.bias, face }
    }

    /// Linear light arriving at a texel, averaged over its sub-samples: each
    /// light's `radiance × N·L × falloff`, shadowed if it casts shadows, plus
    /// ambient × the unoccluded share.
    fn light_at(&self, s: &Sample) -> [f32; 3] {
        let h = hash(self.seed, s.texel as u64);
        let points: [_; SUBSAMPLES] = std::array::from_fn(|j| self.point(s, j, h));
        let mut total = Vector::ZERO;
        let mut occlusion = None;
        for light in self.lights {
            if let Light::Ambient { color, .. } = *light {
                let open = *occlusion.get_or_insert_with(|| self.unoccluded(&points, h));
                total += v(radiance(color, 1.0)) * open;
                continue;
            }
            for &Point { p, n, origin, .. } in &points {
                let (color, l, atten, max) = match *light {
                    Light::Ambient { .. } => unreachable!("handled above"),
                    Light::Directional { direction, color, intensity, .. } => {
                        (radiance(color, intensity), -v(direction).normalize_or_zero(), 1.0, self.far)
                    }
                    Light::Point { position, color, intensity, range, .. } => {
                        let to = v(position) - p;
                        let d = to.length();
                        (radiance(color, intensity), to / d.max(1e-4), distance_falloff(d, range), d - self.bias)
                    }
                    Light::Spot { position, direction, color, intensity, range, cone_angle, .. } => {
                        let to = v(position) - p;
                        let d = to.length();
                        let l = to / d.max(1e-4);
                        let cone = cone_falloff((-l).dot(v(direction).normalize_or_zero()), cone_angle.cos());
                        (radiance(color, intensity), l, distance_falloff(d, range) * cone, d - self.bias)
                    }
                };
                let ndotl = n.dot(l).max(0.0);
                if ndotl * atten <= 0.0 || (light.casts_shadows() && self.blocked(origin, l, max)) {
                    continue;
                }
                total += v(color) * (ndotl * atten / SUBSAMPLES as f32);
            }
        }
        total.to_array()
    }

    /// The share of `ao_samples` cosine-weighted rays, spread over the
    /// sub-samples, that travel `ao_distance` unblocked: 1 in the open, less
    /// in corners. Directions are a Hammersley set rotated per texel.
    fn unoccluded(&self, points: &[Point; SUBSAMPLES], h: u64) -> f32 {
        let n = self.ao_samples;
        if n == 0 {
            return 1.0;
        }
        let (r1, r2) = (unit(h), unit(hash(h, 1)));
        let open = (0..n)
            .filter(|&k| {
                let Point { origin, face, .. } = points[k as usize % SUBSAMPLES];
                let (t, b) = basis(face);
                let u1 = ((k as f32 + 0.5) / n as f32 + r1).fract();
                let u2 = (radical_inverse(k) + r2).fract();
                let (r, phi) = (u1.sqrt(), u2 * std::f32::consts::TAU);
                let dir = t * (r * phi.cos()) + b * (r * phi.sin()) + face * (1.0 - u1).max(0.0).sqrt();
                !self.blocked(origin, dir.normalize_or_zero(), self.ao_distance)
            })
            .count();
        open as f32 / n as f32
    }
}

/// Two unit vectors completing an orthonormal basis with unit `n`
/// (Duff et al. 2017).
fn basis(n: Vector) -> (Vector, Vector) {
    let sign = 1.0f32.copysign(n.z);
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    (
        Vector::new(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x),
        Vector::new(b, sign + n.y * n.y * a, -n.y),
    )
}

/// splitmix64 of two words.
fn hash(a: u64, b: u64) -> u64 {
    let mut z = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x6A09_E667_F3BC_C909);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The top 24 bits as a float in [0, 1).
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// `k`'s bits mirrored about the binary point (van der Corput, base 2).
fn radical_inverse(k: u32) -> f32 {
    (k.reverse_bits() >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Positions, normals and triangles: one quad per face, its own corners.
    #[derive(Default)]
    struct Mesh {
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    }

    impl Mesh {
        /// A quad from corner `o` along `a` then `b` (counter-clockwise seen
        /// from the side `a × b` faces).
        fn quad(&mut self, o: [f32; 3], a: [f32; 3], b: [f32; 3]) {
            let (o, a, b) = (v(o), v(a), v(b));
            let base = self.positions.len() as u32;
            for p in [o, o + a, o + a + b, o + b] {
                self.positions.push(p.to_array());
                self.normals.push(a.cross(b).normalize().to_array());
            }
            self.triangles.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        }

        /// An axis-aligned box from `lo` to `hi`, faces out, no bottom.
        fn block(&mut self, lo: [f32; 3], hi: [f32; 3]) {
            let [w, h, d] = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
            let [x0, y0, z0] = lo;
            let [x1, y1, z1] = hi;
            self.quad([x0, y1, z0], [0.0, 0.0, d], [w, 0.0, 0.0]);
            self.quad([x0, y0, z0], [0.0, 0.0, d], [0.0, h, 0.0]);
            self.quad([x1, y0, z0], [0.0, h, 0.0], [0.0, 0.0, d]);
            self.quad([x0, y0, z0], [0.0, h, 0.0], [w, 0.0, 0.0]);
            self.quad([x0, y0, z1], [w, 0.0, 0.0], [0.0, h, 0.0]);
        }
    }

    /// A 4 × 3 × 4 m Cornell-style room, open at z = 0, faces inward, with a
    /// 1 × 1.2 × 1 m block right of centre.
    fn cornell() -> Mesh {
        let mut m = Mesh::default();
        m.quad([-2.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 0.0, -4.0]); // floor
        m.quad([-2.0, 3.0, -4.0], [4.0, 0.0, 0.0], [0.0, 0.0, 4.0]); // ceiling
        m.quad([-2.0, 0.0, -4.0], [4.0, 0.0, 0.0], [0.0, 3.0, 0.0]); // back
        m.quad([-2.0, 0.0, 0.0], [0.0, 0.0, -4.0], [0.0, 3.0, 0.0]); // left
        m.quad([2.0, 0.0, -4.0], [0.0, 0.0, 4.0], [0.0, 3.0, 0.0]); // right
        m.block([0.2, 0.0, -2.6], [1.2, 1.2, -1.6]);
        m
    }

    const LAMP: [f32; 3] = [0.0, 2.8, -2.0];

    fn lamp() -> Light {
        Light::point(LAMP[0], LAMP[1], LAMP[2], 1.0, 1.0, 1.0, 1.0, 6.0)
    }

    fn options(ao_samples: u32) -> BakeOptions {
        BakeOptions {
            unwrap: UnwrapOptions { texels_per_meter: 16.0, padding: 2, max_size: 1024 },
            ao_samples,
            ao_distance: 1.0,
            seed: 7,
            threads: 4,
        }
    }

    struct Bake {
        mesh: Mesh,
        unwrapped: Unwrapped,
        baked: Baked,
    }

    fn run(mesh: Mesh, lights: &[Light], options: BakeOptions) -> Bake {
        let unwrapped = unwrap(&mesh.positions, &mesh.triangles, options.unwrap).unwrap();
        let baked = bake(&mesh.positions, &mesh.normals, &mesh.triangles, &unwrapped, lights, &options).unwrap();
        Bake { mesh, unwrapped, baked }
    }

    impl Bake {
        /// The sampled texel nearest `at` on a surface facing `normal`: its
        /// light and where it sampled.
        fn near(&self, at: [f32; 3], normal: [f32; 3]) -> ([f32; 3], Vector) {
            let m = &self.mesh;
            rasterize(&self.unwrapped)
                .iter()
                .filter(|s| v(m.normals[m.triangles[s.triangle as usize][0] as usize]).dot(v(normal)) > 0.99)
                .map(|s| {
                    let tri = m.triangles[s.triangle as usize];
                    let p = (0..3).fold(Vector::ZERO, |acc, k| acc + v(m.positions[tri[k] as usize]) * s.weights[k]);
                    (self.baked.light[s.texel as usize], p)
                })
                .min_by(|a, b| a.1.distance(v(at)).total_cmp(&b.1.distance(v(at))))
                .unwrap()
        }
    }

    const UP: [f32; 3] = [0.0, 1.0, 0.0];

    /// Light reaching the floor is `radiance × cos θ × (1 − d²/r²)²`, the
    /// shader's own falloff, at every distance sampled.
    #[test]
    fn floor_light_follows_the_engine_falloff() {
        let b = run(cornell(), &[lamp()], options(0));
        for x in [-1.8f32, -1.2, -0.6, 0.0] {
            let (light, p) = b.near([x, 0.0, -2.0], UP);
            let to = v(LAMP) - p;
            let d = to.length();
            let shader = (1.0 - d * d / 36.0).clamp(0.0, 1.0).powi(2);
            assert_eq!(distance_falloff(d, 6.0), shader);
            let expected = to.y / d * shader;
            assert!((light[0] - expected).abs() <= 0.02 * expected, "at {p}: {light:?} vs {expected}");
            assert_eq!(light[0], light[2]);
        }
    }

    /// A floor texel in the block's shadow gets under 0.3× the light of its
    /// mirror image across the lamp, which nothing blocks.
    #[test]
    fn the_block_casts_a_shadow() {
        let b = run(cornell(), &[lamp().cast_shadows()], options(0));
        let (shadowed, _) = b.near([1.6, 0.0, -2.1], UP);
        let (lit, _) = b.near([-1.6, 0.0, -2.1], UP);
        assert!(lit[0] > 0.1, "{lit:?}");
        assert!(shadowed[0] < 0.3 * lit[0], "{shadowed:?} vs {lit:?}");
        // Without `castShadows`, as at runtime, no shadow.
        let b = run(cornell(), &[lamp()], options(0));
        let (unshadowed, _) = b.near([1.6, 0.0, -2.1], UP);
        assert!((unshadowed[0] - lit[0]).abs() < 0.02 * lit[0], "{unshadowed:?} vs {lit:?}");
    }

    /// Ambient light is darker in a corner than mid-wall.
    #[test]
    fn occlusion_darkens_corners() {
        let ambient = Light::ambient(1.0, 1.0, 1.0);
        let b = run(cornell(), &[ambient.clone()], options(64));
        let (corner, _) = b.near([-1.95, 0.0, -3.95], UP);
        let (wall, _) = b.near([-0.5, 1.5, -4.0], [0.0, 0.0, 1.0]);
        assert!(wall[0] > 0.9, "{wall:?}");
        assert!(corner[0] < 0.8 * wall[0], "{corner:?} vs {wall:?}");
        // No samples: ambient as is.
        let b = run(cornell(), &[ambient], options(0));
        assert_eq!(b.near([-1.95, 0.0, -3.95], UP).0, [1.0; 3]);
    }

    /// Same seed, same bytes at any thread count; another seed differs.
    #[test]
    fn bakes_are_deterministic() {
        let lights = [Light::ambient(0.3, 0.3, 0.3), lamp().cast_shadows()];
        let bytes = |threads, seed| {
            let mut o = options(16);
            (o.threads, o.seed) = (threads, seed);
            run(cornell(), &lights, o).baked.rgba8()
        };
        let one = bytes(1, 7);
        assert_eq!(one, bytes(8, 7));
        assert_ne!(one, bytes(8, 8));
    }

    /// Every texel of every chart's padded rect is filled; nothing else is.
    #[test]
    fn dilation_fills_the_padding() {
        let o = options(0);
        let b = run(cornell(), &[Light::ambient(1.0, 1.0, 1.0)], o);
        let (size, pad) = (b.baked.size, o.unwrap.padding);
        let mut inside = vec![false; (size * size) as usize];
        for r in &b.unwrapped.charts {
            for y in r.y - pad..r.y + r.height + pad {
                for x in r.x - pad..r.x + r.width + pad {
                    inside[(y * size + x) as usize] = true;
                }
            }
        }
        assert_eq!(b.baked.filled, inside);
        let rgba = b.baked.rgba8();
        for (i, &inside) in inside.iter().enumerate() {
            let expected = if inside { [255; 4] } else { [0; 4] };
            assert_eq!(rgba[i * 4..i * 4 + 4], expected, "texel {i}");
        }
    }

    /// A chart thinner than a texel centre still samples its triangle.
    #[test]
    fn a_sliver_chart_is_not_black() {
        let mut m = Mesh::default();
        m.quad([-2.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 0.0, -4.0]);
        // A 1 cm sliver standing on the floor, a chart of its own.
        m.quad([0.0, 0.0, -1.0], [0.0, 0.0, -1.0], [0.0, 0.01, 0.0]);
        let b = run(m, &[Light::ambient(1.0, 1.0, 1.0)], options(0));
        assert_eq!(b.unwrapped.charts.len(), 2);
        for r in &b.unwrapped.charts {
            assert!(b.baked.filled[(r.y * b.baked.size + r.x) as usize], "{r:?}");
        }
    }

    /// medsci1's scale (~26k triangles) at 8 texels a metre. Timing only:
    /// `cargo test --release … -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bake_a_level() {
        let (positions, triangles) = super::super::unwrap::tests::level(72);
        let normals = triangles.iter().fold(vec![[0.0; 3]; positions.len()], |mut n, t| {
            let [a, b, c] = t.map(|i| v(positions[i as usize]));
            t.iter().for_each(|&i| n[i as usize] = (b - a).cross(c - a).normalize().to_array());
            n
        });
        let lights: Vec<Light> = (0..72)
            .map(|r| Light::point(r as f32 * 7.0 + 3.0, 2.6, 3.0, 1.0, 0.9, 0.8, 1.0, 6.0).cast_shadows())
            .chain([Light::ambient(0.2, 0.2, 0.25)])
            .collect();
        let mut o = options(16);
        o.unwrap = UnwrapOptions { texels_per_meter: 8.0, padding: 1, max_size: 2048 };
        o.threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let start = std::time::Instant::now();
        let u = unwrap(&positions, &triangles, o.unwrap).unwrap();
        let unwrapped = start.elapsed();
        let baked = bake(&positions, &normals, &triangles, &u, &lights, &o).unwrap();
        eprintln!(
            "{} triangles, {} charts, {}² atlas, {} texels sampled, {} lights + 16 AO rays, {} threads: unwrap {:?}, total {:?}",
            triangles.len(),
            u.charts.len(),
            u.size,
            rasterize(&u).len(),
            lights.len(),
            o.threads,
            unwrapped,
            start.elapsed()
        );
        assert!(baked.filled.iter().any(|&f| f));
    }
}
