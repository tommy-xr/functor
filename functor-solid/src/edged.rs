//! Extrusions whose top and bottom edges are bevelled (45°) or rounded all
//! round the outline — exactly, not stepped.
//!
//! At depth `d` in from the wall, an edge band's cross-section is the sketch
//! shrunk by `d`, as `Sketch::offset(-d)` draws it: convex corners stay sharp,
//! concave ones round. The band is swept as a 2D wavefront: each edge slides
//! inward parallel to itself, each convex corner rides where its two edges
//! meet, and a concave corner fans out into an arc. Between levels every face
//! is then a flat trapezoid — exact for a bevel, a faceted quarter circle for a
//! round. An edge that shrinks to nothing merges its corners (a rounded corner
//! turning sharp); a contour that would split or vanish is an error.
//!
//! Chosen over a Minkowski sum with a cone (rounds sharp convex corners, and
//! ~170 ms for a concave L) and booleans of offset slabs (stepped): it needs
//! no booleans, so it is sub-millisecond and leaves nothing near-coincident.

use std::f64::consts::{FRAC_PI_2, TAU};

use manifold_rust::linalg::Vec2;
use manifold_rust::polygon::triangulate_idx;
use manifold_rust::types::PolyVert;

use crate::sketch::{contour_area, WELD_TOLERANCE};
use crate::{segments_for, Mesh, Sketch, CHORD_TOLERANCE};

/// How an extrusion's top or bottom edge is finished.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeShape {
    Sharp,
    /// A 45° chamfer reaching this far (mm) down the wall and in from it.
    Bevel(f64),
    /// A quarter-circle fillet of this radius (mm).
    Round(f64),
}

impl EdgeShape {
    /// How far (mm) the edge reaches down the wall, and in from it.
    pub fn size(self) -> f64 {
        match self {
            EdgeShape::Sharp => 0.0,
            EdgeShape::Bevel(s) | EdgeShape::Round(s) => s,
        }
    }

    /// `[depth in from the wall, rise from the wall's end]`, wall to cap.
    fn profile(self) -> Vec<P> {
        match self {
            EdgeShape::Sharp => vec![[0.0, 0.0]],
            EdgeShape::Bevel(b) => vec![[0.0, 0.0], [b, b]],
            // Faceted like a circle of radius r: a quarter of its segments.
            EdgeShape::Round(r) => {
                let k = segments_for(r) / 4;
                (0..=k)
                    .map(|j| match j {
                        _ if j == k => [r, r],
                        _ => {
                            let a = FRAC_PI_2 * j as f64 / k as f64;
                            [r * (1.0 - a.cos()), r * a.sin()]
                        }
                    })
                    .collect()
            }
        }
    }
}

impl std::fmt::Display for EdgeShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EdgeShape::Sharp => write!(f, "Solid.Sharp"),
            EdgeShape::Bevel(s) => write!(f, "Solid.Bevel({s})"),
            EdgeShape::Round(s) => write!(f, "Solid.Round({s})"),
        }
    }
}

/// Events within this depth (mm) of a level happen at it, so levels stay far
/// enough apart that no two vertices share an f32 position.
const EPS: f64 = WELD_TOLERANCE;

type P = [f64; 2];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn rotate(a: P, angle: f64) -> P {
    let (s, c) = angle.sin_cos();
    [a[0] * c - a[1] * s, a[0] * s + a[1] * c]
}

/// The point `x` with `a·x = ca` and `b·x = cb`; `None` when the lines are
/// (nearly) parallel.
fn meet(a: P, ca: f64, b: P, cb: f64) -> Option<P> {
    let det = cross(a, b);
    (det.abs() > 1e-9).then(|| [(ca * b[1] - cb * a[1]) / det, (cb * a[0] - ca * b[0]) / det])
}

/// A wavefront corner: position, velocity per mm of depth, mesh vertex.
#[derive(Clone, Copy)]
struct Node {
    p: P,
    v: P,
    id: u32,
}

/// An edge's line: it slides along its inward unit normal `n` at `speed` mm
/// per mm of depth (1, or less for a chord of a concave corner's arc).
#[derive(Clone, Copy)]
struct Line {
    n: P,
    speed: f64,
}

