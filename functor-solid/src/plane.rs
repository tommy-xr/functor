//! Plane operations: trim a solid to one side of a plane, and round or bevel
//! the straight edge where two flat faces meet. Planes are `normal·p = offset`
//! with a unit `normal` pointing into the material kept. Both build CSG tree
//! nodes, like a boolean, so they evaluate together with the booleans around
//! them.

use std::f64::consts::TAU;

use cgmath::{EuclideanSpace, InnerSpace, Matrix3, Matrix4, SquareMatrix, Transform, Vector3};
use manifold_rust::csg_tree::CsgNode;
use manifold_rust::types::OpType;

use crate::csg::to_manifold;
use crate::{Bounds, Mesh, Sketch};

/// `normal·p = offset`, `normal` unit length.
pub type Plane = ([f64; 3], f64);

/// How an edge is finished: rounded to a radius, or bevelled back a distance
/// along each face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeCut {
    Round(f64),
    Bevel(f64),
}

/// How far a round's circle is drawn past true size (mm), so it crosses each
/// face at a slight crease instead of grazing it: a faceted arc can't be
/// exactly tangent, and a graze leaves hairline slivers. 5× the chord sag.
const ROUND_OVERSIZE: f64 = 0.05;

/// How far a cutter reaches past the faces (mm), so no cut face coincides
/// with the solid's.
const SLACK: f64 = 0.5;

/// `child` with everything on the side `normal` points away from removed:
/// intersected with a box past the child's extent on the kept side.
pub(crate) fn trim(child: CsgNode, (normal, offset): Plane) -> Result<CsgNode, String> {
    let Some(b) = extent(&child) else {
        return Ok(child);
    };
    let n = Vector3::from(normal);
    let centre = (Vector3::from(b.min) + Vector3::from(b.max)) / 2.0;
    let gap = n.dot(centre) - offset;
    // Every point of the child lies within `reach` of `foot`, on the plane.
    let foot = centre - n * gap;
    let reach = (Vector3::from(b.max) - Vector3::from(b.min)).magnitude() + gap.abs() + 1.0;
    let [x, y, z] = crate::basis(any_perpendicular(n).into(), normal)?;
    let placed = Matrix4::from_cols(
        Vector3::from(x).extend(0.0),
        Vector3::from(y).extend(0.0),
        Vector3::from(z).extend(0.0),
        (foot + n * reach).extend(1.0),
    );
    let mut half = Mesh::default();
    half.append(&crate::mesh::boxed([2.0 * reach; 3]), placed);
    Ok(CsgNode::op_n(OpType::Intersect, vec![child, leaf(&half)?]))
}

/// `child` minus the cutter that finishes the edge where faces on planes `a`
/// and `b` meet, run along that line past the child's extent.
/// `cut` is in the edge's own frame, `scale` times smaller than the tree's;
/// `phase` turns a round's facets by that fraction of a step (see
/// [`facet_phase`]).
pub(crate) fn edge(
    child: CsgNode,
    a: Plane,
    b: Plane,
    cut: EdgeCut,
    scale: f64,
    phase: f64,
) -> Result<CsgNode, String> {
    let Some(bounds) = extent(&child) else {
        return Ok(child);
    };
    let corners = (0..8).map(|i: u8| {
        Vector3::from(std::array::from_fn(|k| {
            if i >> k & 1 == 0 {
                bounds.min[k]
            } else {
                bounds.max[k]
            }
        }))
    });
    let cutter = leaf(&cutter(a, b, cut, scale, phase, corners))?;
    Ok(CsgNode::op_n(OpType::Subtract, vec![child, cutter]))
}

