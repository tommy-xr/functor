//! Convex hulls and ball offsets (grow, shrink, round) via `manifold-rust`,
//! on the same inputs as the booleans in `csg`.

use std::sync::Arc;

use cgmath::{Matrix4, SquareMatrix};
use manifold_rust::linalg::Vec3;
use manifold_rust::manifold::Manifold;

use crate::csg::{from_manifold, node, to_manifold, SNAP};
use crate::{mesh, segments_for, Mesh, Solid};

/// The convex hull of `children`, in their shared frame.
pub(crate) fn hull(children: &[Arc<Solid>]) -> Result<Mesh, String> {
    let parts = children
        .iter()
        .map(|c| evaluated(c))
        .collect::<Result<Vec<_>, _>>()?;
    from_manifold(&Manifold::hull_manifolds(&parts))
}

/// `child` shrunk by `shrink` mm, then grown by `grow` mm (either may be 0).
pub(crate) fn morph(child: &Solid, shrink: f64, grow: f64) -> Result<Mesh, String> {
    let mut m = convexified(child)?;
    if shrink > 0.0 {
        let ball = ball(shrink)?;
        let whole = m;
        m = whole
            .try_convex_erosion(&ball, None, None)
            .or_else(|| whole.try_erode_by_convex(&ball, None, None))
            .unwrap_or_else(|| whole.minkowski_difference(&ball));
        if m.is_empty() && !whole.is_empty() {
            return Err(vanished(&whole, shrink, grow > 0.0));
        }
    }
    if grow > 0.0 {
        // Erosion leaves many coplanar slivers; merging them first makes a
        // round's dilation ~5× faster.
        if shrink > 0.0 {
            m = m.simplify(SNAP);
        }
        let ball = ball(grow)?;
        // Non-convex: one hull per triangle, unioned as a balanced tree;
        // convex (declined here): one hull of pairwise vertex sums.
        m = m
            .try_dilate_by_convex(&ball, None, None)
            .unwrap_or_else(|| m.minkowski_sum(&ball));
    }
    from_manifold(&m.simplify(SNAP))
}

/// A solid as one Manifold, in its own frame (a group is its pieces' union).
fn evaluated(solid: &Solid) -> Result<Manifold, String> {
    Ok(Manifold::from_impl(
        node(solid, Matrix4::identity())?.evaluate(),
    ))
}

/// `child` as one Manifold, as its exact hull when convex but for snapping:
/// snapped split quads (on a sphere, a rounded edge) sit a hair off flat,
/// which sends the kernel down its per-triangle paths, ~100–1000× slower.
fn convexified(child: &Solid) -> Result<Manifold, String> {
    let m = evaluated(child)?;
    let hull = m.convex_hull();
    // Snapping moves a vertex at most SNAP: dents no deeper than that.
    if hull.volume() - m.volume() > SNAP * hull.surface_area() {
        return Ok(m);
    }
    let points = child.mesh()?.vertices;
    let points: Vec<_> = points.iter().map(|&[x, y, z]| Vec3::new(x, y, z)).collect();
    Ok(Manifold::hull(&points))
}

/// A sphere of `radius` faceted like `Solid.sphere`: vertices on the axes, so
/// flat axis-aligned faces move by exactly `radius`. Hulled, so the
/// Minkowski fast paths see it as convex (its split quads may not be exactly
/// flat).
fn ball(radius: f64) -> Result<Manifold, String> {
    Ok(to_manifold(&mesh::sphere(radius, segments_for(radius)))?.convex_hull())
}

/// Why shrinking `whole` by `depth` left nothing: an inscribed ball of that
/// radius fits nowhere. Names the deepest that fits for a convex solid.
fn vanished(whole: &Manifold, depth: f64, round: bool) -> String {
    let what = if round {
        format!("Solid.round({depth}) removes the whole part")
    } else {
        format!("Solid.offset({}) shrinks the part to nothing", -depth)
    };
    let limit = deepest(whole, depth)
        .map(|d| format!(" — at most {d:.2} mm fits"))
        .unwrap_or_default();
    format!("{what}: it is nowhere {} mm thick{limit}", 2.0 * depth)
}

