//! The offline baker behind `functor bake`: linear illumination for each
//! texel of an unwrapped mesh. Lights use the engine's falloff
//! (`light::distance_falloff`, `cone_falloff`), shadow rays go through a parry
//! trimesh BVH (`castShadows` lights only, as at runtime), and ambient light
//! is darkened by occlusion. Bounces then gather light reflected off the
//! lit surfaces, and are denoised within each chart. Each chart is finally
//! dilated into its padding.
//!
//! Deterministic: a texel's samples come from `(seed, texel, sample)`, each
//! pass reads only the previous pass's finished lightmap, and nearest hits
//! break ties by triangle index, so the bytes don't depend on the thread count.

use std::sync::atomic::{AtomicUsize, Ordering};

use rapier3d::parry::math::Vector;
use rapier3d::parry::partitioning::TraversalAction;
use rapier3d::parry::query::details::local_ray_intersection_with_triangle;
use rapier3d::parry::query::{Ray, RayCast};
use rapier3d::parry::shape::TriMesh;

use super::{unwrap, UnwrapOptions, Unwrapped};
use crate::color_space::{linear_to_srgb, srgb_to_linear3};
use crate::light::{cone_falloff, distance_falloff, radiance, Light};
use crate::model::ModelSource;

/// Bumped when the same inputs would bake different texels.
pub const BAKE_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub struct BakeOptions {
    pub unwrap: UnwrapOptions,
    /// Ambient-occlusion rays per texel; 0 leaves ambient light unoccluded.
    pub ao_samples: u32,
    /// How far an occlusion ray looks for geometry.
    pub ao_distance: f32,
    /// Times light reflects off lit surfaces before it is stored; 0 is
    /// direct light only.
    pub bounces: u32,
    /// Rays per texel per bounce.
    pub gi_samples: u32,
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
    /// RGBA8 of `light / scale`, clamped at 1 and sRGB-encoded, so the darks
    /// keep their steps (linear 0.1 is code 89, not 26); the game decodes it
    /// with `Texture.Srgb` and multiplies `scale` back. Alpha is 255 on
    /// filled texels, 0 on background.
    pub fn rgba8(&self, scale: f32) -> Vec<u8> {
        let byte = |v: f32| (linear_to_srgb((v / scale).clamp(0.0, 1.0)) * 255.0).round() as u8;
        self.light
            .iter()
            .zip(&self.filled)
            .flat_map(|(c, &f)| [byte(c[0]), byte(c[1]), byte(c[2]), if f { 255 } else { 0 }])
            .collect()
    }
}