/// A round of `r` between faces whose inward normals have dot product `c`
/// must reach past its own oversize, or its circle swallows the corner and
/// cuts nothing (r < 0.12 mm at 90°, 1.4 mm at 150°).
pub(crate) fn check_round(r: f64, c: f64) -> Result<(), String> {
    // Half the angle between the faces; the circle's centre is r / sin(h) out.
    let h = (std::f64::consts::PI - c.clamp(-1.0, 1.0).acos()) / 2.0;
    let least = ROUND_OVERSIZE / (1.0 / h.sin() - 1.0);
    if r <= least {
        return Err(format!(
            "a {r} mm round is too small for this {:.0}° edge to cut — use at least \
{:.2} mm, or a bevel",
            (2.0 * h).to_degrees(),
            ((least + 1e-9) * 100.0).ceil() / 100.0
        ));
    }
    Ok(())
}

/// A fraction in [0, 1) fixed by an edge's own planes and size. Equal rounds
/// that meet at a corner, faceted alike, would put facet edges at the same
/// heights, crossing exactly: a pinch. Turning each by its own fraction of a
/// step keeps them apart. Hashed from the edge as written, not as moved, so
/// it matches across platforms.
pub(crate) fn facet_phase(a: Plane, b: Plane, cut: EdgeCut) -> f64 {
    let (EdgeCut::Round(size) | EdgeCut::Bevel(size)) = cut;
    let bits = [
        a.0[0], a.0[1], a.0[2], a.1, b.0[0], b.0[1], b.0[2], b.1, size,
    ];
    // splitmix64 over each value's bits.
    let mut h: u64 = 0;
    for x in bits {
        h = (h ^ x.to_bits()).wrapping_add(0x9e37_79b9_7f4a_7c15);
        h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        h ^= h >> 31;
    }
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// A plane in a transform's frame (rigid plus uniform scale, maybe mirrored).
pub(crate) fn moved((normal, offset): Plane, xform: Matrix4<f64>) -> Plane {
    let linear = Matrix3::from_cols(xform.x.truncate(), xform.y.truncate(), xform.z.truncate());
    let n = (linear * Vector3::from(normal)).normalize();
    let on = xform.transform_point(cgmath::Point3::from_vec(Vector3::from(normal) * offset));
    (n.into(), n.dot(on.to_vec()))
}

/// A tree's bounds, never smaller than its result: a subtraction's first
/// operand, an intersection's overlap, a union's whole. `None` if empty.
fn extent(node: &CsgNode) -> Option<Bounds> {
    match node {
        CsgNode::Leaf(leaf) => (leaf.num_vert() > 0).then(|| {
            let b = leaf.get_bounding_box();
            Bounds {
                min: [b.min.x, b.min.y, b.min.z],
                max: [b.max.x, b.max.y, b.max.z],
            }
        }),
        CsgNode::Op { op, children, .. } => {
            let mut boxes = children.iter().map(extent);
            match op {
                OpType::Subtract => boxes.next().flatten(),
                OpType::Add => boxes.flatten().reduce(|a, b| Bounds {
                    min: std::array::from_fn(|k| a.min[k].min(b.min[k])),
                    max: std::array::from_fn(|k| a.max[k].max(b.max[k])),
                }),
                OpType::Intersect => boxes
                    .collect::<Option<Vec<_>>>()?
                    .into_iter()
                    .reduce(|a, b| Bounds {
                        min: std::array::from_fn(|k| a.min[k].max(b.min[k])),
                        max: std::array::from_fn(|k| a.max[k].min(b.max[k])),
                    })
                    .filter(|b| (0..3).all(|k| b.min[k] <= b.max[k])),
            }
        }
    }
}

fn leaf(mesh: &Mesh) -> Result<CsgNode, String> {
    Ok(CsgNode::leaf(to_manifold(mesh)?.into_impl()))
}

/// Some unit vector perpendicular to unit `n`.
fn any_perpendicular(n: Vector3<f64>) -> Vector3<f64> {
    let axis = if n.x.abs() < 0.9 {
        Vector3::unit_x()
    } else {
        Vector3::unit_y()
    };
    axis - n * axis.dot(n)
}

/// The edge's cutter: its 2D profile across the edge, extruded along the edge
/// line past every point in `span`.
fn cutter(
    a: Plane,
    b: Plane,
    cut: EdgeCut,
    scale: f64,
    phase: f64,
    span: impl Iterator<Item = Vector3<f64>>,
) -> Mesh {
    let (na, nb) = (Vector3::from(a.0), Vector3::from(b.0));
    let c = na.dot(nb);
    // The point on both planes nearest the origin, and the line's direction.
    let p0 = na * ((a.1 - b.1 * c) / (1.0 - c * c)) + nb * ((b.1 - a.1 * c) / (1.0 - c * c));
    let d = na.cross(nb).normalize();
    // Along each face, away from the edge (into the other face's side).
    let away = |n: Vector3<f64>, other: Vector3<f64>| {
        let u = d.cross(n);
        if u.dot(other) > 0.0 {
            u
        } else {
            -u
        }
    };
    let (ua, ub) = (away(na, nb), away(nb, na));
    // Profile frame: X along face a, Y into the solid; face b leaves at `angle`.
    let angle = ub.dot(na).atan2(ub.dot(ua));
    let w = ua.cross(na);
    let (lo, hi) = span.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
        let t = (p - p0).dot(w);
        (lo.min(t), hi.max(t))
    });
    let (profile, reach) = profile(angle, cut, scale, phase);
    let margin = SLACK + reach;
    let placed = Matrix4::from_cols(
        ua.extend(0.0),
        na.extend(0.0),
        w.extend(0.0),
        (p0 + w * (lo - margin)).extend(1.0),
    );
    let mut mesh = Mesh::default();
    mesh.append(&profile.extrude(hi - lo + 2.0 * margin), placed);
    mesh
}

