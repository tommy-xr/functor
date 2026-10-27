//! Printable solids for Functor parts.
//!
//! A [`Solid`] is an immutable description tree — primitives, transforms, and
//! groups — in millimetres, Z up (the 3D-printing convention). It is turned
//! into a closed triangle [`Mesh`] only when something needs triangles
//! (export, measurement), so building and editing a solid stays cheap.
//!
//! Every primitive meshes closed and outward-facing, and transforms preserve
//! that. A group keeps each piece its own shell, so pieces must overlap or stay
//! apart: STL readers join pieces that share a vertex position (see
//! [`Mesh::shared_positions`]). Booleans ([`Solid::Boolean`]) merge solids
//! into one closed shell.

mod csg;
mod mesh;
mod sketch;
pub mod stl;
mod text;
pub mod threemf;

use std::sync::{Arc, OnceLock};

use cgmath::{Matrix4, Rad, SquareMatrix, Vector3};

pub use mesh::{face_normal, Bounds, Mesh};
pub use sketch::Sketch;
pub use text::{text, text_width, Font};

/// Maximum distance (mm) a curved surface's facets may sag from the true
/// surface. 0.01 mm is well under a 0.4 mm nozzle's resolution.
pub const CHORD_TOLERANCE: f64 = 0.01;

#[derive(Debug, Clone, PartialEq)]
pub enum Solid {
    /// An axis-aligned box centered on the origin.
    Box { size: [f64; 3] },
    /// A Z-axis cylinder centered on the origin.
    Cylinder { radius: f64, height: f64 },
    /// A sphere centered on the origin.
    Sphere { radius: f64 },
    /// A 2D profile swept from z = 0 to z = `height`.
    Extrude { sketch: Arc<Sketch>, height: f64 },
    Transform {
        matrix: Matrix4<f64>,
        child: Arc<Solid>,
    },
    /// Pieces placed side by side. Overlaps are NOT merged — each piece stays
    /// its own closed shell (booleans are a separate operation).
    Group(Vec<Arc<Solid>>),
    /// A boolean of the children: union of all, the first minus the rest, or
    /// the intersection of all. A group child counts as the union of its pieces.
    /// Build with [`Solid::boolean`].
    Boolean {
        op: BoolOp,
        children: Vec<Arc<Solid>>,
        result: BooleanResult,
    },
}

/// A boolean's mesh, evaluated once in the boolean's own frame and reused by
/// every transform above it (e.g. a part rotated in `draw` each frame). Curves
/// inside are faceted at that frame's scale. Ignored by equality.
#[derive(Clone, Default)]
pub struct BooleanResult(Arc<OnceLock<Result<Arc<Mesh>, String>>>);

impl PartialEq for BooleanResult {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for BooleanResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BooleanResult")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
}

impl Solid {
    pub fn boolean(op: BoolOp, children: Vec<Arc<Solid>>) -> Solid {
        Solid::Boolean {
            op,
            children,
            result: BooleanResult::default(),
        }
    }

    pub fn translate(self: Arc<Self>, offset: [f64; 3]) -> Solid {
        self.transform(Matrix4::from_translation(Vector3::from(offset)))
    }

    /// Rotate about the X (0), Y (1), or Z (2) axis.
    pub fn rotate(self: Arc<Self>, axis: usize, radians: f64) -> Solid {
        let angle = Rad(radians);
        self.transform(match axis {
            0 => Matrix4::from_angle_x(angle),
            1 => Matrix4::from_angle_y(angle),
            _ => Matrix4::from_angle_z(angle),
        })
    }

    pub fn scale(self: Arc<Self>, factor: f64) -> Solid {
        self.transform(Matrix4::from_scale(factor))
    }

    fn transform(self: Arc<Self>, matrix: Matrix4<f64>) -> Solid {
        Solid::Transform {
            matrix,
            child: self,
        }
    }

    /// Triangulate the whole tree. Fails only if a boolean does.
    pub fn mesh(&self) -> Result<Mesh, String> {
        let mut out = Mesh::default();
        self.append_to(&mut out, Matrix4::identity())?;
        Ok(out)
    }