/// For a convex solid, about the deepest shrink (mm, rounded down to 0.01)
/// that leaves something: a bisection over the closed-form erosion.
fn deepest(whole: &Manifold, depth: f64) -> Option<f64> {
    if !whole.as_impl().is_convex() {
        return None;
    }
    let fits = |r: f64| {
        ball(r).is_ok_and(|b| {
            whole
                .try_convex_erosion(&b, None, None)
                .is_some_and(|m| !m.is_empty())
        })
    };
    let (mut lo, mut hi) = (0.0, depth);
    while hi - lo > 0.002 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo * 100.0).floor() / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sketch;
    use std::f64::consts::PI;
    use std::time::Instant;

    fn arc(s: Solid) -> Arc<Solid> {
        Arc::new(s)
    }

    fn cube(size: f64) -> Arc<Solid> {
        arc(Solid::Box { size: [size; 3] })
    }

    /// Closed, shared-position-free, and within `tol` (relative) of `want`.
    fn near(solid: &Solid, want: f64, tol: f64) -> Mesh {
        let mesh = solid.mesh().unwrap();
        assert!(mesh.print_problems("it").is_empty(), "{:?}", mesh.print_problems("it"));
        let got = mesh.volume();
        assert!((got / want - 1.0).abs() < tol, "volume {got}, want {want}");
        mesh
    }

    #[test]
    fn hull_of_two_spheres_is_a_capsule() {
        let (r, l) = (5.0, 20.0);
        let ball = arc(Solid::Sphere { radius: r });
        let far = ball.clone().translate([l, 0.0, 0.0]);
        let capsule = Solid::hull(vec![ball, arc(far)]);
        near(&capsule, PI * r * r * l + 4.0 / 3.0 * PI * r.powi(3), 0.005);
    }

    /// A cube of side `a` grown by `r`: faces, quarter-cylinder edges, and
    /// eighth-sphere corners.
    fn grown_cube(a: f64, r: f64) -> f64 {
        a.powi(3) + 6.0 * a * a * r + 3.0 * PI * r * r * a + 4.0 / 3.0 * PI * r.powi(3)
    }

    #[test]
    fn offset_grows_and_shrinks_a_cube() {
        let grown = near(&cube(10.0).offset(1.0), grown_cube(10.0, 1.0), 0.005);
        let b = grown.bounds().unwrap();
        assert_eq!((b.min, b.max), ([-6.0; 3], [6.0; 3]));
        // Shrinking a convex solid is exact: the faces move in, corners stay sharp.
        let shrunk = cube(10.0).offset(-1.0).mesh().unwrap();
        assert!((shrunk.volume() - 512.0).abs() < 1e-9, "{}", shrunk.volume());
    }

    #[test]
    fn round_rounds_a_cube_in_place() {
        let (s, r) = (10.0, 1.0);
        let mesh = near(&cube(s).rounded(r), grown_cube(s - 2.0 * r, r), 0.005);
        let b = mesh.bounds().unwrap();
        assert_eq!((b.min, b.max), ([-5.0; 3], [5.0; 3]));
    }

    /// A 20 × 20 mm L of 6 mm arms, 6 mm tall: concave.
    fn l_shape() -> Arc<Solid> {
        let outline = vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 6.0],
            [6.0, 6.0],
            [6.0, 20.0],
            [0.0, 20.0],
        ];
        arc(Solid::Extrude {
            sketch: Arc::new(Sketch::polygon(outline)),
            height: 6.0,
        })
    }

    #[test]
    fn rounding_follows_a_concave_outline() {
        // An L: rounding keeps its inner corner sharp and loses a little volume.
        let solid = l_shape();
        let before = solid.mesh().unwrap().volume();
        let rounded = solid.clone().rounded(1.0).mesh().unwrap();
        assert!(rounded.print_problems("it").is_empty());
        assert!(rounded.volume() < before && rounded.volume() > 0.9 * before);
        let b = rounded.bounds().unwrap();
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9);
        assert!(close(b.min, [0.0; 3]) && close(b.max, [20.0, 20.0, 6.0]), "{b:?}");
    }

    #[test]
    fn shrinking_away_names_the_limit() {
        let err = cube(10.0).offset(-6.0).mesh().unwrap_err();
        assert_eq!(
            err,
            "Solid.offset(-6) shrinks the part to nothing: it is nowhere 12 mm thick \
— at most 4.99 mm fits"
        );
        let err = arc(Solid::Box { size: [20.0, 20.0, 4.0] })
            .rounded(2.5)
            .mesh()
            .unwrap_err();
        assert!(err.starts_with("Solid.round(2.5) removes the whole part"), "{err}");
        assert!(err.ends_with("at most 1.99 mm fits"), "{err}");
    }

    /// Not a gate (timings vary by machine): run with `--ignored --nocapture`
    /// in release to see the cost of each path.
    #[test]
    #[ignore]
    fn timings() {
        let l = l_shape();
        let disc = arc(Solid::Extrude {
            sketch: Arc::new(Sketch::circle(20.0)),
            height: 6.0,
        });
        let holed = arc(Solid::Extrude {
            sketch: Arc::new(Sketch::rect(40.0, 30.0).subtract(&Sketch::circle(8.0))),
            height: 6.0,
        });
        let cases: Vec<(&str, Arc<Solid>)> = vec![
            ("cube 10", cube(10.0)),
            ("sphere 10", arc(Solid::Sphere { radius: 10.0 })),
            ("concave L", l),
            ("disc r20", disc),
            ("holed plate", holed),
        ];
        for (name, s) in cases {
            let tris = s.mesh().unwrap().triangles.len();
            for (op, solid) in [
                ("offset +1", Solid::offset(s.clone(), 1.0)),
                ("offset -1", Solid::offset(s.clone(), -1.0)),
                ("round 1", Solid::rounded(s.clone(), 1.0)),
            ] {
                let t = Instant::now();
                let mesh = solid.mesh().unwrap();
                eprintln!(
                    "{name} ({tris} tris) {op}: {:.1} ms, {} tris",
                    t.elapsed().as_secs_f64() * 1e3,
                    mesh.triangles.len()
                );
            }
        }
    }
}