/// The material a cut removes across an edge whose faces leave the origin
/// along +X and at `angle` radians from it (the solid between them), its
/// size `scale`d, and how far along each face it reaches.
fn profile(angle: f64, cut: EdgeCut, scale: f64, phase: f64) -> (Sketch, f64) {
    let h = angle / 2.0;
    let (sa, ca) = angle.sin_cos();
    // The corner out to `leg` along each face, with SLACK outside both.
    let corner = |leg: f64| {
        Sketch::polygon(vec![
            [-SLACK * h.cos(), -SLACK * h.sin()],
            [leg, -SLACK],
            [leg, 0.0],
            [leg * ca, leg * sa],
            [leg * ca - SLACK * sa, leg * sa + SLACK * ca],
        ])
    };
    match cut {
        EdgeCut::Bevel(leg) => (corner(leg * scale), leg * scale),
        // The corner minus the circle tangent to both faces.
        EdgeCut::Round(r) => {
            // The oversize scales too, so a scaled part keeps its shape.
            let (r, radius) = (r * scale, (r + ROUND_OVERSIZE) * scale);
            let leg = r / h.tan();
            let centre = r / h.sin();
            let (cx, cy) = (centre * h.cos(), centre * h.sin());
            let turn = TAU * phase / crate::segments_for(radius) as f64;
            let (sin, cos) = turn.sin_cos();
            let circle = Sketch::circle(radius)
                .map_points(|[x, y]| [x * cos - y * sin + cx, x * sin + y * cos + cy]);
            (corner(leg).subtract(&circle), leg)
        }
    }
}

/// The reflection through a plane: `p - 2 (normal·p - offset) normal`.
pub(crate) fn mirror_matrix((n, offset): Plane) -> Matrix4<f64> {
    let n = Vector3::from(n);
    let outer = Matrix3::from_cols(n * n.x, n * n.y, n * n.z);
    Matrix4::from_translation(n * (2.0 * offset)) * Matrix4::from(Matrix3::identity() - outer * 2.0)
}

