//! 2D profiles in the XY plane (millimetres): primitives, booleans, offset.
//!
//! A [`Sketch`] is evaluated eagerly — 2D booleans are cheap — and always
//! normalized: non-overlapping shapes, outer contours counter-clockwise and
//! holes clockwise. Extruding one ([`crate::Solid::Extrude`]) yields a closed
//! solid.

use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use i_triangle::float::triangulatable::Triangulatable;

use crate::{segments_for, Mesh};

/// Shapes → contours → points: each shape is an outer contour plus holes.
pub type Shapes = Vec<Vec<Vec<[f64; 2]>>>;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sketch {
    pub shapes: Shapes,
}

impl Sketch {
    /// A `width` × `height` rectangle centered on the origin.
    pub fn rect(width: f64, height: f64) -> Sketch {
        let (x, y) = (width / 2.0, height / 2.0);
        Sketch {
            shapes: vec![vec![vec![[-x, -y], [x, -y], [x, y], [-x, y]]]],
        }
    }

    pub fn circle(radius: f64) -> Sketch {
        let ring = crate::mesh::ring(radius, 0.0, segments_for(radius))
            .map(|[x, y, _]| [x, y])
            .collect();
        Sketch {
            shapes: vec![vec![ring]],
        }
    }

    /// A filled outline, in any winding; self-intersections fill by nonzero
    /// winding.
    pub fn polygon(points: Vec<[f64; 2]>) -> Sketch {
        Sketch {
            shapes: vec![vec![points]].simplify_shape(FillRule::NonZero),
        }
    }

    pub fn union(sketches: &[Sketch]) -> Sketch {
        let all: Shapes = sketches.iter().flat_map(|s| s.shapes.clone()).collect();
        Sketch {
            shapes: all.simplify_shape(FillRule::NonZero),
        }
    }

    pub fn subtract(&self, cut: &Sketch) -> Sketch {
        self.overlay(cut, OverlayRule::Difference)
    }

    pub fn intersect(&self, other: &Sketch) -> Sketch {
        self.overlay(other, OverlayRule::Intersect)
    }

    fn overlay(&self, other: &Sketch, rule: OverlayRule) -> Sketch {
        Sketch {
            shapes: self.shapes.overlay(&other.shapes, rule, FillRule::NonZero),
        }
    }

    /// Grow (positive) or shrink (negative) by `distance`, rounding corners on
    /// growth. Shrinking past a feature's half-width removes it.
    pub fn offset(&self, distance: f64) -> Sketch {
        if distance == 0.0 || self.shapes.is_empty() {
            return self.clone();
        }
        // Round joins step by `angle` (L / R): match `circle`'s faceting. i_overlay
        // floors the step at 0.01π, so offsets past ~80 mm sag more than the
        // chord tolerance.
        let angle = TAU / segments_for(distance.abs()) as f64;
        let style = OutlineStyle::new(distance).line_join(LineJoin::Round(angle));
        Sketch {
            shapes: self.shapes.outline(&style),
        }
    }

    pub fn map_points(&self, f: impl Fn([f64; 2]) -> [f64; 2]) -> Sketch {
        Sketch {
            shapes: self
                .shapes
                .iter()
                .map(|shape| {
                    shape
                        .iter()
                        .map(|contour| contour.iter().map(|&p| f(p)).collect())
                        .collect()
                })
                .collect(),
        }
    }