/// One contour's wavefront: `lines[i]` runs from `nodes[i]` to the next
/// node, material on its left.
struct Front {
    nodes: Vec<Node>,
    lines: Vec<Line>,
}

/// How deep the inset failed, and where if known.
struct Stuck {
    depth: f64,
    at: Option<P>,
}

impl Front {
    /// A contour's wavefront at the wall; `ids` are its corners' vertices. A
    /// concave corner fans into nodes that all start at the corner — unless,
    /// down to `depth`, a sharp mitre stays within the chord tolerance.
    fn new(points: &[P], ids: &[u32], depth: f64) -> Front {
        let m = points.len();
        let normal = |i: usize| {
            let e = sub(points[(i + 1) % m], points[i]);
            let len = dot(e, e).sqrt();
            [-e[1] / len, e[0] / len]
        };
        let (mut nodes, mut lines) = (Vec::new(), Vec::new());
        for i in 0..m {
            let (a, b) = (normal((i + m - 1) % m), normal(i));
            // Positive turning left: a convex corner.
            let turn = cross(a, b).atan2(dot(a, b));
            let sag = depth * (1.0 / (turn / 2.0).cos() - 1.0);
            let k = if turn >= 0.0 || sag <= CHORD_TOLERANCE {
                0
            } else {
                (-turn / (TAU / segments_for(depth) as f64)).ceil() as usize
            };
            let node = |v| Node {
                p: points[i],
                v,
                id: ids[i],
            };
            if k == 0 {
                let v = meet(a, 1.0, b, 1.0).unwrap_or(a);
                nodes.push(node(v));
            } else {
                for j in 0..=k {
                    if j > 0 {
                        lines.push(Line {
                            n: rotate(a, turn * (j as f64 - 0.5) / k as f64),
                            speed: (turn / (2.0 * k as f64)).cos(),
                        });
                    }
                    nodes.push(node(rotate(a, turn * j as f64 / k as f64)));
                }
            }
            lines.push(Line { n: b, speed: 1.0 });
        }
        Front { nodes, lines }
    }

    /// Depth until line `i` shrinks to nothing (infinite if it doesn't).
    fn collapse(&self, i: usize) -> f64 {
        let (a, b) = (self.nodes[i], self.nodes[(i + 1) % self.nodes.len()]);
        let n = self.lines[i].n;
        let e = [n[1], -n[0]];
        let rate = dot(sub(a.v, b.v), e);
        if rate > 1e-12 {
            (dot(sub(b.p, a.p), e) / rate).max(0.0)
        } else {
            f64::INFINITY
        }
    }

    fn advance(&mut self, dt: f64) {
        for n in &mut self.nodes {
            n.p = [n.p[0] + n.v[0] * dt, n.p[1] + n.v[1] * dt];
        }
    }

