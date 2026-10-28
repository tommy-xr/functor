//! `Lightmap.unwrap`: lightmap UVs for a static mesh. Edge-adjacent, coplanar
//! triangles become charts; each is projected onto its plane at a fixed
//! texel density, turned to its smallest bounding rectangle, and packed with
//! `Texture.atlas`'s packer. A vertex on a seam between charts is
//! duplicated, since it needs a lightmap UV in each.

use std::collections::HashMap;

use crate::texture::{check_max_size, pack};

/// A chart only takes triangles facing within ~5.7° of its first one, so
/// projecting onto that plane shrinks no triangle by more than 0.5%.
const MIN_COS: f64 = 0.995;

#[derive(Debug, Clone, Copy)]
pub struct UnwrapOptions {
    pub texels_per_meter: f64,
    /// Texels kept free around each chart, for a bake to dilate into.
    pub padding: u32,
    /// The largest atlas side: a power of two, 1 to 4096.
    pub max_size: u32,
}

/// One chart's texels in the atlas, padding not included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChartRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Unwrapped {
    /// The input vertex each output vertex copies.
    pub source: Vec<u32>,
    /// The input triangles, in order, indexing the output vertices.
    pub triangles: Vec<[u32; 3]>,
    /// One per output vertex, in [0, 1]; `v = 0` is the atlas's row 0.
    pub uvs2: Vec<[f32; 2]>,
    /// Each triangle's chart.
    pub chart_of: Vec<u32>,
    /// Ordered by lowest triangle.
    pub charts: Vec<ChartRect>,
    /// The atlas side in texels: square, a power of two.
    pub size: u32,
}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// `a` normalized, or zero for a zero vector.
fn unit(a: V3) -> V3 {
    let len = dot(a, a).sqrt();
    if len > 0.0 {
        a.map(|x| x / len)
    } else {
        [0.0; 3]
    }
}