/// Unwrap a validated static mesh and bake `lights` into it. `albedo` is
/// an authored (sRGB) colour per group (one if it has none): the share of
/// light each group reflects. Returns the unwrapped mesh (`uvs2` filled, no
/// lightmap) and its lightmap.
pub fn bake_mesh(
    source: &ModelSource,
    lights: &[Light],
    albedo: &[[f32; 3]],
    options: &BakeOptions,
) -> Result<(ModelSource, Baked), String> {
    let groups = source.groups.len().max(1);
    if albedo.len() != groups {
        return Err(format!(
            "albedo: one colour per group: expected {groups}, got {}",
            albedo.len()
        ));
    }
    // Per triangle, linear.
    let mut reflect = vec![srgb_to_linear3(albedo[0]); source.triangles.len()];
    for (group, &color) in source.groups.iter().zip(albedo) {
        let range = group.first as usize..(group.first + group.count) as usize;
        reflect[range].fill(srgb_to_linear3(color));
    }
    let unwrapped = unwrap(&source.positions, &source.triangles, options.unwrap)?;
    let baked = bake(
        &source.positions,
        &source.normals,
        &source.triangles,
        &unwrapped,
        lights,
        &reflect,
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
/// `positions` (with one normal per position). `albedo` is linear, one per
/// triangle.
pub fn bake(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    triangles: &[[u32; 3]],
    unwrapped: &Unwrapped,
    lights: &[Light],
    albedo: &[[f32; 3]],
    options: &BakeOptions,
) -> Result<Baked, String> {
    if !(options.ao_distance.is_finite() && options.ao_distance > 0.0) {
        return Err(format!(
            "the occlusion distance must be a finite number above 0, got {}",
            options.ao_distance
        ));
    }
    let scene = Scene::new(
        positions, normals, triangles, unwrapped, lights, albedo, options,
    )?;
    let samples = rasterize(unwrapped);
    let threads = options.threads;
    let padding = options.unwrap.padding;
    let direct = par_map(&samples, threads, |s| scene.light_at(s));
    let mut light: Vec<[f32; 3]> = direct.iter().map(|(l, a)| add(*l, *a)).collect();

    // Each bounce gathers the last one's light (first, the direct lights';
    // ambient already stands in for bounced light) off the surfaces it hits.
    if options.bounces > 0 && options.gi_samples > 0 {
        let mut last: Vec<[f32; 3]> = direct.iter().map(|(l, _)| *l).collect();
        let mut indirect = vec![[0.0f32; 3]; samples.len()];
        for bounce in 0..options.bounces {
            // Nothing left to bounce (e.g. ambient light alone).
            if last.iter().all(|c| *c == [0.0; 3]) {
                break;
            }
            let lit = lightmap(unwrapped, &samples, &last, padding);
            last = par_map(&samples, threads, |s| scene.gather(s, &lit, bounce));
            indirect.iter_mut().zip(&last).for_each(|(i, l)| *i = add(*i, *l));
        }
        let size = unwrapped.size as usize;
        let mut chart = vec![u32::MAX; size * size];
        let mut grid = vec![[0.0f32; 3]; size * size];
        for (s, value) in samples.iter().zip(&indirect) {
            chart[s.texel as usize] = unwrapped.chart_of[s.triangle as usize];
            grid[s.texel as usize] = *value;
        }
        let smooth = denoise(unwrapped.size, &chart, &grid, threads);
        for (s, l) in samples.iter().zip(&mut light) {
            *l = add(*l, smooth[s.texel as usize]);
        }
    }
    Ok(lightmap(unwrapped, &samples, &light, padding))
}

/// One value per sample, placed at its texel and dilated into each chart's
/// padding.
fn lightmap(u: &Unwrapped, samples: &[Sample], values: &[[f32; 3]], padding: u32) -> Baked {
    let size = u.size;
    let mut baked = Baked {
        size,
        light: vec![[0.0; 3]; (size * size) as usize],
        filled: vec![false; (size * size) as usize],
    };
    for (s, value) in samples.iter().zip(values) {
        baked.light[s.texel as usize] = *value;
        baked.filled[s.texel as usize] = true;
    }
    dilate(&mut baked, u, padding);
    baked
}

/// `f` of each item, on `threads` workers that claim chunks in turn; each
/// result lands at its item's index, so the output is order-free.
fn par_map<T: Sync, R: Send + Copy + Default>(
    items: &[T],
    threads: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    const CHUNK: usize = 256;
    let chunks = items.len().div_ceil(CHUNK);
    let next = AtomicUsize::new(0);
    let done: Vec<Vec<(usize, Vec<R>)>> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let chunk = next.fetch_add(1, Ordering::Relaxed);
                        if chunk >= chunks {
                            return out;
                        }
                        let range = chunk * CHUNK..((chunk + 1) * CHUNK).min(items.len());
                        out.push((range.start, items[range].iter().map(&f).collect()));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().expect("a bake worker panicked"))
            .collect()
    });
    let mut results = vec![R::default(); items.len()];
    for (start, values) in done.into_iter().flatten() {
        results[start..start + values.len()].copy_from_slice(&values);
    }
    results
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Smooths Monte Carlo noise in bounced light: an à-trous filter (5 × 5
/// B3-spline taps at strides 1, 2 and 4) that only reads texels of the same
/// chart (`chart`, `u32::MAX` where none) and weighs each tap down by its
/// relative difference in brightness, so seams, corners and real gradients
/// stay. Each pass reads only the last.
pub(crate) fn denoise(
    size: u32,
    chart: &[u32],
    light: &[[f32; 3]],
    threads: usize,
) -> Vec<[f32; 3]> {
    const KERNEL: [f32; 5] = [1.0 / 16.0, 1.0 / 4.0, 3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0];
    // A tap 50% brighter or darker keeps e^-1 of its weight.
    const SIGMA: f32 = 0.5;
    let size = size as i64;
    let texels: Vec<usize> = (0..chart.len()).filter(|&i| chart[i] != u32::MAX).collect();
    let mut out = light.to_vec();
    for stride in [1i64, 2, 4] {
        let prev = out.clone();
        let filtered = par_map(&texels, threads, |&i| {
            let (x, y) = (i as i64 % size, i as i64 / size);
            let l = luminance(prev[i]);
            let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
            for (dy, ky) in KERNEL.iter().enumerate() {
                for (dx, kx) in KERNEL.iter().enumerate() {
                    let (qx, qy) = (x + (dx as i64 - 2) * stride, y + (dy as i64 - 2) * stride);
                    if qx < 0 || qy < 0 || qx >= size || qy >= size {
                        continue;
                    }
                    let q = (qy * size + qx) as usize;
                    if chart[q] != chart[i] {
                        continue;
                    }
                    let lq = luminance(prev[q]);
                    let relative = (l - lq).abs() / (SIGMA * 0.5 * (l + lq) + 1e-4);
                    let w = kx * ky * (-relative * relative).exp();
                    sum = add(sum, prev[q].map(|c| c * w));
                    total += w;
                }
            }
            sum.map(|c| c / total)
        });
        for (&i, value) in texels.iter().zip(filtered) {
            out[i] = value;
        }
    }
    out
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
        let p = texels(u, tri);
        if cross(p[0], p[1], p[2]) == 0.0 {
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
                let w = barycentric(p, [x as f64 + 0.5, y as f64 + 0.5]);
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
        let clamp = |v: f64, lo: u32, len: u32| {
            (v.floor() as i64).clamp(lo as i64, (lo + len - 1) as i64) as usize
        };
        let texel =
            clamp(centroid(1), rect.y, rect.height) * size + clamp(centroid(0), rect.x, rect.width);
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

/// A triangle's corners in atlas texels.
fn texels(u: &Unwrapped, tri: &[u32; 3]) -> [[f64; 2]; 3] {
    tri.map(|v| u.uvs2[v as usize].map(|x| x as f64 * u.size as f64))
}

/// The barycentric weights of `at` in a triangle with nonzero area.
fn barycentric([a, b, c]: [[f64; 2]; 3], at: [f64; 2]) -> [f64; 3] {
    let area = cross(a, b, c);
    [cross(b, c, at), cross(c, a, at), cross(a, b, at)].map(|w| w / area)
}

/// Fill each chart's empty texels, out to `padding` past its rect, with the
/// mean of their filled neighbours, a ring at a time, so filtering at chart
/// edges never reaches background. A ring reads only earlier rings.
fn dilate(baked: &mut Baked, u: &Unwrapped, padding: u32) {
    let size = baked.size as i64;
    for rect in &u.charts {
        let (x0, y0) = (
            rect.x as i64 - padding as i64,
            rect.y as i64 - padding as i64,
        );
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
        let queue = |i: usize, queued: &mut [bool]| {
            let k = ((i as i64 / size - y0) * (x1 - x0) + i as i64 % size - x0) as usize;
            !std::mem::replace(&mut queued[k], true)
        };
        let mut ring: Vec<usize> = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * size + x) as usize;
                if !baked.filled[i]
                    && neighbours(i).any(|j| baked.filled[j])
                    && queue(i, &mut queued)
                {
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

const NEIGHBOURS: [(i64, i64); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

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
    /// How far `origin` sits off the surface.
    bias: f32,
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
    /// Linear, per triangle.
    albedo: &'a [[f32; 3]],
    /// Longer than any path through the scene (directional shadow rays).
    far: f32,
    ao_samples: u32,
    ao_distance: f32,
    gi_samples: u32,
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
        albedo: &'a [[f32; 3]],
        options: &BakeOptions,
    ) -> Result<Self, String> {
        // A parry trimesh BVH, as `Physics.trimesh` will build; one shared
        // builder can serve both once that lands.
        let mesh = TriMesh::new(
            positions.iter().map(|&p| v(p)).collect(),
            triangles.to_vec(),
        )
        .map_err(|e| format!("cannot build the ray mesh: {e:?}"))?;
        let (lo, hi) = positions.iter().fold(
            (
                Vector::splat(f32::INFINITY),
                Vector::splat(f32::NEG_INFINITY),
            ),
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
                .map(|t| texels(unwrapped, t))
                .collect(),
            size: unwrapped.size,
            lights,
            albedo,
            far: diagonal * 2.0 + 1.0,
            ao_samples: options.ao_samples,
            ao_distance: options.ao_distance,
            gi_samples: options.gi_samples,
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
        let (x, y) = ((s.texel % self.size) as f64, (s.texel / self.size) as f64);
        let at = [
            x + ((j & 1) as f64 + unit(hash(h, 2 * j as u64 + 2)) as f64) / 2.0,
            y + ((j >> 1) as f64 + unit(hash(h, 2 * j as u64 + 3)) as f64) / 2.0,
        ];
        let corners = self.texels[s.triangle as usize];
        let weights = if cross(corners[0], corners[1], corners[2]) == 0.0 {
            s.weights
        } else {
            let w = barycentric(corners, at).map(|w| w.max(0.0));
            let sum: f64 = w.iter().sum();
            w.map(|w| (w / sum) as f32)
        };
        let mix = |values: &[[f32; 3]]| {
            (0..3).fold(Vector::ZERO, |acc, k| {
                acc + v(values[tri[k] as usize]) * weights[k]
            })
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
        // Off the surface by more than f32 rounding at this distance from
        // the origin, against self-hits; local, so far geometry can't widen it.
        let bias = 1e-4 + 1e-5 * p.abs().max_element();
        Point {
            p,
            n,
            origin: p + face * bias,
            bias,
            face,
        }
    }

    /// Linear light arriving at a texel, averaged over its sub-samples: each
    /// light's `radiance × N·L × falloff`, shadowed if it casts shadows; and
    /// apart, ambient × the unoccluded share.
    fn light_at(&self, s: &Sample) -> ([f32; 3], [f32; 3]) {
        let h = hash(self.seed, s.texel as u64);
        let points: [_; SUBSAMPLES] = std::array::from_fn(|j| self.point(s, j, h));
        let mut total = Vector::ZERO;
        let mut ambient = Vector::ZERO;
        let mut occlusion = None;
        for light in self.lights {
            if let Light::Ambient { color, .. } = *light {
                let open = *occlusion.get_or_insert_with(|| self.unoccluded(&points, h));
                ambient += v(radiance(color, 1.0)) * open;
                continue;
            }
            for &Point {
                p, n, origin, bias, ..
            } in &points
            {
                let (color, l, atten, max) = match *light {
                    Light::Ambient { .. } => unreachable!("handled above"),
                    Light::Directional {
                        direction,
                        color,
                        intensity,
                        ..
                    } => (
                        radiance(color, intensity),
                        -v(direction).normalize_or_zero(),
                        1.0,
                        self.far,
                    ),
                    Light::Point {
                        position,
                        color,
                        intensity,
                        range,
                        ..
                    } => {
                        let to = v(position) - p;
                        let d = to.length();
                        (
                            radiance(color, intensity),
                            to / d.max(1e-4),
                            distance_falloff(d, range),
                            d - 2.0 * bias,
                        )
                    }
                    Light::Spot {
                        position,
                        direction,
                        color,
                        intensity,
                        range,
                        cone_angle,
                        ..
                    } => {
                        let to = v(position) - p;
                        let d = to.length();
                        let l = to / d.max(1e-4);
                        let cone = cone_falloff(
                            (-l).dot(v(direction).normalize_or_zero()),
                            cone_angle.cos(),
                        );
                        (
                            radiance(color, intensity),
                            l,
                            distance_falloff(d, range) * cone,
                            d - 2.0 * bias,
                        )
                    }
                };
                let ndotl = n.dot(l).max(0.0);
                if ndotl * atten <= 0.0 || (light.casts_shadows() && self.blocked(origin, l, max)) {
                    continue;
                }
                total += v(color) * (ndotl * atten / SUBSAMPLES as f32);
            }
        }
        (total.to_array(), ambient.to_array())
    }

    /// Light reflected onto texel `s` by the surfaces around it, whose
    /// incoming light is `lit`: the mean, over `gi_samples` cosine-weighted
    /// rays, of the albedo × light where each first hits a front face. That
    /// is a Lambertian bounce in the lightmap's units (the shader draws a
    /// surface as albedo × light). Misses and back faces add nothing.
    fn gather(&self, s: &Sample, lit: &Baked, bounce: u32) -> [f32; 3] {
        let h = hash(self.seed, s.texel as u64);
        let points: [_; SUBSAMPLES] = std::array::from_fn(|j| self.point(s, j, h));
        // A rotation per texel and bounce, apart from occlusion's.
        let r = hash(h, 16 + bounce as u64);
        let (r1, r2) = (unit(r), unit(hash(r, 1)));
        let n = self.gi_samples;
        let mut sum = Vector::ZERO;
        for k in 0..n {
            let Point { origin, face, .. } = points[k as usize % SUBSAMPLES];
            let dir = cosine_dir(face, k, n, r1, r2);
            let Some((tri, weights)) = self.nearest(origin, dir, self.far) else {
                continue;
            };
            let corners = self.triangles[tri as usize];
            let [a, b, c] = corners.map(|i| v(self.positions[i as usize]));
            let normal = (0..3).fold(Vector::ZERO, |acc, k| {
                acc + v(self.normals[corners[k] as usize]) * weights[k]
            });
            let mut facing = (b - a).cross(c - a);
            if facing.dot(normal) < 0.0 {
                facing = -facing;
            }
            if facing.dot(dir) >= 0.0 {
                continue;
            }
            let texels = self.texels[tri as usize];
            let at = [0, 1].map(|k| (0..3).map(|j| texels[j][k] * weights[j] as f64).sum());
            sum += v(self.albedo[tri as usize]) * v(bilinear(lit, at));
        }
        (sum / n as f32).to_array()
    }

    /// The first triangle along unit `dir` within `max`, and the hit's
    /// barycentric weights. A tie (a ray through a shared edge) goes to the
    /// lowest triangle, whatever order the BVH visits them in.
    fn nearest(&self, origin: Vector, dir: Vector, max: f32) -> Option<(u32, [f32; 3])> {
        let ray = Ray::new(origin, dir);
        let mut best: Option<(f32, u32, [f32; 3])> = None;
        self.mesh.bvh().traverse(|node| {
            let limit = best.map_or(max, |b| b.0);
            if node.cast_ray(&ray, limit) > limit {
                return TraversalAction::Prune;
            }
            if let Some(t) = node.leaf_data() {
                let tri = self.mesh.triangle(t);
                if let Some((hit, w)) = local_ray_intersection_with_triangle(tri.a, tri.b, tri.c, &ray) {
                    let toi = hit.time_of_impact;
                    let better = best.map_or(toi <= max, |(bt, bi, _)| {
                        toi < bt || (toi == bt && t < bi)
                    });
                    if better {
                        best = Some((toi, t, w.to_array()));
                    }
                }
            }
            TraversalAction::Continue
        });
        best.map(|(_, t, w)| (t, w))
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
                let dir = cosine_dir(face, k, n, r1, r2);
                !self.blocked(origin, dir, self.ao_distance)
            })
            .count();
        open as f32 / n as f32
    }
}

/// Direction `k` of `n` cosine-weighted about unit `face`: a Hammersley set
/// rotated by `(r1, r2)` in [0, 1).
fn cosine_dir(face: Vector, k: u32, n: u32, r1: f32, r2: f32) -> Vector {
    let (t, b) = basis(face);
    let u1 = ((k as f32 + 0.5) / n as f32 + r1).fract();
    let u2 = (radical_inverse(k) + r2).fract();
    let (r, phi) = (u1.sqrt(), u2 * std::f32::consts::TAU);
    let dir = t * (r * phi.cos()) + b * (r * phi.sin()) + face * (1.0 - u1).max(0.0).sqrt();
    dir.normalize_or_zero()
}

/// `lit` filtered bilinearly at `at`, in texels; background texels don't
/// count.
fn bilinear(lit: &Baked, at: [f64; 2]) -> [f32; 3] {
    let size = lit.size as i64;
    let (x, y) = (at[0] - 0.5, at[1] - 0.5);
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
    for (dx, dy, w) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        let (qx, qy) = (x0 + dx, y0 + dy);
        if qx < 0 || qy < 0 || qx >= size || qy >= size {
            continue;
        }
        let q = (qy * size + qx) as usize;
        if lit.filled[q] && w > 0.0 {
            sum = add(sum, lit.light[q].map(|c| c * w));
            total += w;
        }
    }
    if total > 0.0 {
        sum.map(|c| c / total)
    } else {
        [0.0; 3]
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
    let mut z = a ^ b
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0x6A09_E667_F3BC_C909);
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
            self.triangles
                .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
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
            unwrap: UnwrapOptions {
                texels_per_meter: 16.0,
                padding: 2,
                max_size: 1024,
            },
            ao_samples,
            ao_distance: 1.0,
            bounces: 0,
            gi_samples: 64,
            seed: 7,
            threads: 4,
        }
    }

    struct Bake {
        mesh: Mesh,
        unwrapped: Unwrapped,
        baked: Baked,
    }

    /// Linear albedo: Cornell white, red and green.
    const WHITE: [f32; 3] = [0.73, 0.73, 0.73];
    const RED: [f32; 3] = [0.63, 0.065, 0.05];
    const GREEN: [f32; 3] = [0.14, 0.45, 0.09];

    fn run(mesh: Mesh, lights: &[Light], options: BakeOptions) -> Bake {
        let albedo = vec![WHITE; mesh.triangles.len()];
        run_with(mesh, lights, &albedo, options)
    }

    fn run_with(mesh: Mesh, lights: &[Light], albedo: &[[f32; 3]], options: BakeOptions) -> Bake {
        let unwrapped = unwrap(&mesh.positions, &mesh.triangles, options.unwrap).unwrap();
        let baked = bake(
            &mesh.positions,
            &mesh.normals,
            &mesh.triangles,
            &unwrapped,
            lights,
            albedo,
            &options,
        )
        .unwrap();
        Bake {
            mesh,
            unwrapped,
            baked,
        }
    }

    impl Bake {
        /// The sampled texel nearest `at` on a surface facing `normal`: its
        /// light and where it sampled.
        fn near(&self, at: [f32; 3], normal: [f32; 3]) -> ([f32; 3], Vector) {
            let m = &self.mesh;
            rasterize(&self.unwrapped)
                .iter()
                .filter(|s| {
                    v(m.normals[m.triangles[s.triangle as usize][0] as usize]).dot(v(normal)) > 0.99
                })
                .map(|s| {
                    let tri = m.triangles[s.triangle as usize];
                    let p = (0..3).fold(Vector::ZERO, |acc, k| {
                        acc + v(m.positions[tri[k] as usize]) * s.weights[k]
                    });
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
            assert!(
                (light[0] - expected).abs() <= 0.02 * expected,
                "at {p}: {light:?} vs {expected}"
            );
            assert_eq!(light[0], light[2]);
        }
    }

    /// A sun lights by N·L alone: straight down, the floor gets all of it
    /// and the back wall none; tilted 45° toward the wall, each gets cos 45°.
    #[test]
    fn directional_light_is_n_dot_l() {
        let sun = |dy: f32, dz: f32| Light::directional(0.0, dy, dz, 1.0, 1.0, 1.0, 1.0);
        let back = [0.0, 0.0, 1.0];
        let b = run(cornell(), &[sun(-1.0, 0.0)], options(0));
        assert!((b.near([-1.0, 0.0, -1.0], UP).0[0] - 1.0).abs() < 1e-5);
        assert_eq!(b.near([-1.0, 1.5, -4.0], back).0[0], 0.0);
        let b = run(cornell(), &[sun(-1.0, -1.0)], options(0));
        let cos45 = std::f32::consts::FRAC_1_SQRT_2;
        assert!((b.near([-1.0, 0.0, -1.0], UP).0[0] - cos45).abs() < 1e-5);
        assert!((b.near([-1.0, 1.5, -4.0], back).0[0] - cos45).abs() < 1e-5);
    }

    /// A spot aimed down from the lamp: full inside its cone (`cos θ ×
    /// falloff`, as a point light), nothing past its edge.
    #[test]
    fn spot_light_stays_in_its_cone() {
        let spot = Light::spot(
            LAMP[0], LAMP[1], LAMP[2], 0.0, -1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 6.0, 0.5236,
        );
        let b = run(cornell(), &[spot], options(0));
        let (inside, p) = b.near([0.3, 0.0, -2.0], UP);
        let to = v(LAMP) - p;
        let expected = to.y / to.length() * distance_falloff(to.length(), 6.0);
        assert!(
            (inside[0] - expected).abs() <= 0.02 * expected,
            "{inside:?} vs {expected}"
        );
        // 1.8 m out is 33° off the axis, past the 30° cone.
        assert_eq!(b.near([-1.8, 0.0, -2.0], UP).0[0], 0.0);
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
        assert!(
            (unshadowed[0] - lit[0]).abs() < 0.02 * lit[0],
            "{unshadowed:?} vs {lit:?}"
        );
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

    /// Same seed, same bytes at any thread count, bounces and denoising
    /// included; another seed differs.
    #[test]
    fn bakes_are_deterministic() {
        let lights = [Light::ambient(0.3, 0.3, 0.3), lamp().cast_shadows()];
        let bytes = |threads, seed| {
            let mut o = options(16);
            (o.threads, o.seed, o.bounces, o.gi_samples) = (threads, seed, 2, 16);
            run(cornell(), &lights, o).baked.rgba8(1.0)
        };
        let one = bytes(1, 7);
        assert_eq!(one, bytes(8, 7));
        assert_ne!(one, bytes(8, 8));
    }

    /// Geometry far away (here an unused vertex 10 km off) doesn't change
    /// how rays leave a surface, so it doesn't change the bake.
    #[test]
    fn far_geometry_leaves_the_bake_alone() {
        let lights = [Light::ambient(0.3, 0.3, 0.3), lamp().cast_shadows()];
        let mut far = cornell();
        far.positions.push([10_000.0, 0.0, 0.0]);
        far.normals.push([0.0, 1.0, 0.0]);
        let bytes = |m| run(m, &lights, options(16)).baked.rgba8(1.0);
        assert_eq!(bytes(far), bytes(cornell()));
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
        let rgba = b.baked.rgba8(1.0);
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

    /// The Cornell box's albedo: red left wall, green right, white elsewhere.
    fn cornell_albedo(m: &Mesh) -> Vec<[f32; 3]> {
        (0..m.triangles.len())
            .map(|t| match t / 2 {
                3 => RED,
                4 => GREEN,
                _ => WHITE,
            })
            .collect()
    }

    /// Bounced light takes the colour of what it bounced off: in the box
    /// without its block, under a lamp at mid-height, the floor beside the
    /// red wall turns red and beside the green wall green (whose albedo is a
    /// third as saturated). With no bounce, the white lamp's light stays white.
    #[test]
    fn bounced_light_takes_the_walls_colour() {
        let room = || {
            let mut m = cornell();
            m.triangles.truncate(10);
            m
        };
        let lights = [Light::point(0.0, 1.5, -2.0, 1.0, 1.0, 1.0, 1.0, 6.0)];
        let albedo = &cornell_albedo(&cornell())[..10];
        let mut o = options(0);
        o.bounces = 1;
        let b = run_with(room(), &lights, albedo, o);
        let (left, _) = b.near([-1.9, 0.0, -1.0], UP);
        let (right, _) = b.near([1.9, 0.0, -1.0], UP);
        assert!(left[0] / left[1] > 1.15, "beside red: {left:?}");
        assert!(right[1] / right[0] > 1.05, "beside green: {right:?}");
        o.bounces = 0;
        let b = run_with(room(), &lights, albedo, o);
        let (left, _) = b.near([-1.9, 0.0, -1.0], UP);
        assert_eq!(left[0], left[1], "direct only: {left:?}");
    }

    /// Each bounce adds light, less each time (albedo < 1), so the total
    /// converges.
    #[test]
    fn bounces_add_less_and_less() {
        let lights = [lamp().cast_shadows()];
        let total = |bounces| {
            let mut o = options(0);
            (o.bounces, o.gi_samples) = (bounces, 32);
            let b = run(cornell(), &lights, o);
            b.baked.light.iter().map(|c| luminance(*c)).sum::<f32>()
        };
        let totals: Vec<f32> = (0..=4).map(total).collect();
        let steps: Vec<f32> = totals.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.iter().all(|&d| d > 0.0), "{totals:?}");
        assert!(steps.windows(2).all(|d| d[1] < d[0]), "{totals:?}");
        assert!(steps[3] < 0.3 * steps[0], "{totals:?}");
    }

    /// Denoising averages within a chart and never reads across one: a
    /// bright chart beside a noisy dark one stays as it was, the dark one
    /// gets smoother but no brighter, and texels of no chart aren't read.
    #[test]
    fn denoise_stays_within_its_chart() {
        let size = 16u32;
        let n = (size * size) as usize;
        let chart: Vec<u32> = (0..n)
            .map(|i| match i % 16 {
                15 => u32::MAX,
                x if x < 8 => 0,
                _ => 1,
            })
            .collect();
        let light: Vec<[f32; 3]> = (0..n)
            .map(|i| match chart[i] {
                0 => [1.0; 3],
                1 => [0.1 + 0.05 * (unit(hash(3, i as u64)) - 0.5); 3],
                _ => [100.0; 3],
            })
            .collect();
        let out = denoise(size, &chart, &light, 2);
        let spread = |values: &[[f32; 3]]| {
            let dark: Vec<f32> = (0..n).filter(|&i| chart[i] == 1).map(|i| values[i][0]).collect();
            let mean = dark.iter().sum::<f32>() / dark.len() as f32;
            let max = dark.iter().cloned().fold(0.0, f32::max);
            (dark.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / dark.len() as f32, max)
        };
        for i in 0..n {
            match chart[i] {
                0 => assert!((out[i][0] - 1.0).abs() < 1e-6, "bright texel {i}: {:?}", out[i]),
                u32::MAX => assert_eq!(out[i], light[i]),
                _ => {}
            }
        }
        let (before, _) = spread(&light);
        let (after, max) = spread(&out);
        assert!(max < 0.125, "the dark chart read the bright one: {max}");
        assert!(after < 0.25 * before, "{after} vs {before}");
    }

    /// sRGB bytes keep the darks: a 0–0.1 linear ramp gets over three times
    /// as many levels as linear bytes would give, and `scale` divides
    /// before encoding.
    #[test]
    fn srgb_encoding_keeps_dark_steps() {
        let ramp = Baked {
            size: 32,
            light: (0..1024).map(|i| [0.1 * i as f32 / 1023.0; 3]).collect(),
            filled: vec![true; 1024],
        };
        let levels = |bytes: Vec<u8>| {
            let mut red: Vec<u8> = bytes.chunks(4).map(|p| p[0]).collect();
            red.dedup();
            red.len()
        };
        let linear = levels(
            ramp.light
                .iter()
                .flat_map(|c| [(c[0] * 255.0).round() as u8, 0, 0, 255])
                .collect(),
        );
        let srgb = levels(ramp.rgba8(1.0));
        assert_eq!(linear, 27);
        assert!(srgb > 3 * linear, "{srgb} levels vs {linear}");
        // Halved by `scale` 2, then decoded: 1.0 is stored as linear 0.5.
        let one = Baked {
            size: 1,
            light: vec![[1.0; 3]],
            filled: vec![true],
        };
        let byte = one.rgba8(2.0)[0];
        assert!((crate::color_space::srgb_u8_to_linear(byte) - 0.5).abs() < 0.005, "{byte}");
    }

    /// A ray through the edge two triangles share hits the lower one,
    /// whichever order the mesh lists them in.
    #[test]
    fn nearest_hit_ties_go_to_the_lowest_triangle() {
        for flip in [false, true] {
            let mut m = Mesh::default();
            m.quad([-1.0, 0.0, 1.0], [2.0, 0.0, 0.0], [0.0, 0.0, -2.0]);
            if flip {
                m.triangles.swap(0, 1);
            }
            let o = options(0);
            let u = unwrap(&m.positions, &m.triangles, o.unwrap).unwrap();
            let albedo = [WHITE; 2];
            let scene =
                Scene::new(&m.positions, &m.normals, &m.triangles, &u, &[], &albedo, &o).unwrap();
            // (0, 0, 0) is on the shared diagonal.
            let hit = scene.nearest(Vector::new(0.0, 1.0, 0.0), Vector::new(0.0, -1.0, 0.0), 10.0);
            assert_eq!(hit.map(|h| h.0), Some(0), "flipped: {flip}");
        }
    }

    /// Bake times: the Cornell box at 32 texels a metre and medsci1's scale
    /// (~26k triangles) at 8 and 16, each direct only and with bounces.
    /// Timing only: add `--config
    /// 'profile.dev.package.functor_runtime_common.opt-level=3'` (and parry3d,
    /// glam), then `-- --ignored --nocapture`. `BAKE_TIMES=tpm,ao,bounces,gi`
    /// times the level at one setting instead.
    #[test]
    #[ignore]
    fn bake_times() {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let time = |name: &str,
                    (positions, normals, triangles): (&[[f32; 3]], &[[f32; 3]], &[[u32; 3]]),
                    lights: &[Light],
                    (tpm, ao, bounces, gi): (f64, u32, u32, u32)| {
            let o = BakeOptions {
                unwrap: UnwrapOptions {
                    texels_per_meter: tpm,
                    padding: 2,
                    max_size: 4096,
                },
                ao_samples: ao,
                bounces,
                gi_samples: gi,
                threads,
                ..options(0)
            };
            let albedo = vec![WHITE; triangles.len()];
            let start = std::time::Instant::now();
            let u = unwrap(positions, triangles, o.unwrap).unwrap();
            let unwrapped = start.elapsed();
            bake(positions, normals, triangles, &u, lights, &albedo, &o).unwrap();
            eprintln!(
                "{name}: {} triangles, {} charts, {s}×{s}, {} texels, {} lights, {ao} AO rays, {bounces} bounces × {gi} rays, {threads} threads: unwrap {unwrapped:?}, total {:?}",
                triangles.len(),
                u.charts.len(),
                rasterize(&u).len(),
                lights.len(),
                start.elapsed(),
                s = u.size,
            );
        };
        let (positions, triangles) = super::super::unwrap::tests::level(72);
        let normals = triangles
            .iter()
            .fold(vec![[0.0; 3]; positions.len()], |mut n, t| {
                let [a, b, c] = t.map(|i| v(positions[i as usize]));
                t.iter()
                    .for_each(|&i| n[i as usize] = (b - a).cross(c - a).normalize().to_array());
                n
            });
        let lights: Vec<Light> = (0..72)
            .map(|r| {
                Light::point(r as f32 * 7.0 + 3.0, 2.6, 3.0, 1.0, 0.9, 0.8, 1.0, 6.0).cast_shadows()
            })
            .chain([Light::ambient(0.2, 0.2, 0.25)])
            .collect();
        let level = (&positions[..], &normals[..], &triangles[..]);
        if let Ok(setting) = std::env::var("BAKE_TIMES") {
            let n: Vec<f64> = setting.split(',').map(|x| x.parse().unwrap()).collect();
            let at = |i: usize| n[i] as u32;
            return time("level", level, &lights, (n[0], at(1), at(2), at(3)));
        }
        let box_lights = [Light::ambient(0.3, 0.3, 0.3), lamp().cast_shadows()];
        let m = cornell();
        for setting in [(32.0, 64, 0, 0), (32.0, 64, 1, 64), (32.0, 64, 2, 64)] {
            time(
                "cornell",
                (&m.positions, &m.normals, &m.triangles),
                &box_lights,
                setting,
            );
        }
        for setting in [(8.0, 16, 0, 0), (8.0, 16, 1, 32), (16.0, 64, 0, 0), (16.0, 64, 1, 32)] {
            time("level", level, &lights, setting);
        }
    }
}
