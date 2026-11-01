//! Convex hulls and ball offsets (grow, shrink, round) via `manifold-rust`,
//! on the same inputs as the booleans in `csg`.

use std::sync::Arc;

use cgmath::{InnerSpace, Matrix4, SquareMatrix, Vector3};
use manifold_rust::linalg::Vec3;
use manifold_rust::manifold::Manifold;

use crate::csg::{from_manifold, node, SNAP};
use crate::{face_normal, mesh, segments_for, Mesh, Solid};

/// The convex hull of `children`, in their shared frame.
pub(crate) fn hull(children: &[Arc<Solid>]) -> Result<Mesh, String> {
    let mut all = Mesh::default();
    for child in children {
        child.append_to(&mut all, Matrix4::identity())?;
    }
    from_manifold(&hull_of(&all))
}

/// The convex hull of a mesh's vertices.
fn hull_of(mesh: &Mesh) -> Manifold {
    let points: Vec<_> = mesh
        .vertices
        .iter()
        .map(|&[x, y, z]| Vec3::new(x, y, z))
        .collect();
    Manifold::hull(&points)
}

/// `child` shrunk by `shrink` mm, then grown by `grow` mm (either may be 0).
pub(crate) fn morph(child: &Solid, shrink: f64, grow: f64) -> Result<Mesh, String> {
    let mut m = convexified(child)?;
    // Nothing to offset (e.g. an intersection of solids that don't meet).
    if m.is_empty() {
        return from_manifold(&m);
    }
    if shrink > 0.0 {
        let ball = ball(shrink);
        let whole = m;
        m = whole
            .try_convex_erosion(&ball, None, None)
            .or_else(|| whole.try_erode_by_convex(&ball, None, None))
            .unwrap_or_else(|| whole.minkowski_difference(&ball));
        if m.is_empty() {
            return Err(vanished(&whole, shrink, grow > 0.0));
        }
    }
    if grow > 0.0 {
        // Erosion leaves many coplanar slivers; merging them first makes a
        // round's dilation ~5× faster.
        if shrink > 0.0 {
            m = m.simplify(SNAP);
        }
        let ball = ball(grow);
        // Non-convex: one hull per triangle, unioned as a balanced tree;
        // convex (declined here): one hull of pairwise vertex sums.
        m = m
            .try_dilate_by_convex(&ball, None, None)
            .unwrap_or_else(|| m.minkowski_sum(&ball));
    }
    from_manifold(&m.simplify(SNAP))
}

/// `child` as one Manifold (a group is its pieces' union), or its exact hull
/// when convex but for snapping: snapped split quads (on a sphere, a rounded
/// edge) sit a hair off flat, which sends the kernel down its per-triangle
/// paths, ~100–1000× slower.
fn convexified(child: &Solid) -> Result<Manifold, String> {
    let m = Manifold::from_impl(node(child, Matrix4::identity())?.evaluate());
    if m.is_empty() {
        return Ok(m);
    }
    let hull = m.convex_hull();
    // Snapping moves a vertex at most SNAP: a quick reject first, as dents that
    // shallow miss no more volume than they hold.
    if hull.volume() - m.volume() > SNAP * hull.surface_area() || !nearly_convex(&m)? {
        return Ok(m);
    }
    Ok(hull_of(&child.mesh()?))
}

/// Whether no vertex of `m` is more than 2 × SNAP outside any of its faces'
/// planes: convex up to snapping. (Holes leave no volume to miss, e.g. a
/// hairline one through a block, but their walls fail this.)
fn nearly_convex(m: &Manifold) -> Result<bool, String> {
    let mesh = from_manifold(m)?;
    let points: Vec<Vector3<f64>> = mesh.vertices.iter().map(|&p| p.into()).collect();
    Ok(mesh.triangles.iter().all(|t| {
        let [a, b, c] = t.map(|i| points[i as usize]);
        let n = face_normal(a, b, c);
        points.iter().all(|&p| n.dot(p - a) <= 2.0 * SNAP)
    }))
}

/// A sphere of `radius` faceted like `Solid.sphere`: vertices on the axes, so
/// flat axis-aligned faces move by exactly `radius`. Hulled, so the
/// Minkowski fast paths see it as convex.
fn ball(radius: f64) -> Manifold {
    hull_of(&mesh::sphere(radius, segments_for(radius)))
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
    format!(
        "{what}: it is nowhere thicker than {} mm{limit}",
        2.0 * depth
    )
}

