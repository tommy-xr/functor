//! Frustum culling: conservative local bounds and the clip-space test.
//!
//! A draw is skipped only when its box lies wholly outside the clip volume of
//! the same `projection * view * world` its vertex shader uses. So every eye,
//! render target and shadow pass culls with its own frustum (the shadow pass
//! with the light's), and a skipped draw would have rasterized nothing.

use cgmath::{Matrix4, Vector3, Vector4};

use crate::render::VertexPositionTextureSkinned;

/// An axis-aligned box. Empty (`min > max`) when it holds no point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vector3<f32>,
    pub max: Vector3<f32>,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb {
        min: Vector3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
        max: Vector3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
    };

    pub const fn new(min: [f32; 3], max: [f32; 3]) -> Aabb {
        Aabb {
            min: Vector3::new(min[0], min[1], min[2]),
            max: Vector3::new(max[0], max[1], max[2]),
        }
    }

    pub fn is_empty(&self) -> bool {
        !(self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z)
    }

    pub fn include(&mut self, p: Vector3<f32>) {
        self.min = Vector3::new(self.min.x.min(p.x), self.min.y.min(p.y), self.min.z.min(p.z));
        self.max = Vector3::new(self.max.x.max(p.x), self.max.y.max(p.y), self.max.z.max(p.z));
    }

    pub fn union(mut self, other: &Aabb) -> Aabb {
        if !other.is_empty() {
            self.include(other.min);
            self.include(other.max);
        }
        self
    }

    /// The box around this one under the affine `m`.
    pub fn transform(&self, m: &Matrix4<f32>) -> Aabb {
        if self.is_empty() {
            return *self;
        }
        let c = (self.min + self.max) * 0.5;
        let e = (self.max - self.min) * 0.5;
        let center = (m * c.extend(1.0)).truncate();
        let row = |i: usize| m.x[i].abs() * e.x + m.y[i].abs() * e.y + m.z[i].abs() * e.z;
        let extent = Vector3::new(row(0), row(1), row(2));
        Aabb {
            min: center - extent,
            max: center + extent,
        }
    }

    /// The farthest the box reaches from the origin.
    pub fn radius(&self) -> f32 {
        let x = self.min.x.abs().max(self.max.x.abs());
        let y = self.min.y.abs().max(self.max.y.abs());
        let z = self.min.z.abs().max(self.max.z.abs());
        (x * x + y * y + z * z).sqrt()
    }
}

/// Whether `bounds` under `clip_from_local` lies wholly outside one of the
/// clip planes `-w <= x, y, z <= w`. The planes are linear in homogeneous
/// clip space, so points behind the eye (`w < 0`) need no special case. An
/// empty or non-finite box is never outside.
pub fn outside(clip_from_local: &Matrix4<f32>, bounds: &Aabb) -> bool {
    if bounds.is_empty() {
        return false;
    }
    let c = (bounds.min + bounds.max) * 0.5;
    let e = (bounds.max - bounds.min) * 0.5;
    let m = clip_from_local;
    let row = |i: usize| Vector4::new(m.x[i], m.y[i], m.z[i], m.w[i]);
    let w = row(3);
    [row(0), row(1), row(2)]
        .into_iter()
        .flat_map(|r| [w + r, w - r])
        .any(|p| {
            let center = p.x * c.x + p.y * c.y + p.z * c.z + p.w;
            let reach = p.x.abs() * e.x + p.y.abs() * e.y + p.z.abs() * e.z;
            // A relative margin, so a box touching the plane is never culled
            // by rounding.
            center + reach < -1e-5 * (center.abs() + reach)
        })
}

/// A model mesh's bounds, computed once from its vertices.
pub struct MeshBounds {
    /// Every vertex, in mesh space (the static draw).
    pub local: Aabb,
    /// Per joint, the vertices it weights, in bind space (the skinned draw).
    pub joints: Vec<Aabb>,
}

impl MeshBounds {
    pub fn of(vertices: &[VertexPositionTextureSkinned]) -> MeshBounds {
        let mut local = Aabb::EMPTY;
        let mut joints: Vec<Aabb> = Vec::new();
        for v in vertices {
            local.include(v.position);
            let slots = [
                (v.joint_indices.x, v.weights.x),
                (v.joint_indices.y, v.weights.y),
                (v.joint_indices.z, v.weights.z),
                (v.joint_indices.w, v.weights.w),
            ];
            for (joint, weight) in slots {
                if weight == 0.0 {
                    continue;
                }
                let joint = joint as usize;
                if joint >= joints.len() {
                    joints.resize(joint + 1, Aabb::EMPTY);
                }
                joints[joint].include(v.position);
            }
        }
        MeshBounds { local, joints }
    }

