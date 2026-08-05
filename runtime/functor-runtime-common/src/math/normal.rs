use cgmath::{Matrix, Matrix3, Matrix4, SquareMatrix};

/// The normal matrix for a world transform: `transpose(inverse(mat3(world)))`.
///
/// Normals are covectors — under a NON-UNIFORM scale, transforming them with
/// the plain linear part tilts them the wrong way (a `scaleXYZ(240, 1, 240)`
/// ground plane's gentle slopes come out nearly horizontal, which kills the
/// key light and reads as ambient-only). The inverse-transpose is the correct
/// transform; for rotations (and, after normalizing, uniform scales) it is the
/// same matrix, so this is a no-op there.
///
/// A singular linear part (a zero scale on some axis) has no inverse; that
/// degenerate transform falls back to the identity rather than producing NaNs
/// — the geometry is flattened to a plane or a line either way. (The cofactor
/// matrix would keep a meaningful normal for the rank-two case, but it also
/// flips the sign under a mirroring scale, so it is a separate change.)
pub fn normal_matrix(world: &Matrix4<f32>) -> Matrix3<f32> {
    let linear = Matrix3::from_cols(world.x.truncate(), world.y.truncate(), world.z.truncate());
    linear
        .invert()
        .map(|matrix| matrix.transpose())
        .unwrap_or_else(Matrix3::identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Deg, InnerSpace, Vector3};

    #[test]
    fn non_uniform_scale_uses_the_inverse_transpose() {
        // The reported bug: a unit-space slope under `scaleXYZ(240, 1, 240)`.
        // The world-space normal must FLATTEN toward +Y (the slope gets 240x
        // wider, not 240x steeper) — the plain linear part does the opposite.
        let world = Matrix4::from_nonuniform_scale(240.0, 1.0, 240.0);
        let local = Vector3::new(-0.2f32, 1.0, -0.1).normalize();

        let actual = (normal_matrix(&world) * local).normalize();
        let expected = Vector3::new(-0.2 / 240.0, 1.0, -0.1 / 240.0).normalize();
        assert!((actual - expected).magnitude() < 1e-6, "got {actual:?}");

        // And it really is different from the naive transform — the naive one
        // tilts this normal ~90 degrees off, to a near-horizontal normal.
        let naive = Vector3::new(-0.2 * 240.0, 1.0, -0.1 * 240.0).normalize();
        assert!(
            actual.dot(naive) < 0.1,
            "the naive transform is not the bug"
        );
    }

    #[test]
    fn rotation_is_unchanged() {
        let world = Matrix4::from_angle_y(Deg(37.0)) * Matrix4::from_angle_x(Deg(-19.0));
        let local = Vector3::new(0.3f32, 0.5, -0.8).normalize();

        let actual = normal_matrix(&world) * local;
        let expected = (world * local.extend(0.0)).truncate();
        assert!((actual - expected).magnitude() < 1e-5);
    }

    #[test]
    fn uniform_scale_and_translation_only_rescale() {
        let world =
            Matrix4::from_translation(Vector3::new(4.0, -2.0, 9.0)) * Matrix4::from_scale(3.0);
        let local = Vector3::new(0.0f32, 1.0, 0.0);

        let actual = (normal_matrix(&world) * local).normalize();
        assert!((actual - local).magnitude() < 1e-6);
    }

    #[test]
    fn a_singular_transform_falls_back_to_identity() {
        let world = Matrix4::from_nonuniform_scale(1.0, 0.0, 1.0);
        let local = Vector3::new(0.0f32, 1.0, 0.0);
        assert_eq!(normal_matrix(&world) * local, local);
    }
}
