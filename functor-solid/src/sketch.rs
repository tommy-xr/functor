//! 2D profiles in the XY plane (millimetres): primitives, booleans, offset.
//!
//! A [`Sketch`] is evaluated eagerly — 2D booleans are cheap — and always
//! normalized: non-overlapping shapes, outer contours counter-clockwise and
//! holes clockwise. Extruding one ([`crate::Solid::Extrude`]) yields a closed
//! solid.

use std::f64::consts::TAU;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use i_triangle::float::triangulatable::Triangulatable;

use crate::{segments_for, Mesh, CHORD_TOLERANCE};

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
        let n = segments_for(radius);
        let ring = (0..n)
            .map(|i| {
                let a = TAU * i as f64 / n as f64;
                [radius * a.cos(), radius * a.sin()]
            })
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
        // Round joins split arcs into segments of at most L = angle × R; pick
        // the angle that keeps their sag within the chord tolerance.
        let r = distance.abs();
        let angle = 2.0 * (1.0 - CHORD_TOLERANCE / r).clamp(-1.0, 1.0).acos();
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
                let n = contour.len();
                (0..n)
                    .map(|i| {
                        let ([x0, y0], [x1, y1]) = (contour[i], contour[(i + 1) % n]);
                        x0 * y1 - x1 * y0
                    })
                    .sum::<f64>()
                    / 2.0
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// The closed prism from z = 0 to z = `height`. Walls are built from the
    /// cap triangulation's own boundary edges, so caps and walls share every
    /// vertex and the mesh is closed by construction.
    pub fn extrude(&self, height: f64) -> Mesh {
        let tri = self.shapes.triangulate().to_triangulation::<u32>();
        let n = tri.points.len() as u32;
        let mut mesh = Mesh::default();
        mesh.vertices
            .extend(tri.points.iter().map(|p| [p[0], p[1], 0.0]));
        mesh.vertices
            .extend(tri.points.iter().map(|p| [p[0], p[1], height]));
        // Undirected edge → its directed occurrence, if seen once (boundary).
        let mut edges = std::collections::HashMap::new();
        for t in tri.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]];
            // Wind every cap triangle counter-clockwise seen from +Z.
            let [pa, pb, pc] = [a, b, c].map(|i| tri.points[i as usize]);
            let ccw = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]) > 0.0;
            let [a, b, c] = if ccw { [a, b, c] } else { [a, c, b] };
            mesh.triangles.push([n + a, n + b, n + c]);
            mesh.triangles.push([a, c, b]);
            for (u, v) in [(a, b), (b, c), (c, a)] {
                let key = (u.min(v), u.max(v));
                if edges.remove(&key).is_none() {
                    edges.insert(key, (u, v));
                }
            }
        }
        // Material lies left of each boundary edge u→v, so this wall faces out.
        for (u, v) in edges.into_values() {
            mesh.triangles.push([u, v, n + v]);
            mesh.triangles.push([u, n + v, n + u]);
        }
        mesh
    }
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
