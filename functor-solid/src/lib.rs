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
//! [`Mesh::shared_positions`]).

mod mesh;
pub mod stl;

use std::sync::Arc;

use cgmath::{Matrix4, Rad, SquareMatrix, Vector3};

pub use mesh::{Bounds, Mesh};

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
    Transform {
        matrix: Matrix4<f64>,
        child: Arc<Solid>,
    },
    /// Pieces placed side by side. Overlaps are NOT merged — each piece stays
    /// its own closed shell (booleans are a separate operation).
    Group(Vec<Arc<Solid>>),
}

impl Solid {
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

    /// Triangulate the whole tree.
    pub fn mesh(&self) -> Mesh {
        let mut out = Mesh::default();
        self.append_to(&mut out, Matrix4::identity());
        out
    }

    fn append_to(&self, out: &mut Mesh, xform: Matrix4<f64>) {
        // Transforms are rigid plus uniform scale, so |det| = scale³. Facets
        // are counted on the radius as it will print, not as written.
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
            Solid::Transform { matrix, child } => child.append_to(out, xform * matrix),
            Solid::Group(children) => {
                for child in children {
                    child.append_to(out, xform);
                }
            }
        }
    }
}

/// Facets around a circle of `radius` so no chord sags more than
/// [`CHORD_TOLERANCE`]; a multiple of 4 so circles stay symmetric on both axes.
/// E.g. r = 5 mm → 52, r = 50 mm → 160.
fn segments_for(radius: f64) -> usize {
    let ratio = (1.0 - CHORD_TOLERANCE / radius).clamp(-1.0, 1.0);
    let n = (std::f64::consts::PI / ratio.acos()).ceil() as usize;
    (n.clamp(12, 512) + 3) / 4 * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(solid: Solid) -> Arc<Solid> {
        Arc::new(solid)
    }

    #[test]
    fn cube_is_twelve_closed_triangles_of_exact_volume() {
        let mesh = Solid::Box { size: [10.0; 3] }.mesh();
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
        .mesh();
        assert_eq!(cyl.non_manifold_edges(), 0);
        let exact = PI * 25.0 * 10.0;
        assert!(
            (cyl.volume() - exact).abs() / exact < 0.005,
            "{}",
            cyl.volume()
        );

        let sphere = Solid::Sphere { radius: 5.0 }.mesh();
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
        let mesh = turned.translate([10.0, 0.0, 3.0]).mesh();
        let b = mesh.bounds().unwrap();
        let close = |a: [f64; 3], e: [f64; 3]| a.iter().zip(e).all(|(a, e)| (a - e).abs() < 1e-9);
        assert!(close(b.min, [8.0, -1.0, 0.0]), "{:?}", b.min);
        assert!(close(b.max, [12.0, 1.0, 6.0]), "{:?}", b.max);
        assert!((mesh.volume() - 48.0).abs() < 1e-9);
        assert_eq!(mesh.non_manifold_edges(), 0);
    }

    #[test]
    fn scale_multiplies_volume_by_cube_of_factor() {
        let mesh = arc(Solid::Box { size: [1.0; 3] }).scale(3.0).mesh();
        assert!((mesh.volume() - 27.0).abs() < 1e-9);
    }

    #[test]
    fn group_keeps_each_piece_closed() {
        let a = arc(Solid::Box { size: [1.0; 3] });
        let b = arc(arc(Solid::Sphere { radius: 1.0 }).translate([5.0, 0.0, 0.0]));
        let mesh = Solid::Group(vec![a, b]).mesh();
        assert_eq!(mesh.non_manifold_edges(), 0);
        assert!(mesh.volume() > 1.0);
    }

    #[test]
    fn pieces_touching_at_a_face_or_corner_share_positions() {
        let cube = || arc(Solid::Box { size: [2.0; 3] });
        let face = Solid::Group(vec![cube(), arc(cube().translate([2.0, 0.0, 0.0]))]).mesh();
        assert_eq!(face.non_manifold_edges(), 0);
        assert_eq!(face.shared_positions(), 4);
        let corner = Solid::Group(vec![cube(), arc(cube().translate([2.0, 2.0, 2.0]))]).mesh();
        assert_eq!(corner.shared_positions(), 1);
        let apart = Solid::Group(vec![cube(), arc(cube().translate([2.5, 0.0, 0.0]))]).mesh();
        assert_eq!(apart.shared_positions(), 0);
    }

    #[test]
    fn mirroring_scale_keeps_triangles_facing_outward() {
        let mesh = arc(Solid::Box { size: [2.0; 3] }).scale(-1.0).mesh();
        assert!((mesh.volume() - 8.0).abs() < 1e-9, "{}", mesh.volume());
        assert_eq!(mesh.non_manifold_edges(), 0);
    }

    #[test]
    fn scaled_curves_keep_their_facet_tolerance() {
        use std::f64::consts::PI;
        let mesh = arc(Solid::Sphere { radius: 1.0 }).scale(100.0).mesh();
        let exact = 4.0 / 3.0 * PI * 1e6;
        assert!(
            (mesh.volume() - exact).abs() / exact < 0.01,
            "{}",
            mesh.volume()
        );
    }

    #[test]
    fn segment_count_tracks_radius() {
        assert_eq!(segments_for(5.0), 52);
        assert_eq!(segments_for(0.1), 12);
        assert!(segments_for(1e6) == 512);
        assert!((12..=512).contains(&segments_for(50.0)) && segments_for(50.0) % 4 == 0);
    }
}