/// A right-handed orthonormal basis `[x, y, z]`: `z` along `z_axis`, `x` the
/// part of `x_axis` perpendicular to it.
pub fn basis(x_axis: [f64; 3], z_axis: [f64; 3]) -> Result<[[f64; 3]; 3], String> {
    let (x, z) = (Vector3::from(x_axis), Vector3::from(z_axis));
    if !(z.magnitude() > 0.0 && z.magnitude().is_finite()) {
        return Err(format!(
            "a frame needs a non-zero, finite z axis, got {z_axis:?}"
        ));
    }
    let z = z.normalize();
    let x_perp = x - z * x.dot(z);
    // x within ~0.06° of z leaves no reliable direction for it (NaN fails too).
    if !(x_perp.magnitude() > 1e-3 * x.magnitude()) {
        return Err(format!(
            "a frame's x axis must not be zero or parallel to its z axis, got x {x_axis:?} \
and z {z_axis:?}"
        ));
    }
    let x = x_perp.normalize();
    Ok([x.into(), z.cross(x).into(), z.into()])
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};
    use std::sync::Arc;

    use super::*;
    use crate::{BoolOp, Solid};

    fn arc(s: Solid) -> Arc<Solid> {
        Arc::new(s)
    }

    /// The box from 0 to `size` on every axis.
    fn cube(size: f64) -> Arc<Solid> {
        arc(arc(Solid::Box { size: [size; 3] }).translate([size / 2.0; 3]))
    }

    fn clean(mesh: &Mesh) {
        assert_eq!(mesh.print_problems("part"), Vec::<String>::new());
    }

    #[test]
    fn trim_keeps_the_side_the_normal_points_to() {
        // x + y + z ≥ 6 removes the corner tetrahedron of volume 6³/6.
        let kept = Solid::trim(cube(10.0), [1.0, 1.0, 1.0], 6.0)
            .unwrap()
            .mesh()
            .unwrap();
        clean(&kept);
        assert!(
            (kept.volume() - (1000.0 - 36.0)).abs() < 1e-9,
            "{}",
            kept.volume()
        );
        // Normal and offset scale together: (0, 0, 2)·p = 8 is z = 4.
        let top = Solid::trim(cube(10.0), [0.0, 0.0, 2.0], 8.0)
            .unwrap()
            .mesh()
            .unwrap();
        assert_eq!(top.bounds().unwrap().min[2], 4.0);
        assert!((top.volume() - 600.0).abs() < 1e-9);
        // A plane past the solid keeps all of it, or nothing.
        let all = Solid::trim(cube(10.0), [0.0, 0.0, 1.0], -5.0)
            .unwrap()
            .mesh()
            .unwrap();
        assert!((all.volume() - 1000.0).abs() < 1e-9);
        let none = Solid::trim(cube(10.0), [0.0, 0.0, 1.0], 15.0)
            .unwrap()
            .mesh()
            .unwrap();
        assert!(none.triangles.is_empty());
        let err = Solid::trim(cube(1.0), [0.0; 3], 1.0).unwrap_err();
        assert!(
            err.contains("a trim plane needs a non-zero, finite normal"),
            "{err}"
        );
    }

    #[test]
    fn mirror_flips_bounds_and_keeps_volume_and_faces() {
        let part = arc(Solid::boolean(
            BoolOp::Subtract,
            vec![
                cube(10.0),
                arc(arc(Solid::Sphere { radius: 4.0 }).translate([10.0, 0.0, 5.0])),
            ],
        ));
        let (before, b) = (part.mesh().unwrap(), part.mesh().unwrap().bounds().unwrap());
        // Through x = 3, given as (2, 0, 0)·p = 6.
        let mirrored = arc(part.clone().mirror([2.0, 0.0, 0.0], 6.0).unwrap());
        let after = mirrored.mesh().unwrap();
        clean(&after);
        assert!((after.volume() - before.volume()).abs() < 1e-9);
        let m = after.bounds().unwrap();
        assert!(
            (m.min[0] - (6.0 - b.max[0])).abs() < 1e-9
                && (m.max[0] - (6.0 - b.min[0])).abs() < 1e-9
        );
        assert_eq!((m.min[1], m.max[2]), (b.min[1], b.max[2]));
        // Inside a boolean too.
        let both = Solid::boolean(BoolOp::Union, vec![part, mirrored])
            .mesh()
            .unwrap();
        clean(&both);
        assert!(both.volume() > before.volume());
    }

    #[test]
    fn place_matches_the_same_rotations_and_move() {
        // x → world Y, y → world Z, z → world X: a turn about X then Z.
        let axes = basis([0.0, 3.0, 0.0], [2.0, 0.0, 0.1]).unwrap();
        let axes = basis(axes[0], [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(axes, [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]]);
        let part = arc(Solid::Box {
            size: [1.0, 2.0, 3.0],
        });
        let placed = part.clone().place([5.0, 6.0, 7.0], axes).mesh().unwrap();
        let turned = arc(arc(part.rotate(0, FRAC_PI_2)).rotate(2, FRAC_PI_2))
            .translate([5.0, 6.0, 7.0])
            .mesh()
            .unwrap();
        for (p, q) in placed.vertices.iter().zip(&turned.vertices) {
            assert!(
                p.iter().zip(q).all(|(a, b)| (a - b).abs() < 1e-12),
                "{p:?} {q:?}"
            );
        }
        // The frame's x is made perpendicular to z; y completes it right-handed.
        let [x, y, z] = basis([1.0, 0.0, 1.0], [0.0, 0.0, 5.0]).unwrap();
        assert_eq!(
            (x, y, z),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn basis_refuses_zero_and_parallel_axes() {
        assert!(basis([1.0, 0.0, 0.0], [0.0; 3])
            .unwrap_err()
            .contains("non-zero, finite z axis"));
        for x in [[0.0; 3], [0.0, 0.0, -2.0]] {
            let err = basis(x, [0.0, 0.0, 1.0]).unwrap_err();
            assert!(err.contains("must not be zero or parallel"), "{err}");
        }
    }

    /// The cube's four bottom faces' planes: the bottom, then -x, +x, -y, +y
    /// sides, each normal pointing in.
    fn bottom_edges(size: f64) -> (Plane, [Plane; 4]) {
        let s = size;
        (
            ([0.0, 0.0, 1.0], 0.0),
            [
                ([1.0, 0.0, 0.0], 0.0),
                ([-1.0, 0.0, 0.0], -s),
                ([0.0, 1.0, 0.0], 0.0),
                ([0.0, -1.0, 0.0], -s),
            ],
        )
    }

    fn finished(cuts: &[EdgeCut]) -> Mesh {
        let (bottom, sides) = bottom_edges(20.0);
        sides
            .iter()
            .zip(cuts)
            .fold(cube(20.0), |s, (side, cut)| {
                arc(Solid::edge(s, bottom, *side, *cut).unwrap())
            })
            .mesh()
            .unwrap()
    }

    #[test]
    fn rounded_bottom_edges_match_the_analytic_volume() {
        // Each edge removes (1 - π/4) r² along its 20 mm; the two cuts at a
        // corner overlap by ∫(r - √(r² - u²))² du = (5/3 - π/2) r³.
        let r: f64 = 2.0;
        let exact = 8000.0 - 4.0 * 20.0 * (1.0 - PI / 4.0) * r * r
            + 4.0 * (5.0 / 3.0 - FRAC_PI_2) * r.powi(3);
        let mesh = finished(&[EdgeCut::Round(r); 4]);
        clean(&mesh);
        assert!(
            (mesh.volume() / exact - 1.0).abs() < 0.005,
            "{} vs {exact}",
            mesh.volume()
        );
        // Off the edge, the faces are untouched: the bottom still reaches z = 0.
        assert_eq!(mesh.bounds().unwrap().min[2], 0.0);
    }

    #[test]
    fn bevelled_bottom_edges_are_exact() {
        // Each removes a 2 × 2 mm right triangle along 20 mm; corners overlap
        // by ∫(2 - z)² dz = 8/3.
        let mesh = finished(&[EdgeCut::Bevel(2.0); 4]);
        clean(&mesh);
        assert!(
            (mesh.volume() - (8000.0 - 160.0 + 4.0 * 8.0 / 3.0)).abs() < 1e-6,
            "{}",
            mesh.volume()
        );
        // Rounds meeting a bevel at a corner.
        let mixed = finished(&[
            EdgeCut::Round(3.0),
            EdgeCut::Round(3.0),
            EdgeCut::Round(3.0),
            EdgeCut::Bevel(3.0),
        ]);
        clean(&mixed);
    }

    /// A wedge like a display stand's shell: square sides, the front leaning
    /// 15° and the back 30°, all bottom edges finished with one size.
    #[test]
    fn oblique_wedge_edges_meet_cleanly_at_equal_sizes() {
        let (front, back) = (15f64.to_radians(), 30f64.to_radians());
        let block = arc(arc(Solid::Box {
            size: [120.0, 200.0, 50.0],
        })
        .translate([0.0, 50.0, 25.0]));
        // The front face through the origin leaning back, the back face through y = 60.
        let front_face = ([0.0, front.cos(), -front.sin()], 0.0);
        let back_normal = [0.0, -back.cos(), -back.sin()];
        let back_face = (back_normal, -60.0 * back.cos());
        let wedge = [front_face, back_face]
            .into_iter()
            .fold(block, |s, (n, d)| arc(Solid::trim(s, n, d).unwrap()));
        let table = ([0.0, 0.0, 1.0], 0.0);
        for size in [3.0, 2.5] {
            let edges = [
                (([1.0, 0.0, 0.0], -60.0), EdgeCut::Round(size)),
                (([-1.0, 0.0, 0.0], -60.0), EdgeCut::Round(size)),
                (front_face, EdgeCut::Round(size)),
                (back_face, EdgeCut::Bevel(size)),
            ];
            let done = edges.iter().fold(wedge.clone(), |s, (face, cut)| {
                arc(Solid::edge(s, table, *face, *cut).unwrap())
            });
            let mesh = done.mesh().unwrap();
            clean(&mesh);
            assert!(mesh.volume() < wedge.mesh().unwrap().volume());
        }
    }

    #[test]
    fn edge_refuses_parallel_faces_and_bad_sizes() {
        let up = ([0.0, 0.0, 1.0], 0.0);
        let err =
            Solid::edge(cube(1.0), up, ([0.0, 0.0, -3.0], -3.0), EdgeCut::Round(1.0)).unwrap_err();
        assert!(err.contains("must not be parallel"), "{err}");
        let err =
            Solid::edge(cube(1.0), up, ([1.0, 0.0, 0.0], 0.0), EdgeCut::Bevel(0.0)).unwrap_err();
        assert!(err.contains("size must be positive, got 0"), "{err}");
        // A round inside its own oversize would cut nothing.
        let err =
            Solid::edge(cube(1.0), up, ([1.0, 0.0, 0.0], 0.0), EdgeCut::Round(0.1)).unwrap_err();
        assert!(
            err.contains("too small for this 90° edge to cut — use at least 0.13 mm"),
            "{err}"
        );
        let obtuse = (
            [-(30f64.to_radians().sin()), 0.0, 30f64.to_radians().cos()],
            0.0,
        );
        let err = Solid::edge(cube(1.0), up, obtuse, EdgeCut::Round(1.0)).unwrap_err();
        assert!(err.contains("150° edge"), "{err}");
        assert!(Solid::edge(cube(1.0), up, obtuse, EdgeCut::Round(1.5)).is_ok());
        // An empty solid stays empty.
        let empty = arc(Solid::trim(cube(1.0), [0.0, 0.0, 1.0], 5.0).unwrap());
        let mesh = Solid::edge(empty, up, ([1.0, 0.0, 0.0], 0.0), EdgeCut::Round(1.0))
            .unwrap()
            .mesh()
            .unwrap();
        assert!(mesh.triangles.is_empty());
    }

    /// Trims and edges evaluate inside the boolean tree, transformed: a
    /// scaled, turned, mirrored trimmed cube rounds the same as built in place.
    #[test]
    fn trims_and_edges_follow_transforms_and_booleans() {
        let (bottom, sides) = bottom_edges(10.0);
        let rounded = arc(Solid::edge(cube(10.0), bottom, sides[0], EdgeCut::Round(1.0)).unwrap());
        let trimmed = arc(Solid::trim(rounded.clone(), [0.0, 0.0, 1.0], 5.0).unwrap());
        let v = trimmed.mesh().unwrap().volume();
        let moved = arc(arc(arc(trimmed.clone().scale(2.0)).rotate(2, 0.7))
            .mirror([1.0, 0.0, 0.0], 3.0)
            .unwrap());
        let mesh = Solid::boolean(
            BoolOp::Union,
            vec![
                moved,
                arc(Solid::Box { size: [1.0; 3] })
                    .translate([100.0, 0.0, 0.0])
                    .into(),
            ],
        )
        .mesh()
        .unwrap();
        clean(&mesh);
        assert!(
            (mesh.volume() - (8.0 * v + 1.0)).abs() < 1e-3 * v,
            "{} vs {}",
            mesh.volume(),
            8.0 * v + 1.0
        );
        // Only z ≥ 5 remains: the round (at z = 0) was trimmed away.
        assert!((v - 500.0).abs() < 1e-6, "{v}");
    }

    /// Equal rounds meeting at a corner on a tilted face, then turned: facets
    /// at equal heights would cross exactly and merge in f32 (a pinch).
    #[test]
    fn equal_rounds_on_a_tilted_cube_stay_clean_when_turned() {
        let t = 7f64.to_radians();
        let rot = |v: [f64; 3]| {
            [
                v[0],
                v[1] * t.cos() - v[2] * t.sin(),
                v[1] * t.sin() + v[2] * t.cos(),
            ]
        };
        let (bottom, sides) = bottom_edges(20.0);
        let tilted = arc(cube(20.0).rotate(0, t));
        let done = sides.iter().fold(tilted, |s, side| {
            arc(Solid::edge(
                s,
                (rot(bottom.0), 0.0),
                (rot(side.0), side.1),
                EdgeCut::Round(2.0),
            )
            .unwrap())
        });
        for angle in [0.3, 1.1, 2.0] {
            let turned = arc(arc(done.clone().rotate(0, angle)).rotate(1, angle * 0.7));
            clean(&turned.mesh().unwrap());
        }
    }

    /// A scaled round is the same material alone (its own-frame mesh, scaled)
    /// as inside a boolean (rebuilt at the boolean's scale).
    #[test]
    fn scaled_rounds_match_inside_and_outside_booleans() {
        let (bottom, sides) = bottom_edges(10.0);
        let rounded = arc(Solid::edge(cube(10.0), bottom, sides[0], EdgeCut::Round(1.0)).unwrap());
        let scaled = arc(rounded.scale(10.0));
        let alone = scaled.mesh().unwrap().volume();
        let far = arc(arc(Solid::Box { size: [1.0; 3] }).translate([500.0, 0.0, 0.0]));
        let joined = Solid::boolean(BoolOp::Union, vec![scaled, far])
            .mesh()
            .unwrap()
            .volume();
        assert!(
            (joined - 1.0 - alone).abs() < 1e-4 * alone,
            "{joined} vs {alone}"
        );
    }
}