/// Lightmap UVs for `triangles` over `positions`, whose indices the caller
/// has checked. Deterministic: charts grow from the lowest unassigned
/// triangle, and output vertices appear in triangle order.
pub fn unwrap(
    positions: &[[f32; 3]],
    triangles: &[[u32; 3]],
    options: UnwrapOptions,
) -> Result<Unwrapped, String> {
    let UnwrapOptions {
        texels_per_meter: tpm,
        padding,
        max_size,
    } = options;
    if !(tpm.is_finite() && tpm > 0.0) {
        return Err(format!(
            "texelsPerMeter must be a finite number above 0, got {tpm}"
        ));
    }
    check_max_size(max_size)?;
    let at = |v: u32| positions[v as usize].map(f64::from);

    // Weld by position, so a chart spans polygons that don't share vertices
    // (`+ 0.0` turns -0 into 0).
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    let weld: Vec<u32> = positions
        .iter()
        .map(|p| {
            let next = ids.len() as u32;
            *ids.entry(p.map(|x| (x + 0.0).to_bits())).or_insert(next)
        })
        .collect();
    let normals: Vec<V3> = triangles
        .iter()
        .map(|t| {
            let o = at(t[0]);
            unit(cross(sub(at(t[1]), o), sub(at(t[2]), o)))
        })
        .collect();
    let edge = |t: &[u32; 3], k: usize| {
        let (a, b) = (weld[t[k] as usize], weld[t[(k + 1) % 3] as usize]);
        (a.min(b), a.max(b))
    };
    let mut edges: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (i, t) in triangles.iter().enumerate() {
        for k in 0..3 {
            edges.entry(edge(t, k)).or_default().push(i as u32);
        }
    }

    // Grow each chart across edges from its seed, taking triangles that face
    // its way and lie within half a texel of its plane. The plane test keeps
    // gentle curves (a spiral ramp) from wrapping onto themselves.
    let half_texel = 0.5 / tpm;
    let mut chart_of = vec![u32::MAX; triangles.len()];
    let mut seeds: Vec<usize> = Vec::new();
    let mut stack = Vec::new();
    for seed in 0..triangles.len() {
        if chart_of[seed] != u32::MAX {
            continue;
        }
        let chart = seeds.len() as u32;
        seeds.push(seed);
        chart_of[seed] = chart;
        let n = normals[seed];
        let origin = at(triangles[seed][0]);
        // A degenerate seed has no plane: it stays alone.
        if n == [0.0; 3] {
            continue;
        }
        let fits = |t: usize| {
            let m = normals[t];
            (m == [0.0; 3] || dot(m, n) >= MIN_COS)
                && triangles[t]
                    .iter()
                    .all(|&v| dot(sub(at(v), origin), n).abs() <= half_texel)
        };
        stack.push(seed);
        while let Some(t) = stack.pop() {
            for k in 0..3 {
                for &u in &edges[&edge(&triangles[t], k)] {
                    let u = u as usize;
                    if chart_of[u] == u32::MAX && fits(u) {
                        chart_of[u] = chart;
                        stack.push(u);
                    }
                }
            }
        }
    }

    // One output vertex per (chart, input vertex), in triangle order.
    let mut remap: HashMap<(u32, u32), u32> = HashMap::new();
    let mut source = Vec::new();
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); seeds.len()];
    let mut out_triangles = Vec::with_capacity(triangles.len());
    for (t, &chart) in triangles.iter().zip(&chart_of) {
        out_triangles.push(t.map(|v| {
            *remap.entry((chart, v)).or_insert_with(|| {
                let id = source.len() as u32;
                source.push(v);
                members[chart as usize].push(id);
                id
            })
        }));
    }

    // Project each chart into texels: (u, v) from its corner, and its size.
    let mut local = vec![[0.0f64; 2]; source.len()];
    let mut sizes = Vec::with_capacity(seeds.len());
    for (chart, verts) in members.iter().enumerate() {
        let n = normals[seeds[chart]];
        let (t, b) = basis(if n == [0.0; 3] { [0.0, 0.0, 1.0] } else { n });
        let flat: Vec<[f64; 2]> = verts
            .iter()
            .map(|&i| {
                let p = at(source[i as usize]);
                [dot(p, t), dot(p, b)]
            })
            .collect();
        let (c, s) = min_area_rotation(&flat);
        let mut uv: Vec<[f64; 2]> = flat
            .iter()
            .map(|&[x, y]| [c * x + s * y, c * y - s * x])
            .collect();
        let extent = |uv: &[[f64; 2]], k: usize| {
            let lo = uv.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
            let hi = uv.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
            (lo, hi - lo)
        };
        // Wider than tall (a quarter turn, not a mirror): packs tighter.
        if extent(&uv, 0).1 < extent(&uv, 1).1 {
            uv.iter_mut().for_each(|p| *p = [p[1], -p[0]]);
        }
        let ((u0, du), (v0, dv)) = (extent(&uv, 0), extent(&uv, 1));
        let texels = |d: f64| (d * tpm).ceil().max(1.0);
        let (w, h) = (texels(du), texels(dv));
        if w + 2.0 * padding as f64 > max_size as f64 || h + 2.0 * padding as f64 > max_size as f64 {
            return Err(format!(
                "the chart from triangle {} is {w}×{h} texels plus padding, over maxSize \
{max_size}; lower texelsPerMeter",
                seeds[chart]
            ));
        }
        for (&i, p) in verts.iter().zip(&uv) {
            local[i as usize] = [(p[0] - u0) * tpm, (p[1] - v0) * tpm];
        }
        sizes.push((w as u32, h as u32));
    }

    let padded: Vec<(u32, u32)> = sizes
        .iter()
        .map(|&(w, h)| (w + 2 * padding, h + 2 * padding))
        .collect();
    let (size, slots) = pack(&padded, max_size).map_err(|area| {
        format!(
            "{} charts need more than {max_size}×{max_size} ({area} texels with padding); \
lower texelsPerMeter or raise maxSize",
            sizes.len()
        )
    })?;
    let charts: Vec<ChartRect> = slots
        .iter()
        .zip(&sizes)
        .map(|(&(x, y), &(width, height))| ChartRect {
            x: x + padding,
            y: y + padding,
            width,
            height,
        })
        .collect();
    let mut vertex_chart = vec![0u32; source.len()];
    for (chart, verts) in members.iter().enumerate() {
        verts.iter().for_each(|&i| vertex_chart[i as usize] = chart as u32);
    }
    let uvs2 = local
        .iter()
        .zip(&vertex_chart)
        .map(|(p, &chart)| {
            let rect = charts[chart as usize];
            [
                ((rect.x as f64 + p[0]) / size as f64) as f32,
                ((rect.y as f64 + p[1]) / size as f64) as f32,
            ]
        })
        .collect();
    Ok(Unwrapped {
        source,
        triangles: out_triangles,
        uvs2,
        chart_of,
        charts,
        size,
    })
}