    /// Emit this front as a level of vertices at height `z`, joined to the
    /// previous level by faces; lines about to vanish merge their corners.
    /// `up`: this level is above the previous one.
    fn level(&mut self, depth: f64, z: f64, up: bool, mesh: &mut Mesh) -> Result<(), Stuck> {
        let n = self.nodes.len();
        let gone: Vec<bool> = (0..n).map(|i| self.collapse(i) <= EPS).collect();
        if gone.iter().filter(|g| !**g).count() < 3 {
            let mean = |k: usize| self.nodes.iter().map(|n| n.p[k]).sum::<f64>() / n as f64;
            return Err(Stuck {
                depth,
                at: Some([mean(0), mean(1)]),
            });
        }
        // Clusters of nodes joined by vanishing lines; start where one begins.
        let start = (0..n).find(|&i| !gone[(i + n - 1) % n]).unwrap();
        let (mut nodes, mut lines) = (Vec::new(), Vec::new());
        let mut cluster = vec![0; n];
        let mut first = start;
        loop {
            let mut last = first;
            while gone[last] {
                last = (last + 1) % n;
            }
            let (p, v) = if first == last {
                (self.nodes[first].p, self.nodes[first].v)
            } else {
                // The merged corner rides where the lines on either side meet.
                let (a, b) = (self.lines[(first + n - 1) % n], self.lines[last]);
                let (pa, pb) = (self.nodes[first].p, self.nodes[last].p);
                match (
                    meet(a.n, dot(a.n, pa), b.n, dot(b.n, pb)),
                    meet(a.n, a.speed, b.n, b.speed),
                ) {
                    (Some(p), Some(v)) => (p, v),
                    // Collinear: a notch closed up.
                    _ if dot(a.n, b.n) > 0.0 => (
                        [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0],
                        [a.n[0] * a.speed, a.n[1] * a.speed],
                    ),
                    // Opposite sides met: the contour pinched off.
                    _ => {
                        return Err(Stuck {
                            depth,
                            at: Some(pa),
                        })
                    }
                }
            };
            let mut i = first;
            loop {
                cluster[i] = nodes.len();
                if i == last {
                    break;
                }
                i = (i + 1) % n;
            }
            mesh.vertices.push([p[0], p[1], z]);
            nodes.push(Node {
                p,
                v,
                id: mesh.vertices.len() as u32 - 1,
            });
            lines.push(self.lines[last]);
            first = (last + 1) % n;
            if first == start {
                break;
            }
        }
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, b) = (self.nodes[i].id, self.nodes[j].id);
            let (c, d) = (nodes[cluster[i]].id, nodes[cluster[j]].id);
            if up {
                quad(mesh, [a, b], [c, d]);
            } else {
                quad(mesh, [c, d], [a, b]);
            }
        }
        self.nodes = nodes;
        self.lines = lines;
        Ok(())
    }
}

/// The wall-like face between edge `lo` and the edge `hi` above it (each
/// `[from, to]`, material on the left), dropping triangles a merged corner
/// or a fan's shared start makes degenerate.
fn quad(mesh: &mut Mesh, [a, b]: [u32; 2], [c, d]: [u32; 2]) {
    for t in [[a, b, d], [a, d, c]] {
        if t[0] != t[1] && t[1] != t[2] && t[2] != t[0] {
            mesh.triangles.push(t);
        }
    }
}