    /// Filled area in mm² (holes subtract).
    pub fn area(&self) -> f64 {
        self.shapes
            .iter()
            .flatten()
            .map(|contour| {
                // Shoelace about the contour's first point, so far-from-origin
                // contours don't lose their area to cancellation.
                let n = contour.len();
                let [ox, oy] = contour[0];
                (0..n)
                    .map(|i| {
                        let ([x0, y0], [x1, y1]) = (contour[i], contour[(i + 1) % n]);
                        (x0 - ox) * (y1 - oy) - (x1 - ox) * (y0 - oy)
                    })
                    .sum::<f64>()
                    / 2.0
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// A point where regions meet only at a vertex (a bowtie, squares corner
    /// to corner): two contour vertices at one position. Extruded, such a
    /// point becomes an edge shared by four faces, which won't slice as a solid.
    pub fn touching_point(&self) -> Option<[f64; 2]> {
        let mut seen = HashSet::new();
        self.shapes
            .iter()
            .flatten()
            .flatten()
            .find(|p| !seen.insert(p.map(|x| (x + 0.0).to_bits())))
            .copied()
    }

    /// The prism from z = 0 to z = `height`. Walls are built from the cap
    /// triangulation's own boundary edges, so caps and walls share every
    /// vertex; it is closed whenever [`Sketch::touching_point`] is `None`.
    pub fn extrude(&self, height: f64) -> Mesh {
        let tri = self.shapes.triangulate().to_triangulation::<u32>();
        let (points, remap) = weld(&tri.points);
        let n = points.len() as u32;
        let mut mesh = Mesh::default();
        mesh.vertices
            .extend(points.iter().map(|p| [p[0], p[1], 0.0]));
        mesh.vertices
            .extend(points.iter().map(|p| [p[0], p[1], height]));
        // i_triangle emits counter-clockwise triangles (seen from +Z).
        let triangles: Vec<[u32; 3]> = tri
            .indices
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]].map(|i| remap[i as usize]))
            .filter(|&[a, b, c]| a != b && b != c && c != a)
            .collect();
        let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
        for &[a, b, c] in &triangles {
            mesh.triangles.push([n + a, n + b, n + c]);
            mesh.triangles.push([a, c, b]);
            for (u, v) in [(a, b), (b, c), (c, a)] {
                *uses.entry((u.min(v), u.max(v))).or_default() += 1;
            }
        }
        // Boundary edges (used once), visited in triangle order so the mesh is
        // deterministic. Material lies left of u→v, so each wall faces out.
        for &[a, b, c] in &triangles {
            for (u, v) in [(a, b), (b, c), (c, a)] {
                if uses[&(u.min(v), u.max(v))] == 1 {
                    mesh.triangles.push([u, v, n + v]);
                    mesh.triangles.push([u, n + v, n + u]);
                }
            }
        }
        mesh
    }
}

/// Points closer than this (mm) are one point: far below any printable
/// feature, yet above f32 resolution for parts under ~1 m, so exported files
/// never carry two copies of one position.
pub(crate) const WELD_TOLERANCE: f64 = 1e-4;