/// Two unit axes spanning the plane with unit normal `n`.
fn basis(n: V3) -> (V3, V3) {
    // Cross with the world axis least aligned with `n`.
    let k = (0..3)
        .min_by(|&i, &j| n[i].abs().total_cmp(&n[j].abs()))
        .unwrap();
    let mut axis = [0.0; 3];
    axis[k] = 1.0;
    let t = unit(cross(axis, n));
    (t, cross(n, t))
}

/// The rotation `(cos, sin)` that gives `points` their smallest bounding
/// rectangle; one side of it lies along a convex-hull edge. Ties keep the
/// unrotated axes, so axis-aligned walls stay aligned with the texel grid.
fn min_area_rotation(points: &[[f64; 2]]) -> (f64, f64) {
    let hull = convex_hull(points);
    let area = |c: f64, s: f64| {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for &[x, y] in &hull {
            for (k, d) in [c * x + s * y, c * y - s * x].into_iter().enumerate() {
                lo[k] = lo[k].min(d);
                hi[k] = hi[k].max(d);
            }
        }
        (hi[0] - lo[0]) * (hi[1] - lo[1])
    };
    let mut best = (1.0, 0.0, area(1.0, 0.0));
    for (i, a) in hull.iter().enumerate() {
        let b = hull[(i + 1) % hull.len()];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if len == 0.0 {
            continue;
        }
        let (c, s) = (dx / len, dy / len);
        let candidate = area(c, s);
        if candidate < best.2 * (1.0 - 1e-9) {
            best = (c, s, candidate);
        }
    }
    (best.0, best.1)
}