/// For a convex solid, about the deepest shrink (mm, rounded down to 0.01)
/// that leaves something: a bisection over the closed-form erosion.
fn deepest(whole: &Manifold, depth: f64) -> Option<f64> {
    if !whole.as_impl().is_convex() {
        return None;
    }
    let fits = |r: f64| {
        whole
            .try_convex_erosion(&ball(r), None, None)
            .is_some_and(|m| !m.is_empty())
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
    use crate::BoolOp;
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
        assert!(
            mesh.print_problems("it").is_empty(),
            "{:?}",
            mesh.print_problems("it")
        );
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
        assert!(
            (shrunk.volume() - 512.0).abs() < 1e-9,
            "{}",
            shrunk.volume()
        );
    }

    #[test]
    fn round_rounds_a_cube_in_place() {
        let (s, r) = (10.0, 1.0);
        let mesh = near(&cube(s).rounded(r), grown_cube(s - 2.0 * r, r), 0.005);
        let b = mesh.bounds().unwrap();
        assert_eq!((b.min, b.max), ([-5.0; 3], [5.0; 3]));
    }

    /// A 20 × 20 mm L of 6 mm arms, 6 mm tall, corner at the origin: concave.
    fn l_shape() -> Arc<Solid> {
        let bar = |x: f64, y: f64| {
            arc(arc(Solid::Box { size: [x, y, 6.0] }).translate([x / 2.0, y / 2.0, 3.0]))
        };
        arc(Solid::boolean(
            BoolOp::Union,
            vec![bar(20.0, 6.0), bar(6.0, 20.0)],
        ))
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
        assert!(
            close(b.min, [0.0; 3]) && close(b.max, [20.0, 20.0, 6.0]),
            "{b:?}"
        );
    }

    #[test]
    fn a_hairline_hole_is_not_hulled_away() {
        // Missing too little volume to fail the snapping test, but a hole.
        let block = cube(100.0);
        let pin = arc(Solid::Box {
            size: [0.0625, 0.0625, 120.0],
        });
        let holed = arc(Solid::boolean(BoolOp::Subtract, vec![block, pin]));
        let shrunk = holed.offset(-1.0).mesh().unwrap();
        // The hole grows to ~2 mm across: far more than a solid block loses.
        assert!(
            shrunk.volume() < 98.0f64.powi(3) - 98.0 * 3.0,
            "{}",
            shrunk.volume()
        );
    }

    #[test]
    fn offsetting_nothing_is_nothing() {
        let apart = arc(Solid::boolean(
            BoolOp::Intersect,
            vec![cube(1.0), arc(cube(1.0).translate([5.0, 0.0, 0.0]))],
        ));
        for s in [
            apart.clone().offset(1.0),
            apart.clone().offset(-1.0),
            apart.rounded(1.0),
        ] {
            assert!(s.mesh().unwrap().triangles.is_empty());
        }
    }

    #[test]
    fn shrinking_away_names_the_limit() {
        let err = cube(10.0).offset(-6.0).mesh().unwrap_err();
        assert_eq!(
            err,
            "Solid.offset(-6) shrinks the part to nothing: it is nowhere thicker than 12 mm \
— at most 4.99 mm fits"
        );
        let err = arc(Solid::Box {
            size: [20.0, 20.0, 4.0],
        })
        .rounded(2.5)
        .mesh()
        .unwrap_err();
        assert!(
            err.starts_with("Solid.round(2.5) removes the whole part"),
            "{err}"
        );
        assert!(err.ends_with("at most 1.99 mm fits"), "{err}");
    }

    /// Not a gate (timings vary by machine): run with `--ignored --nocapture`
    /// in release to see the cost of each path.
    #[test]
    #[ignore]
    fn timings() {
        let l = l_shape();
        let disc = arc(Solid::Cylinder {
            radius: 20.0,
            height: 6.0,
        });
        let holed = arc(Solid::boolean(
            BoolOp::Subtract,
            vec![
                arc(Solid::Box {
                    size: [40.0, 30.0, 6.0],
                }),
                arc(Solid::Cylinder {
                    radius: 8.0,
                    height: 8.0,
                }),
            ],
        ));
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
