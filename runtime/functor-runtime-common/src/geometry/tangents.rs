use cgmath::{InnerSpace, Vector2, Vector3, Vector4};

use crate::render::{VertexPositionTexture, VertexPositionTextureSkinned};

/// A vertex whose tangent frame can be derived from its position/uv/normal.
/// Implemented for both vertex formats so [`compute_tangents`] is shared by the
/// primitives and the glTF tangent fallback.
pub trait TangentVertex {
    fn position(&self) -> Vector3<f32>;
    fn uv(&self) -> Vector2<f32>;
    fn normal(&self) -> Vector3<f32>;
    fn set_tangent(&mut self, tangent: Vector4<f32>);
}

impl TangentVertex for VertexPositionTexture {
    fn position(&self) -> Vector3<f32> {
        self.position
    }
    fn uv(&self) -> Vector2<f32> {
        self.uv
    }
    fn normal(&self) -> Vector3<f32> {
        self.normal
    }
    fn set_tangent(&mut self, tangent: Vector4<f32>) {
        self.tangent = tangent;
    }
}

impl TangentVertex for VertexPositionTextureSkinned {
    fn position(&self) -> Vector3<f32> {
        self.position
    }
    fn uv(&self) -> Vector2<f32> {
        self.uv
    }
    fn normal(&self) -> Vector3<f32> {
        self.normal
    }
    fn set_tangent(&mut self, tangent: Vector4<f32>) {
        self.tangent = tangent;
    }
}

/// Fill in each vertex's tangent from the mesh's positions, uvs, and normals
/// (Lengyel's method): accumulate a per-vertex tangent/bitangent frame from the
/// UV gradient over each triangle, then Gram-Schmidt-orthogonalize the tangent
/// against the normal and record the bitangent handedness in `tangent.w`.
///
/// Used for primitives (which author positions/uvs/normals but no tangent) and
/// as the fallback for glTF meshes lacking a `TANGENT` attribute. Degenerate UVs
/// (zero-area in texture space) fall back to an arbitrary axis perpendicular to
/// the normal, so the frame is always finite.
pub fn compute_tangents<V: TangentVertex>(vertices: &mut [V], indices: &[u32]) {
    let mut tan = vec![Vector3::new(0.0, 0.0, 0.0); vertices.len()];
    let mut bitan = vec![Vector3::new(0.0, 0.0, 0.0); vertices.len()];

    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let p0 = vertices[i0].position();
        let p1 = vertices[i1].position();
        let p2 = vertices[i2].position();
        let w0 = vertices[i0].uv();
        let w1 = vertices[i1].uv();
        let w2 = vertices[i2].uv();

        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let du1 = w1.x - w0.x;
        let dv1 = w1.y - w0.y;
        let du2 = w2.x - w0.x;
        let dv2 = w2.y - w0.y;

        let det = du1 * dv2 - du2 * dv1;
        // Skip zero-area UV triangles; their per-vertex frame comes from the
        // other triangles, or the perpendicular fallback below.
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let t = (e1 * dv2 - e2 * dv1) * r;
        let b = (e2 * du1 - e1 * du2) * r;

        for &i in &[i0, i1, i2] {
            tan[i] += t;
            bitan[i] += b;
        }
    }

    for (v, (t, b)) in vertices.iter_mut().zip(tan.iter().zip(bitan.iter())) {
        let n = v.normal();
        // Gram-Schmidt: project the accumulated tangent onto the plane of n.
        let mut tangent = *t - n * n.dot(*t);
        if tangent.magnitude2() < 1e-12 {
            // Degenerate (no/zero UV gradient): pick any axis ⟂ to the normal.
            let axis = if n.x.abs() < 0.9 {
                Vector3::new(1.0, 0.0, 0.0)
            } else {
                Vector3::new(0.0, 1.0, 0.0)
            };
            tangent = (axis - n * n.dot(axis)).normalize();
        } else {
            tangent = tangent.normalize();
        }
        // Handedness: +1 unless the bitangent is flipped relative to n×t.
        let handedness = if n.cross(tangent).dot(*b) < 0.0 {
            -1.0
        } else {
            1.0
        };
        v.set_tangent(Vector4::new(tangent.x, tangent.y, tangent.z, handedness));
    }
}