/// Andrew's monotone chain: the hull counter-clockwise, without collinear
/// points.
fn convex_hull(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    sorted.dedup();
    if sorted.len() < 3 {
        return sorted;
    }
    let turn = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    // The lower hull left to right, then the upper one back.
    let mut hull: Vec<[f64; 2]> = Vec::with_capacity(sorted.len() + 1);
    for &p in &sorted {
        while hull.len() >= 2 && turn(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in sorted.iter().rev().skip(1) {
        while hull.len() >= lower && turn(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull
}

#[cfg(test)]
mod tests {
    use super::*;

    type Mesh = (Vec<[f32; 3]>, Vec<[u32; 3]>);

    const OPTIONS: UnwrapOptions = UnwrapOptions {
        texels_per_meter: 16.0,
        padding: 2,
        max_size: 4096,
    };

    /// Two triangles over a quad, corners in order around it.
    fn quad(tris: &mut Vec<[u32; 3]>, [a, b, c, d]: [u32; 4]) {
        tris.extend([[a, b, c], [a, c, d]]);
    }

    /// An indexed `w`×`h`×`d` box: 8 shared corners, 12 triangles.
    fn cube(w: f32, h: f32, d: f32) -> Mesh {
        let positions = (0..8)
            .map(|i| [(i & 1) as f32 * w, (i >> 1 & 1) as f32 * h, (i >> 2) as f32 * d])
            .collect();
        let mut tris = Vec::new();
        for face in [
            [0, 2, 6, 4],
            [1, 5, 7, 3],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 1, 3, 2],
            [4, 6, 7, 5],
        ] {
            quad(&mut tris, face);
        }
        (positions, tris)
    }

    /// A 4×3×5 room seen from inside: the floor is four quads with their own
    /// vertices (as level polygons come), the rest an indexed box.
    fn room() -> Mesh {
        let (mut positions, box_tris) = cube(4.0, 3.0, 5.0);
        // Drop the box's floor (y = 0) and tile it instead.
        let mut tris: Vec<[u32; 3]> = box_tris
            .into_iter()
            .filter(|t| t.iter().any(|&v| positions[v as usize][1] > 0.0))
            .collect();
        for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let base = positions.len() as u32;
            let (x, z) = (i as f32 * 2.0, j as f32 * 2.5);
            positions.extend([[x, 0.0, z], [x + 2.0, 0.0, z], [x + 2.0, 0.0, z + 2.5], [x, 0.0, z + 2.5]]);
            quad(&mut tris, [base, base + 1, base + 2, base + 3]);
        }
        (positions, tris)
    }

    /// An indexed cylinder: `n` side quads and two fan caps sharing the rings.
    fn cylinder(n: u32) -> Mesh {
        let ring = |y: f32| {
            (0..n).map(move |i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                [0.5 * a.cos(), y, 0.5 * a.sin()]
            })
        };
        let mut positions: Vec<[f32; 3]> = ring(0.0).chain(ring(2.0)).collect();
        positions.extend([[0.0, 0.0, 0.0], [0.0, 2.0, 0.0]]);
        let mut tris = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            quad(&mut tris, [i, j, n + j, n + i]);
            tris.push([2 * n, j, i]);
            tris.push([2 * n + 1, n + i, n + j]);
        }
        (positions, tris)
    }

    /// A ramp curling 1.5 times round a column, rising 0.1 m a turn: its
    /// normals stay within 2° of each other, so only the plane test keeps
    /// it from projecting onto itself.
    fn spiral() -> Mesh {
        let steps = 96;
        let mut positions = Vec::new();
        for i in 0..=steps {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            let y = 0.1 * i as f32 / 64.0;
            positions.extend([[a.cos(), y, a.sin()], [2.0 * a.cos(), y, 2.0 * a.sin()]]);
        }
        let mut tris = Vec::new();
        for i in 0..steps {
            let k = 2 * i;
            quad(&mut tris, [k, k + 2, k + 3, k + 1]);
        }
        (positions, tris)
    }

    /// `n` rooms of 6×3×6 m in a row, every wall, floor and ceiling tiled
    /// with 1 m polygons of their own, plus six 0.5 m crates per room at
    /// assorted angles: about medsci1's triangle count at `n` = 72.
    fn level(n: usize) -> Mesh {
        let (mut positions, mut tris) = (Vec::new(), Vec::new());
        let tile = |positions: &mut Vec<[f32; 3]>, tris: &mut Vec<[u32; 3]>, corners: [[f32; 3]; 4]| {
            let base = positions.len() as u32;
            positions.extend(corners);
            quad(tris, [base, base + 1, base + 2, base + 3]);
        };
        for r in 0..n {
            let x0 = r as f32 * 7.0;
            for i in 0..6 {
                for j in 0..6 {
                    let (x, z) = (x0 + i as f32, j as f32);
                    for y in [0.0, 3.0] {
                        tile(&mut positions, &mut tris, [[x, y, z], [x + 1.0, y, z], [x + 1.0, y, z + 1.0], [x, y, z + 1.0]]);
                    }
                }
                for j in 0..3 {
                    let (s, y) = (i as f32, j as f32);
                    tile(&mut positions, &mut tris, [[x0 + s, y, 0.0], [x0 + s + 1.0, y, 0.0], [x0 + s + 1.0, y + 1.0, 0.0], [x0 + s, y + 1.0, 0.0]]);
                    tile(&mut positions, &mut tris, [[x0 + s, y, 6.0], [x0 + s + 1.0, y, 6.0], [x0 + s + 1.0, y + 1.0, 6.0], [x0 + s, y + 1.0, 6.0]]);
                    tile(&mut positions, &mut tris, [[x0, y, s], [x0, y, s + 1.0], [x0, y + 1.0, s + 1.0], [x0, y + 1.0, s]]);
                    tile(&mut positions, &mut tris, [[x0 + 6.0, y, s], [x0 + 6.0, y, s + 1.0], [x0 + 6.0, y + 1.0, s + 1.0], [x0 + 6.0, y + 1.0, s]]);
                }
            }
            for c in 0..6 {
                let (crate_positions, crate_tris) = cube(0.5, 0.5, 0.5);
                let angle = (r * 6 + c) as f32 * 0.37;
                let (sin, cos) = angle.sin_cos();
                let base = positions.len() as u32;
                positions.extend(crate_positions.iter().map(|&[x, y, z]| {
                    [x0 + 1.0 + c as f32 * 0.8 + cos * x - sin * z, y, 3.0 + sin * x + cos * z]
                }));
                tris.extend(crate_tris.iter().map(|t| t.map(|v| base + v)));
            }
        }
        (positions, tris)
    }

    fn texel(u: &Unwrapped, v: u32) -> [f64; 2] {
        u.uvs2[v as usize].map(|x| x as f64 * u.size as f64)
    }

    /// Twice the signed texel area of output triangle `i`.
    fn uv_area(u: &Unwrapped, i: usize) -> f64 {
        let [a, b, c] = u.triangles[i].map(|v| texel(u, v));
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }

    /// Unwrap and check every invariant: triangles map back to the input,
    /// uvs2 lie in [0, 1] and inside their chart, charts share no vertex,
    /// padded charts are disjoint, and each chart's texel density is
    /// `texels_per_meter` within 1%.
    fn check((positions, tris): &Mesh, options: UnwrapOptions) -> Unwrapped {
        let u = unwrap(positions, tris, options).unwrap();
        assert_eq!(u.triangles.len(), tris.len());
        for (t, out) in tris.iter().zip(&u.triangles) {
            assert_eq!(out.map(|v| u.source[v as usize]), *t);
        }
        assert!(u.uvs2.iter().flatten().all(|x| (0.0..=1.0).contains(x)));
        let mut owner = vec![u32::MAX; u.source.len()];
        for (i, out) in u.triangles.iter().enumerate() {
            let chart = u.chart_of[i];
            let r = u.charts[chart as usize];
            for &v in out {
                assert!(owner[v as usize] == u32::MAX || owner[v as usize] == chart, "vertex {v} in two charts");
                owner[v as usize] = chart;
                let [x, y] = texel(&u, v);
                let eps = 1e-3;
                assert!(x >= r.x as f64 - eps && x <= (r.x + r.width) as f64 + eps, "triangle {i} leaves {r:?}");
                assert!(y >= r.y as f64 - eps && y <= (r.y + r.height) as f64 + eps, "triangle {i} leaves {r:?}");
            }
        }
        let side = u.size as usize;
        let mut used = vec![false; side * side];
        let pad = options.padding;
        for r in &u.charts {
            for y in r.y - pad..r.y + r.height + pad {
                for x in r.x - pad..r.x + r.width + pad {
                    let cell = &mut used[y as usize * side + x as usize];
                    assert!(!*cell, "padded charts overlap at {x},{y}");
                    *cell = true;
                }
            }
        }
        let mut areas = vec![(0.0, 0.0); u.charts.len()];
        for (i, t) in tris.iter().enumerate() {
            let [a, b, c] = t.map(|v| positions[v as usize].map(f64::from));
            let world = dot(cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(c, a))).sqrt();
            let chart = &mut areas[u.chart_of[i] as usize];
            chart.0 += uv_area(&u, i).abs();
            chart.1 += world;
        }
        for (chart, (uv, world)) in areas.iter().enumerate() {
            if *world > 1e-9 {
                let density = (uv / world).sqrt();
                let tpm = options.texels_per_meter;
                assert!((density / tpm - 1.0).abs() < 0.01, "chart {chart}: {density} texels/m");
            }
        }
        u
    }

    /// No point of the atlas lies strictly inside two triangles (sampled at
    /// a quarter texel).
    fn assert_no_overlap(u: &Unwrapped) {
        let side = u.size as usize * 4;
        let mut hits = vec![u32::MAX; side * side];
        for (i, t) in u.triangles.iter().enumerate() {
            let [a, b, c] = t.map(|v| texel(u, v).map(|x| x * 4.0));
            let sign = uv_area(u, i).signum();
            let edge = |p: [f64; 2], q: [f64; 2], s: [f64; 2]| {
                sign * ((q[0] - p[0]) * (s[1] - p[1]) - (q[1] - p[1]) * (s[0] - p[0])) > 1e-6
            };
            let lo = |k: usize| a[k].min(b[k]).min(c[k]).floor() as usize;
            let hi = |k: usize| (a[k].max(b[k]).max(c[k]).ceil() as usize).min(side);
            for y in lo(1)..hi(1) {
                for x in lo(0)..hi(0) {
                    let s = [x as f64 + 0.5, y as f64 + 0.5];
                    if edge(a, b, s) && edge(b, c, s) && edge(c, a, s) {
                        let hit = &mut hits[y * side + x];
                        assert_eq!(*hit, u32::MAX, "triangles {hit} and {i} overlap");
                        *hit = i as u32;
                    }
                }
            }
        }
    }

    #[test]
    fn an_indexed_cube_comes_back_with_24_vertices_in_6_charts() {
        let mesh = cube(1.0, 1.0, 1.0);
        let u = check(&mesh, OPTIONS);
        assert_eq!((u.charts.len(), u.source.len()), (6, 24));
        assert!((0..12).all(|i| uv_area(&u, i).abs() > 1.0));
        assert_no_overlap(&u);
        // Each corner splits three ways: one position, three distinct uvs2.
        for v in 0..8 {
            let copies: Vec<[f32; 2]> = (0..u.source.len())
                .filter(|&i| u.source[i] == v)
                .map(|i| u.uvs2[i])
                .collect();
            assert_eq!(copies.len(), 3, "corner {v}");
            assert!(copies[0] != copies[1] && copies[1] != copies[2] && copies[0] != copies[2]);
        }
        // A 1 m face at 16 texels/m is 16 texels square.
        assert!(u.charts.iter().all(|r| (r.width, r.height) == (16, 16)));
    }

    #[test]
    fn chart_counts_follow_the_planes() {
        let room = check(&room(), OPTIONS);
        assert_eq!(room.charts.len(), 6);
        assert_no_overlap(&room);
        // Floor tiles weld into one chart; their unshared vertices stay apart.
        assert_eq!(room.source.len(), 4 * 5 + 16);
        let cylinder = check(&cylinder(16), OPTIONS);
        assert_eq!(cylinder.charts.len(), 16 + 2);
        assert_no_overlap(&cylinder);
    }

    #[test]
    fn a_gentle_spiral_splits_instead_of_overlapping() {
        let u = check(&spiral(), OPTIONS);
        assert!(u.charts.len() > 1, "{} charts", u.charts.len());
        assert_no_overlap(&u);
    }

    #[test]
    fn charts_turn_to_their_smallest_rectangle() {
        // A 2×1 m quad at 30° in the floor plane packs as 32×16 (plus
        // rounding), not its 36×30 axis-aligned bounds.
        let (sin, cos) = 0.5236f32.sin_cos();
        let corner = |x: f32, z: f32| [cos * x - sin * z, 0.0, sin * x + cos * z];
        let mesh = (vec![corner(0.0, 0.0), corner(2.0, 0.0), corner(2.0, 1.0), corner(0.0, 1.0)], vec![[0, 1, 2], [0, 2, 3]]);
        let u = check(&mesh, OPTIONS);
        let r = u.charts[0];
        assert!(r.width <= 33 && r.height <= 17, "{r:?}");
    }

    #[test]
    fn unwrap_is_deterministic() {
        let mesh = level(4);
        let a = unwrap(&mesh.0, &mesh.1, OPTIONS).unwrap();
        let b = unwrap(&mesh.0, &mesh.1, OPTIONS).unwrap();
        let bits = |u: &Unwrapped| u.uvs2.iter().flatten().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!((&a.source, &a.triangles, bits(&a)), (&b.source, &b.triangles, bits(&b)));
    }

    #[test]
    fn errors_name_the_problem() {
        let (positions, tris) = cube(1.0, 1.0, 1.0);
        let err = |options| unwrap(&positions, &tris, options).err().unwrap();
        assert_eq!(
            err(UnwrapOptions { texels_per_meter: 0.0, ..OPTIONS }),
            "texelsPerMeter must be a finite number above 0, got 0"
        );
        assert_eq!(
            err(UnwrapOptions { max_size: 48, ..OPTIONS }),
            "maxSize must be a power of two, got 48"
        );
        assert_eq!(
            err(UnwrapOptions { max_size: 16, ..OPTIONS }),
            "the chart from triangle 0 is 16×16 texels plus padding, over maxSize 16; lower \
texelsPerMeter"
        );
        assert_eq!(
            err(UnwrapOptions { max_size: 32, ..OPTIONS }),
            "6 charts need more than 32×32 (2400 texels with padding); lower texelsPerMeter or \
raise maxSize"
        );
    }

    /// The design's medsci1 scale: ~26k triangles.
    #[test]
    fn a_level_unwraps_into_2048() {
        let mesh = level(72);
        assert!((25_000..28_000).contains(&mesh.1.len()), "{} triangles", mesh.1.len());
        let options = UnwrapOptions { texels_per_meter: 8.0, padding: 1, max_size: 2048 };
        let start = std::time::Instant::now();
        let u = unwrap(&mesh.0, &mesh.1, options).unwrap();
        eprintln!(
            "unwrapped {} triangles into {} charts, {}² atlas, in {:?}",
            mesh.1.len(),
            u.charts.len(),
            u.size,
            start.elapsed()
        );
        assert_eq!(u.charts.len(), 72 * (6 + 6 * 6));
        check(&mesh, options);
    }
}