/// Merge points within [`WELD_TOLERANCE`] — an offset can leave neighbours a
/// few nanometres apart. Returns the kept points and each input's new index.
fn weld(input: &[[f64; 2]]) -> (Vec<[f64; 2]>, Vec<u32>) {
    let cell = |p: [f64; 2]| p.map(|x| (x / WELD_TOLERANCE).floor() as i64);
    let mut grid: HashMap<[i64; 2], Vec<u32>> = HashMap::new();
    let mut points: Vec<[f64; 2]> = Vec::new();
    let remap = input
        .iter()
        .map(|&p| {
            let [cx, cy] = cell(p);
            let near = (-1..=1)
                .flat_map(|dx| (-1..=1).map(move |dy| [cx + dx, cy + dy]))
                .filter_map(|c| grid.get(&c))
                .flatten()
                .find(|&&i| {
                    let q = points[i as usize];
                    (q[0] - p[0]).hypot(q[1] - p[1]) <= WELD_TOLERANCE
                })
                .copied();
            near.unwrap_or_else(|| {
                points.push(p);
                let i = points.len() as u32 - 1;
                grid.entry([cx, cy]).or_default().push(i);
                i
            })
        })
        .collect();
    (points, remap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs()
    }

    #[test]
    fn primitives_have_exact_or_near_analytic_area() {
        assert!(close(Sketch::rect(4.0, 3.0).area(), 12.0, 1e-12));
        assert!(close(Sketch::circle(5.0).area(), PI * 25.0, 0.005));
        let tri = Sketch::polygon(vec![[0.0, 0.0], [0.0, 2.0], [2.0, 0.0]]);
        assert!(close(tri.area(), 2.0, 1e-9), "clockwise input normalizes");
    }

    #[test]
    fn booleans_combine_areas() {
        let a = Sketch::rect(4.0, 4.0);
        let b = Sketch::rect(4.0, 4.0).map_points(|[x, y]| [x + 2.0, y]);
        assert!(close(
            Sketch::union(&[a.clone(), b.clone()]).area(),
            24.0,
            1e-9
        ));
        assert!(close(a.intersect(&b).area(), 8.0, 1e-9));
        let ring = Sketch::rect(10.0, 10.0).subtract(&Sketch::rect(4.0, 4.0));
        assert!(close(ring.area(), 84.0, 1e-9));
        assert_eq!(ring.shapes[0].len(), 2, "outer contour + hole");
    }

    #[test]
    fn offset_grows_by_perimeter_and_rounds_corners() {
        // Rounded growth of a w×h rect by d: wh + 2(w+h)d + πd².
        let grown = Sketch::rect(10.0, 6.0).offset(1.0);
        assert!(
            close(grown.area(), 60.0 + 32.0 + PI, 0.002),
            "{}",
            grown.area()
        );
        let shrunk = Sketch::rect(10.0, 6.0).offset(-1.0);
        assert!(close(shrunk.area(), 8.0 * 4.0, 0.002), "{}", shrunk.area());
        assert!(Sketch::rect(2.0, 2.0).offset(-1.5).is_empty());
    }

    #[test]
    fn regions_meeting_at_a_point_are_detected() {
        let bowtie = Sketch::polygon(vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]]);
        assert!(bowtie.touching_point().is_some());
        let corner = Sketch::union(&[
            Sketch::rect(2.0, 2.0),
            Sketch::rect(2.0, 2.0).map_points(|[x, y]| [x + 2.0, y + 2.0]),
        ]);
        assert_eq!(corner.touching_point(), Some([1.0, 1.0]));
        let notched = Sketch::rect(10.0, 10.0).subtract(&Sketch::circle(5.0));
        assert!(notched.touching_point().is_some());
        assert_eq!(
            Sketch::rect(10.0, 10.0)
                .subtract(&Sketch::circle(3.0))
                .touching_point(),
            None
        );
    }

    #[test]
    fn extrusion_is_deterministic() {
        let s = Sketch::rect(10.0, 10.0).subtract(&Sketch::circle(3.0));
        assert_eq!(s.extrude(1.0), s.extrude(1.0));
    }

    #[test]
    fn weld_merges_only_points_within_tolerance() {
        let (points, remap) = weld(&[[0.0, 0.0], [5e-5, 0.0], [2e-4, 0.0], [10.0, 0.0]]);
        assert_eq!(points.len(), 3);
        assert_eq!(remap, vec![0, 0, 1, 2]);
        // Far from the origin, real geometry is untouched.
        let far = Sketch::rect(2.0, 2.0).map_points(|[x, y]| [x + 1e8, y]);
        assert!((far.extrude(1.0).volume() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn area_survives_far_from_origin() {
        let far = Sketch::rect(10.0, 6.0).map_points(|[x, y]| [x + 1e9, y + 1e9]);
        assert!(close(far.area(), 60.0, 1e-9), "{}", far.area());
    }

    #[test]
    fn extrusions_are_closed_with_volume_area_times_height() {
        for sketch in [
            Sketch::rect(10.0, 6.0),
            Sketch::circle(5.0),
            Sketch::rect(10.0, 10.0).subtract(&Sketch::circle(3.0)),
            Sketch::union(&[
                Sketch::rect(4.0, 4.0),
                Sketch::rect(4.0, 4.0).map_points(|[x, y]| [x + 10.0, y]),
            ]),
            Sketch::rect(10.0, 6.0).offset(1.5),
        ] {
            let mesh = sketch.extrude(2.0);
            assert_eq!(mesh.non_manifold_edges(), 0, "{sketch:?}");
            assert_eq!(mesh.shared_positions(), 0);
            assert!(
                close(mesh.volume(), sketch.area() * 2.0, 1e-9),
                "{}",
                mesh.volume()
            );
        }
    }
}