/// A point where two non-adjacent segments of the fronts cross or touch, to
/// say where an inset failed.
fn crossing(fronts: &[Front]) -> Option<P> {
    let segments: Vec<(usize, usize, usize, P, P)> = fronts
        .iter()
        .enumerate()
        .flat_map(|(f, front)| {
            let n = front.nodes.len();
            (0..n).map(move |i| (f, i, n, front.nodes[i].p, front.nodes[(i + 1) % n].p))
        })
        .collect();
    let side = |a: P, b: P, c: P| cross(sub(b, a), sub(c, a));
    for (k, &(f, i, n, a, b)) in segments.iter().enumerate() {
        for &(g, j, _, c, d) in &segments[k + 1..] {
            if f == g && (j == (i + 1) % n || i == (j + 1) % n) {
                continue;
            }
            let apart =
                |k: usize| a[k].max(b[k]) < c[k].min(d[k]) || c[k].max(d[k]) < a[k].min(b[k]);
            if apart(0) || apart(1) {
                continue;
            }
            if side(a, b, c) * side(a, b, d) <= 0.0 && side(c, d, a) * side(c, d, b) <= 0.0 {
                return Some([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
            }
        }
    }
    None
}

/// Sweep `shape`'s band in from the wall's `contours` (vertices `ids`) of
/// `reference` to its cap, adding faces to `mesh`; `z(rise)` places a level.
/// Returns the cap's fronts.
fn band(
    reference: &Sketch,
    contours: &[Vec<P>],
    ids: &[Vec<u32>],
    shape: EdgeShape,
    z: &dyn Fn(f64) -> f64,
    mesh: &mut Mesh,
) -> Result<Vec<Front>, Stuck> {
    let up = z(1.0) > z(0.0);
    let mut fronts: Vec<Front> = contours
        .iter()
        .zip(ids)
        .map(|(c, ids)| Front::new(c, ids, shape.size()))
        .collect();
    let mut t = 0.0;
    for w in shape.profile().windows(2) {
        let ([d0, r0], [d1, r1]) = (w[0], w[1]);
        while t < d1 {
            // Step to the next corner merge, or to the profile's next point.
            let next = fronts
                .iter()
                .flat_map(|f| (0..f.nodes.len()).map(|i| f.collapse(i)))
                .fold(f64::INFINITY, f64::min)
                + t;
            let to = if next < d1 - EPS {
                next.max(t + EPS)
            } else {
                d1
            };
            for f in &mut fronts {
                f.advance(to - t);
            }
            t = to;
            let height = z(r0 + (r1 - r0) * (t - d0) / (d1 - d0));
            for f in &mut fronts {
                f.level(t, height, up, mesh)?;
            }
        }
    }
    if shape == EdgeShape::Sharp {
        return Ok(fronts);
    }
    // The wavefront can't change topology, so a split, a vanished piece, or a
    // hole that swallowed its outline leaves it disagreeing with the true
    // inset in contour count or area. Compare against `Sketch::offset`.
    let truth = reference.offset(-t);
    let area: f64 = fronts
        .iter()
        .map(|f| contour_area(&f.nodes.iter().map(|n| n.p).collect::<Vec<_>>()))
        .sum();
    let contours = truth.shapes.iter().flatten().count();
    if contours != fronts.len() || (area - truth.area()).abs() > 0.01 * truth.area().abs() + 1e-3 {
        return Err(Stuck {
            depth: t,
            at: crossing(&fronts),
        });
    }
    Ok(fronts)
}

/// Triangles over the fronts' interior, facing up (or down).
fn cap(fronts: &[Front], up: bool, mesh: &mut Mesh) {
    let polys: Vec<Vec<PolyVert>> = fronts
        .iter()
        .map(|f| {
            f.nodes
                .iter()
                .map(|n| PolyVert {
                    pos: Vec2::new(n.p[0], n.p[1]),
                    idx: n.id as i32,
                })
                .collect()
        })
        .collect();
    for t in triangulate_idx(&polys, -1.0, true) {
        let [a, b, c] = [t.x, t.y, t.z].map(|i| i as u32);
        mesh.triangles.push(if up { [a, b, c] } else { [a, c, b] });
    }
}

impl Sketch {
    /// The prism from z = 0 to `height` with its `top` and `bottom` edges
    /// finished all round the outline. Fails with a teaching message when
    /// the edges overlap or the sketch can't shrink that far in one piece.
    pub fn extrude_edged(
        &self,
        height: f64,
        top: EdgeShape,
        bottom: EdgeShape,
    ) -> Result<Mesh, String> {
        for (name, e) in [("top", top), ("bottom", bottom)] {
            if e != EdgeShape::Sharp && !(e.size() > 0.0 && e.size().is_finite()) {
                return Err(format!(
                    "the {name}'s {e} needs a positive size (Solid.Sharp is a plain edge)"
                ));
            }
        }
        let (bt, bb) = (top.size(), bottom.size());
        if bt + bb > height {
            return Err(format!(
                "the top {top} and bottom {bottom} reach {} mm down the wall, more than the \
{height} mm height — together they can reach at most the height",
                bt + bb
            ));
        }
        if top == EdgeShape::Sharp && bottom == EdgeShape::Sharp {
            return Ok(self.extrude(height));
        }
        // Each contour without repeated corners (an offset can leave neighbours
        // nanometres apart).
        let contours: Vec<Vec<P>> = self
            .shapes
            .iter()
            .flatten()
            .map(|c| {
                let mut c = c.clone();
                c.dedup_by(|b, a| (b[0] - a[0]).hypot(b[1] - a[1]) <= WELD_TOLERANCE);
                while c.len() > 1 && {
                    let (a, b) = (c[0], c[c.len() - 1]);
                    (b[0] - a[0]).hypot(b[1] - a[1]) <= WELD_TOLERANCE
                } {
                    c.pop();
                }
                c
            })
            .filter(|c| c.len() >= 3)
            .collect();
        let build = |top: EdgeShape, bottom: EdgeShape| -> Result<Mesh, (EdgeShape, Stuck)> {
            let mut mesh = Mesh::default();
            let ring = |mesh: &mut Mesh, z: f64| -> Vec<Vec<u32>> {
                contours
                    .iter()
                    .map(|c| {
                        c.iter()
                            .map(|p| {
                                mesh.vertices.push([p[0], p[1], z]);
                                mesh.vertices.len() as u32 - 1
                            })
                            .collect()
                    })
                    .collect()
            };
            let (z0, z1) = (bb, height - bt);
            let low = ring(&mut mesh, z0);
            let high = if z1 - z0 > EPS {
                ring(&mut mesh, z1)
            } else {
                low.clone()
            };
            if z1 - z0 > EPS {
                for (lo, hi) in low.iter().zip(&high) {
                    let n = lo.len();
                    for i in 0..n {
                        let j = (i + 1) % n;
                        quad(&mut mesh, [lo[i], lo[j]], [hi[i], hi[j]]);
                    }
                }
            }
            let fronts =
                band(self, &contours, &high, top, &|r| z1 + r, &mut mesh).map_err(|s| (top, s))?;
            cap(&fronts, true, &mut mesh);
            let fronts = band(self, &contours, &low, bottom, &|r| z0 - r, &mut mesh)
                .map_err(|s| (bottom, s))?;
            cap(&fronts, false, &mut mesh);
            Ok(mesh)
        };
        build(top, bottom).map_err(|(edge, stuck)| {
            // The deepest inset that works, to 0.01 mm, to name the limit.
            let fits = |d: f64| build(EdgeShape::Bevel(d), EdgeShape::Sharp).is_ok();
            let (mut lo, mut hi) = (0.0, stuck.depth.max(EPS));
            while hi - lo > 0.005 {
                let mid = (lo + hi) / 2.0;
                if fits(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let side = if edge == top { "top" } else { "bottom" };
            let near = match stuck.at {
                Some([x, y]) => format!("near ({x:.2}, {y:.2}) "),
                None => String::new(),
            };
            format!(
                "the {side} {edge} shrinks the sketch {} mm all round, but {near}its outline \
would split or vanish: at most {:.2} mm fits",
                edge.size(),
                (lo * 100.0).floor() / 100.0,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;
    use EdgeShape::{Bevel, Round, Sharp};

    /// Closed, printable, and its volume within `rel` of `exact`.
    fn check(mesh: &Mesh, exact: f64, rel: f64) {
        assert_eq!(mesh.print_problems("part"), Vec::<String>::new());
        let v = mesh.volume();
        assert!(
            (v - exact).abs() <= rel * exact,
            "volume {v}, expected {exact}"
        );
    }

    /// Volume of the band layers by the midpoint rule over `Sketch::offset`.
    fn layered(s: &Sketch, height: f64, inset: impl Fn(f64) -> f64) -> f64 {
        let n = 400;
        let dz = height / n as f64;
        (0..n)
            .map(|i| s.offset(-inset((i as f64 + 0.5) * dz)).area() * dz)
            .sum()
    }

    fn rounded_rect(w: f64, h: f64, r: f64) -> Sketch {
        Sketch::rect(w - 2.0 * r, h - 2.0 * r).offset(r)
    }

    fn l_shape() -> Sketch {
        Sketch::union(&[
            Sketch::rect(40.0, 10.0),
            Sketch::rect(10.0, 40.0).map_points(|[x, y]| [x - 15.0, y + 15.0]),
        ])
    }

    #[test]
    fn circle_bevel_and_round_match_analytic_volume_and_bounds() {
        let (r, h) = (10.0, 10.0);
        let bevel = Sketch::circle(r)
            .extrude_edged(h, Bevel(2.0), Sharp)
            .unwrap();
        let frustum = PI * 2.0 * (r * r + r * 8.0 + 64.0) / 3.0;
        check(&bevel, PI * r * r * 8.0 + frustum, 0.005);
        let b = bevel.bounds().unwrap();
        assert_eq!((b.min[2], b.max[2]), (0.0, h));
        assert!((b.max[0] - r).abs() < 1e-9 && (b.min[1] + r).abs() < 1e-9);

        let f = 3.0;
        let round = Sketch::circle(r).extrude_edged(h, Round(f), Sharp).unwrap();
        let band = PI * ((r - f).powi(2) * f + (r - f) * PI * f * f / 2.0 + 2.0 * f.powi(3) / 3.0);
        check(&round, PI * r * r * (h - f) + band, 0.005);
    }

    #[test]
    fn rounded_rect_bevels_both_ends_including_corner_radius_equal_to_size() {
        let (w, d, h, b) = (60.0, 40.0, 12.0, 3.0);
        for corner in [5.0, 3.0] {
            let area = |x: f64| (w - 2.0 * x) * (d - 2.0 * x) - (4.0 - PI) * (corner - x).powi(2);
            // Simpson's rule is exact for the quadratic area.
            let band = b / 6.0 * (area(0.0) + 4.0 * area(b / 2.0) + area(b));
            let mesh = rounded_rect(w, d, corner)
                .extrude_edged(h, Bevel(b), Bevel(b))
                .unwrap();
            check(&mesh, area(0.0) * (h - 2.0 * b) + 2.0 * band, 0.005);
            let bb = mesh.bounds().unwrap();
            assert_eq!((bb.min[2], bb.max[2]), (0.0, h));
            assert!((bb.max[0] - w / 2.0).abs() < 1e-6, "{:?}", bb);
        }
    }

    #[test]
    fn concave_outlines_and_holes_are_closed_and_match_layered_offsets() {
        let ring = Sketch::rect(30.0, 30.0).subtract(&Sketch::circle(6.0));
        let pill = Sketch::union(&[
            Sketch::rect(40.0, 30.0),
            Sketch::circle(15.0).map_points(|[x, y]| [x - 20.0, y]),
            Sketch::circle(15.0).map_points(|[x, y]| [x + 20.0, y]),
        ]);
        for s in [l_shape(), ring, pill] {
            let (h, b, r) = (8.0, 2.0, 2.5);
            let mesh = s.extrude_edged(h, Bevel(b), Round(r)).unwrap();
            let bevel = layered(&s, b, |z| z);
            let round = layered(&s, r, |z| r - (r * r - (r - z).powi(2)).sqrt());
            check(&mesh, s.area() * (h - b - r) + bevel + round, 0.005);
        }
    }

    #[test]
    fn edges_may_meet_mid_wall() {
        let mesh = l_shape()
            .extrude_edged(4.0, Round(2.0), Bevel(2.0))
            .unwrap();
        assert_eq!(mesh.print_problems("part"), Vec::<String>::new());
    }

    #[test]
    fn oversized_edges_name_the_limit() {
        let tall = Sketch::circle(5.0).extrude_edged(3.0, Bevel(2.0), Round(1.5));
        assert!(tall
            .unwrap_err()
            .contains("reach 3.5 mm down the wall, more than the 3 mm height"));
        // 4 mm wide: it vanishes 2 mm in.
        let thin = Sketch::rect(20.0, 4.0).extrude_edged(10.0, Bevel(2.5), Sharp);
        let e = thin.unwrap_err();
        assert!(
            e.starts_with("the top Solid.Bevel(2.5) shrinks the sketch 2.5 mm"),
            "{e}"
        );
        assert!(e.ends_with("at most 1.99 mm fits"), "{e}");
        // Two squares joined by a 2 mm neck split 1 mm in, at the neck.
        let dumbbell = Sketch::union(&[
            Sketch::rect(10.0, 10.0).map_points(|[x, y]| [x - 10.0, y]),
            Sketch::rect(10.0, 10.0).map_points(|[x, y]| [x + 10.0, y]),
            Sketch::rect(12.0, 2.0),
        ]);
        let e = dumbbell.extrude_edged(5.0, Sharp, Round(1.5)).unwrap_err();
        assert!(e.starts_with("the bottom Solid.Round(1.5)"), "{e}");
        assert!(e.ends_with("at most 0.99 mm fits"), "{e}");
        let e = Sketch::circle(3.0)
            .extrude_edged(5.0, Bevel(0.0), Sharp)
            .unwrap_err();
        assert!(e.contains("needs a positive size"), "{e}");
    }
}