    /// The posed mesh's bounds under the skinning `palette`. A skinned vertex
    /// is a weighted average of its joints' transforms of it, so it lies in
    /// the hull of those joints' transformed boxes. `None` (draw) when a
    /// joint is missing from the palette.
    pub fn posed(&self, palette: &[Matrix4<f32>]) -> Option<Aabb> {
        let mut out = Aabb::EMPTY;
        for (joint, bounds) in self.joints.iter().enumerate() {
            if bounds.is_empty() {
                continue;
            }
            out = out.union(&bounds.transform(palette.get(joint)?));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{perspective, Deg, Point3, SquareMatrix};

    fn clip() -> Matrix4<f32> {
        // Eye at the origin looking down -Z.
        perspective(Deg(60.0), 1.0, 0.1, 100.0)
            * Matrix4::look_at_rh(
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(0.0, 0.0, -1.0),
                Vector3::unit_y(),
            )
    }

    fn unit_at(x: f32, y: f32, z: f32) -> Matrix4<f32> {
        Matrix4::from_translation(Vector3::new(x, y, z))
    }

    const CUBE: Aabb = Aabb::new([-0.5; 3], [0.5; 3]);

    #[test]
    fn in_front_is_kept_and_behind_is_culled() {
        assert!(!outside(&(clip() * unit_at(0.0, 0.0, -5.0)), &CUBE));
        assert!(outside(&(clip() * unit_at(0.0, 0.0, 5.0)), &CUBE));
        // Beyond the far plane and far off to the side.
        assert!(outside(&(clip() * unit_at(0.0, 0.0, -200.0)), &CUBE));
        assert!(outside(&(clip() * unit_at(50.0, 0.0, -5.0)), &CUBE));
    }

    #[test]
    fn a_box_straddling_the_eye_is_kept() {
        assert!(!outside(&(clip() * Matrix4::from_scale(10.0)), &CUBE));
    }

    #[test]
    fn a_box_touching_a_plane_is_kept() {
        // The right plane at depth 5 is x = 5 tan(30°); the quad's left edge
        // sits exactly on it.
        let quad = Aabb::new([-0.5, -0.5, 0.0], [0.5, 0.5, 0.0]);
        let edge = 5.0 * (30.0f32).to_radians().tan();
        assert!(!outside(&(clip() * unit_at(edge + 0.5, 0.0, -5.0)), &quad));
        assert!(outside(&(clip() * unit_at(edge + 0.51, 0.0, -5.0)), &quad));
    }

    #[test]
    fn empty_and_nan_boxes_are_never_culled() {
        assert!(!outside(&clip(), &Aabb::EMPTY));
        let nan = Aabb::new([f32::NAN; 3], [f32::NAN; 3]);
        assert!(!outside(&(clip() * unit_at(0.0, 0.0, 5.0)), &nan));
    }

    #[test]
    fn transform_bounds_a_rotated_box() {
        let m = Matrix4::from_angle_y(Deg(45.0));
        let b = CUBE.transform(&m);
        let half = 0.5 * 2f32.sqrt();
        assert!((b.max.x - half).abs() < 1e-5 && (b.min.z + half).abs() < 1e-5);
        assert!((b.max.y - 0.5).abs() < 1e-6);
    }

    fn vertex(p: [f32; 3], joints: [f32; 4], weights: [f32; 4]) -> VertexPositionTextureSkinned {
        VertexPositionTextureSkinned {
            position: p.into(),
            uv: cgmath::vec2(0.0, 0.0),
            normal: cgmath::vec3(0.0, 1.0, 0.0),
            tangent: cgmath::vec4(1.0, 0.0, 0.0, 1.0),
            joint_indices: joints.into(),
            weights: weights.into(),
        }
    }

    #[test]
    fn posed_bounds_follow_each_joint() {
        let bounds = MeshBounds::of(&[
            vertex([0.0, 0.0, 0.0], [0.0; 4], [1.0, 0.0, 0.0, 0.0]),
            vertex([0.0, 1.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
            vertex([0.0, 2.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]),
        ]);
        assert_eq!(bounds.local, Aabb::new([0.0; 3], [0.0, 2.0, 0.0]));
        // Joint 1 moves its vertices 10 m along X; joint 0 stays put.
        let palette = [Matrix4::identity(), unit_at(10.0, 0.0, 0.0)];
        let posed = bounds.posed(&palette).unwrap();
        assert_eq!(posed, Aabb::new([0.0; 3], [10.0, 2.0, 0.0]));
        // A joint past the palette: unknown, so the caller draws.
        assert!(bounds.posed(&palette[..1]).is_none());
    }
}