    pub(crate) fn append_to(&self, out: &mut Mesh, xform: Matrix4<f64>) -> Result<(), String> {
        // Transforms are rigid plus uniform scale, so |det| = scale³. Facets
        // are counted on the radius as it will print, not as written (sketch
        // curves are faceted when drawn, so draw those at final size).
        let scale = xform.determinant().cbrt().abs();
        match self {
            Solid::Box { size } => out.append(&mesh::boxed(*size), xform),
            Solid::Cylinder { radius, height } => out.append(
                &mesh::cylinder(*radius, *height, segments_for(radius * scale)),
                xform,
            ),
            Solid::Sphere { radius } => {
                out.append(&mesh::sphere(*radius, segments_for(radius * scale)), xform)
            }
            Solid::Extrude { sketch, height } => out.append(&sketch.extrude(*height), xform),
            Solid::Transform { matrix, child } => return child.append_to(out, xform * matrix),
            Solid::Group(children) => {
                for child in children {
                    child.append_to(out, xform)?;
                }
            }
            Solid::Boolean { result, .. } => {
                let mesh = result
                    .0
                    .get_or_init(|| csg::evaluate(self).map(Arc::new))
                    .clone()?;
                out.append(&mesh, xform);
            }
        }
        Ok(())
    }
}

/// Facets around a circle of `radius` so no chord sags more than
/// [`CHORD_TOLERANCE`]; a multiple of 4 so circles stay symmetric on both axes.
/// E.g. r = 5 mm → 52, r = 50 mm → 160.
pub(crate) fn segments_for(radius: f64) -> usize {
    let ratio = (1.0 - CHORD_TOLERANCE / radius).clamp(-1.0, 1.0);
    let n = (std::f64::consts::PI / ratio.acos()).ceil() as usize;
    (n.clamp(12, 512) + 3) / 4 * 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn arc(solid: Solid) -> Arc<Solid> {
        Arc::new(solid)
    }

    #[test]
    fn cube_is_twelve_closed_triangles_of_exact_volume() {
        let mesh = Solid::Box { size: [10.0; 3] }.mesh().unwrap();
        assert_eq!(mesh.triangles.len(), 12);
        assert_eq!(mesh.non_manifold_edges(), 0);
        assert!((mesh.volume() - 1000.0).abs() < 1e-9);
        let b = mesh.bounds().unwrap();
        assert_eq!((b.min, b.max), ([-5.0; 3], [5.0; 3]));
    }

    #[test]
    fn cylinder_and_sphere_are_closed_and_near_analytic_volume() {
        use std::f64::consts::PI;
        let cyl = Solid::Cylinder {
            radius: 5.0,
            height: 10.0,
        }
        .mesh()
        .unwrap();
        assert_eq!(cyl.non_manifold_edges(), 0);
        let exact = PI * 25.0 * 10.0;
        assert!(
            (cyl.volume() - exact).abs() / exact < 0.005,
            "{}",
            cyl.volume()
        );

        let sphere = Solid::Sphere { radius: 5.0 }.mesh().unwrap();
        assert_eq!(sphere.non_manifold_edges(), 0);
        // Inscribed facets sag along both axes at quad centers: ~0.6% under.
        let exact = 4.0 / 3.0 * PI * 125.0;
        assert!(
            (sphere.volume() - exact).abs() / exact < 0.01,
            "{}",
            sphere.volume()
        );
    }

    #[test]
    fn transforms_move_bounds_and_keep_volume() {
        let cube = arc(Solid::Box {
            size: [2.0, 4.0, 6.0],
        });
        let turned = arc(cube.rotate(2, std::f64::consts::FRAC_PI_2));
        let mesh = turned.translate([10.0, 0.0, 3.0]).mesh().unwrap();
        let b = mesh.bounds().unwrap();
        let close = |a: [f64; 3], e: [f64; 3]| a.iter().zip(e).all(|(a, e)| (a - e).abs() < 1e-9);
        assert!(close(b.min, [8.0, -1.0, 0.0]), "{:?}", b.min);
        assert!(close(b.max, [12.0, 1.0, 6.0]), "{:?}", b.max);
        assert!((mesh.volume() - 48.0).abs() < 1e-9);
        assert_eq!(mesh.non_manifold_edges(), 0);
    }

    #[test]
    fn scale_multiplies_volume_by_cube_of_factor() {
        let mesh = arc(Solid::Box { size: [1.0; 3] })
            .scale(3.0)
            .mesh()
            .unwrap();
        assert!((mesh.volume() - 27.0).abs() < 1e-9);
    }

    #[test]
    fn group_keeps_each_piece_closed() {
        let a = arc(Solid::Box { size: [1.0; 3] });
        let b = arc(arc(Solid::Sphere { radius: 1.0 }).translate([5.0, 0.0, 0.0]));
        let mesh = Solid::Group(vec![a, b]).mesh().unwrap();
        assert_eq!(mesh.non_manifold_edges(), 0);
        assert!(mesh.volume() > 1.0);
    }

    #[test]
    fn pieces_touching_at_a_face_or_corner_share_positions() {
        let cube = || arc(Solid::Box { size: [2.0; 3] });
        let face = Solid::Group(vec![cube(), arc(cube().translate([2.0, 0.0, 0.0]))])
            .mesh()
            .unwrap();
        assert_eq!(face.non_manifold_edges(), 0);
        assert_eq!(face.shared_positions(), 4);
        let corner = Solid::Group(vec![cube(), arc(cube().translate([2.0, 2.0, 2.0]))])
            .mesh()
            .unwrap();
        assert_eq!(corner.shared_positions(), 1);
        let apart = Solid::Group(vec![cube(), arc(cube().translate([2.5, 0.0, 0.0]))])
            .mesh()
            .unwrap();
        assert_eq!(apart.shared_positions(), 0);
    }

    #[test]
    fn mirroring_scale_keeps_triangles_facing_outward() {
        let mesh = arc(Solid::Box { size: [2.0; 3] })
            .scale(-1.0)
            .mesh()
            .unwrap();
        assert!((mesh.volume() - 8.0).abs() < 1e-9, "{}", mesh.volume());
        assert_eq!(mesh.non_manifold_edges(), 0);
    }

    #[test]
    fn scaled_curves_keep_their_facet_tolerance() {
        use std::f64::consts::PI;
        let mesh = arc(Solid::Sphere { radius: 1.0 })
            .scale(100.0)
            .mesh()
            .unwrap();
        let exact = 4.0 / 3.0 * PI * 1e6;
        assert!(
            (mesh.volume() - exact).abs() / exact < 0.01,
            "{}",
            mesh.volume()
        );
    }

    fn boolean(op: BoolOp, children: Vec<Arc<Solid>>) -> Mesh {
        Solid::boolean(op, children).mesh().unwrap()
    }

    fn closed(mesh: &Mesh) {
        assert_eq!(mesh.non_manifold_edges(), 0);
        assert_eq!(mesh.shared_positions(), 0);
    }

    #[test]
    fn subtract_and_intersect_partition_the_box() {
        let cube = || arc(Solid::Box { size: [20.0; 3] });
        let ball = || arc(Solid::Sphere { radius: 12.0 });
        let cut = boolean(BoolOp::Subtract, vec![cube(), ball()]);
        let common = boolean(BoolOp::Intersect, vec![cube(), ball()]);
        closed(&cut);
        closed(&common);
        assert!((cut.volume() + common.volume() - 8000.0).abs() < 1e-6);
    }

    #[test]
    fn union_obeys_inclusion_exclusion() {
        let a = || {
            arc(Solid::Cylinder {
                radius: 5.0,
                height: 30.0,
            })
        };
        let b = || {
            arc(arc(Solid::Cylinder {
                radius: 5.0,
                height: 30.0,
            })
            .rotate(0, FRAC_PI_2))
        };
        let (va, vb) = (a().mesh().unwrap().volume(), b().mesh().unwrap().volume());
        let both = boolean(BoolOp::Union, vec![a(), b()]);
        let common = boolean(BoolOp::Intersect, vec![a(), b()]);
        closed(&both);
        // Inputs snap to a 15 nm grid, so compare relative to unsnapped volumes.
        assert!((both.volume() - (va + vb - common.volume())).abs() < 1e-5 * both.volume());
    }

    #[test]
    fn union_merges_pieces_that_touch() {
        // Two boxes sharing a face: one closed 2-box solid, no shared vertices.
        let cube = || arc(Solid::Box { size: [10.0; 3] });
        let joined = boolean(
            BoolOp::Union,
            vec![cube(), arc(cube().translate([10.0, 0.0, 0.0]))],
        );
        closed(&joined);
        assert!((joined.volume() - 2000.0).abs() < 1e-6);
        // A group operand unions its pieces too.
        let group = arc(Solid::Group(vec![
            cube(),
            arc(cube().translate([10.0, 0.0, 0.0])),
        ]));
        assert_eq!(
            boolean(BoolOp::Union, vec![group]).volume(),
            joined.volume()
        );
    }

    #[test]
    fn flush_through_hole_is_closed() {
        let plate = arc(Solid::Box {
            size: [20.0, 20.0, 4.0],
        });
        let pin = arc(Solid::Cylinder {
            radius: 3.0,
            height: 4.0,
        });
        let (pin_volume, holed) = (
            pin.mesh().unwrap().volume(),
            boolean(BoolOp::Subtract, vec![plate, pin]),
        );
        closed(&holed);
        assert!((holed.volume() - (1600.0 - pin_volume)).abs() < 1e-5 * holed.volume());
    }

    #[test]
    fn booleans_nest_and_are_deterministic() {
        let shell = || {
            let outer = arc(Solid::Sphere { radius: 10.0 });
            let inner = arc(Solid::Sphere { radius: 9.6 }); // 0.4 mm wall
            let slot = arc(arc(Solid::Box {
                size: [30.0, 2.0, 30.0],
            })
            .translate([0.0, 0.0, 5.0]));
            let hollow = arc(Solid::boolean(BoolOp::Subtract, vec![outer, inner]));
            Solid::boolean(BoolOp::Subtract, vec![hollow, slot])
                .mesh()
                .unwrap()
        };
        let a = shell();
        closed(&a);
        assert!(a.volume() > 0.0);
        assert_eq!(a, shell());
    }

    #[test]
    fn subtract_takes_many_cutters_and_groups_union_inside_booleans() {
        let cube = || arc(Solid::Box { size: [10.0; 3] });
        let slab = || {
            arc(Solid::Box {
                size: [2.0, 20.0, 20.0],
            })
        };
        let two_slots = boolean(
            BoolOp::Subtract,
            vec![
                cube(),
                arc(slab().translate([-3.0, 0.0, 0.0])),
                arc(slab().translate([3.0, 0.0, 0.0])),
            ],
        );
        closed(&two_slots);
        assert!((two_slots.volume() - 600.0).abs() < 1e-6);
        // A transformed group of two slabs cuts the same as two separate cutters.
        let pair = arc(Solid::Group(vec![
            arc(slab().translate([-3.0, 0.0, 0.0])),
            arc(slab().translate([3.0, 0.0, 0.0])),
        ]));
        let as_group = boolean(
            BoolOp::Subtract,
            vec![cube(), arc(pair.translate([0.0, 0.0, 0.0]))],
        );
        assert!((as_group.volume() - 600.0).abs() < 1e-6);
        // Intersect with a group child keeps what overlaps either piece.
        let pair = arc(Solid::Group(vec![
            arc(slab().translate([-3.0, 0.0, 0.0])),
            arc(slab().translate([3.0, 0.0, 0.0])),
        ]));
        assert!((boolean(BoolOp::Intersect, vec![cube(), pair]).volume() - 400.0).abs() < 1e-6);
    }

    #[test]
    fn boolean_is_evaluated_once_and_reused_under_transforms() {
        let part = arc(Solid::boolean(
            BoolOp::Subtract,
            vec![
                arc(Solid::Box { size: [10.0; 3] }),
                arc(Solid::Sphere { radius: 6.0 }),
            ],
        ));
        let a = arc(part.clone().rotate(2, 0.3)).mesh().unwrap();
        let Solid::Boolean { result, .. } = &*part else {
            unreachable!()
        };
        assert!(result.0.get().is_some(), "evaluated by the first mesh");
        let b = arc(part.clone().rotate(2, 0.3)).mesh().unwrap();
        assert_eq!(a, b);
        // A boolean inside a plain group is just another shell.
        let shelf = Solid::Group(vec![
            part.clone(),
            arc(Solid::Box { size: [1.0; 3] })
                .translate([20.0, 0.0, 0.0])
                .into(),
        ])
        .mesh()
        .unwrap();
        assert!((shelf.volume() - (part.mesh().unwrap().volume() + 1.0)).abs() < 1e-6);
    }

    /// Knurls placed with f32 angles (as Functor Lang's `Angle` stores them)
    /// land ~1e-7 mm off the dome's rim vertices; snapping inputs keeps the
    /// result free of slivers that f32 export would collapse.
    #[test]
    fn near_coincident_cuts_leave_no_slivers() {
        let body = arc(Solid::boolean(
            BoolOp::Intersect,
            vec![
                arc(arc(Solid::Cylinder {
                    radius: 15.0,
                    height: 28.0,
                })
                .translate([0.0, 0.0, 14.0])),
                arc(arc(Solid::Sphere { radius: 40.0 }).translate([0.0, 0.0, -26.0])),
            ],
        ));
        let knurls = (0..24)
            .map(|i| {
                let knurl = arc(arc(Solid::Cylinder {
                    radius: 1.5,
                    height: 42.0,
                })
                .translate([15.0, 0.0, 0.0]));
                let angle = (std::f64::consts::TAU * i as f64 / 24.0) as f32 as f64;
                arc(knurl.rotate(2, angle))
            })
            .collect();
        let grip = arc(Solid::boolean(BoolOp::Union, knurls));
        closed(&boolean(BoolOp::Subtract, vec![body, grip]));
    }

    #[test]
    fn segment_count_tracks_radius() {
        assert_eq!(segments_for(5.0), 52);
        assert_eq!(segments_for(0.1), 12);
        assert!(segments_for(1e6) == 512);
        assert!((12..=512).contains(&segments_for(50.0)) && segments_for(50.0) % 4 == 0);
    }
}
