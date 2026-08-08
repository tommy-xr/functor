use cgmath::{vec2, vec3, vec4};
use glow::{Buffer, HasContext, VertexArray};

use crate::render::vertex::{Vertex, VertexAttributeType};
use crate::render::VertexPositionTexture;

/// A filled convex polygon in the XY plane (z = 0), facing +Z — the geometry
/// behind `Sprite.polygon` and `Sprite.circle`.
///
/// The points are used as given, in the sprite's own coordinate space (NOT
/// re-centered), and triangulated as a fan from the first point. A fan is only
/// correct for a convex polygon, which is why `Sprite.polygon` rejects a
/// non-convex outline at construction rather than filling it wrongly.
///
/// Like [`super::HeightmapMesh`], one mesh is kept per POINT COUNT and its
/// vertex buffer is re-uploaded in place when the points differ from what is
/// currently uploaded — no per-frame VAO/VBO/EBO churn. The index buffer is a
/// function of the point count alone, so it is uploaded once and never again.
/// `Sprite.circle` exploits this deliberately: every circle lowers to the SAME
/// unit-radius ring plus a scale transform, so all circles in a frame share one
/// mesh and upload nothing.
///
/// GL objects are persistent for the scene's lifetime (never evicted), so they
/// have no `Drop` — the context frees them at teardown.
pub struct PolygonMesh {
    /// Reused CPU-side vertex scratch, recomputed in place when points change.
    vertices: Vec<VertexPositionTexture>,
    /// Fan indices — constant for the point count, so uploaded once at `create`
    /// and never re-uploaded. Retained only for the draw call's element count.
    indices: Vec<u32>,
    vao: VertexArray,
    vbo: Buffer,
    ebo: Buffer,
    /// The points currently in the VBO. Re-upload is skipped when the incoming
    /// points match EXACTLY — compared, not hashed, so no hash collision can
    /// ever skip an upload that was needed. (Every circle lowers to the same
    /// unit ring, so that case matches and uploads once for the process.)
    uploaded: Vec<[f32; 2]>,
}

impl PolygonMesh {
    /// Build and upload the fan mesh for `points.len()` vertices, seeded with
    /// `points`. The caller guarantees at least 3 points (`Sprite.polygon`
    /// rejects fewer) — `create` clamps defensively rather than panicking.
    pub fn create(gl: &glow::Context, points: &[[f32; 2]]) -> PolygonMesh {
        let count = points.len().max(3);
        let indices = build_fan_indices(count);
        let mut vertices = Vec::with_capacity(count);
        fill_vertices(&mut vertices, count, points);

        unsafe {
            let indices_u8: &[u8] = core::slice::from_raw_parts(
                indices.as_ptr() as *const u8,
                indices.len() * core::mem::size_of::<u32>(),
            );
            let counters = crate::gpu_counters::gpu_counters();
            let ebo = gl.create_buffer().unwrap();
            counters.buffer_created();
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, indices_u8, glow::STATIC_DRAW);
            counters.uploaded(indices_u8.len());

            // DYNAMIC_DRAW: re-uploaded in place whenever the points change.
            let vbo = gl.create_buffer().unwrap();
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vertices_u8 = vertices_bytes(&vertices);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices_u8, glow::DYNAMIC_DRAW);
            counters.uploaded(vertices_u8.len());

            let vao = gl.create_vertex_array().unwrap();
            counters.vao_created();
            gl.bind_vertex_array(Some(vao));

            let attributes = <VertexPositionTexture>::get_vertex_attributes();
            let total_size = <VertexPositionTexture>::get_total_size() as i32;
            for (i, attribute) in attributes.iter().enumerate() {
                let i = i as u32;
                gl.enable_vertex_attrib_array(i);
                match attribute.attribute_type {
                    VertexAttributeType::Float => {
                        gl.vertex_attrib_pointer_f32(
                            i,
                            attribute.size as i32,
                            glow::FLOAT,
                            false,
                            total_size,
                            attribute.offset as i32,
                        );
                    }
                }
            }

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);

            PolygonMesh {
                vertices,
                indices,
                vao,
                vbo,
                ebo,
                uploaded: points.to_vec(),
            }
        }
    }

    /// Re-upload the vertex buffer if `points` differ from what is uploaded.
    /// `points.len()` must equal the mesh's vertex count — guaranteed by the
    /// caller, which keys the mesh by point count.
    pub fn update(&mut self, gl: &glow::Context, points: &[[f32; 2]]) {
        if self.uploaded == points {
            return;
        }
        let count = self.vertices.len();
        fill_vertices(&mut self.vertices, count, points);
        unsafe {
            let vertices_u8 = vertices_bytes(&self.vertices);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertices_u8);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            crate::gpu_counters::gpu_counters().uploaded(vertices_u8.len());
        }
        self.uploaded.clear();
        self.uploaded.extend_from_slice(points);
    }

    pub fn draw(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ebo));
            gl.draw_elements(
                glow::TRIANGLES,
                self.indices.len() as i32,
                glow::UNSIGNED_INT,
                0,
            );
        }
    }
}

/// Fan triangles from vertex 0 — a function of the point count only.
fn build_fan_indices(count: usize) -> Vec<u32> {
    let mut indices = Vec::with_capacity((count - 2) * 3);
    for i in 1..count - 1 {
        indices.push(0);
        indices.push(i as u32);
        indices.push(i as u32 + 1);
    }
    indices
}

/// Recompute the vertex scratch in place from `points`.
///
/// UVs are normalized over the polygon's bounding box, and the tangent is the
/// CONSTANT +X with `w = 1`. That is not a shortcut: the polygon is planar in XY
/// facing +Z with axis-aligned bbox UVs, so +X/+Y/+Z is exactly the tangent
/// frame the general solver would derive — computing it per upload would be the
/// same answer for more work, on a mesh re-uploaded every frame. (Nothing samples
/// either today; filled shapes use the untextured emissive material.)
fn fill_vertices(vertices: &mut Vec<VertexPositionTexture>, count: usize, points: &[[f32; 2]]) {
    let mut min = [f32::INFINITY, f32::INFINITY];
    let mut max = [f32::NEG_INFINITY, f32::NEG_INFINITY];
    for point in points.iter().take(count) {
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    // Guard the degenerate case anyway: a 1.0 span maps every vertex to u/v 0.
    let span = [
        if max[0] > min[0] { max[0] - min[0] } else { 1.0 },
        if max[1] > min[1] { max[1] - min[1] } else { 1.0 },
    ];

    let normal = vec3(0.0, 0.0, 1.0);
    vertices.clear();
    for index in 0..count {
        let point = points.get(index).copied().unwrap_or([0.0, 0.0]);
        let mut vertex = VertexPositionTexture::new(
            vec3(point[0], point[1], 0.0),
            vec2(
                (point[0] - min[0]) / span[0],
                (point[1] - min[1]) / span[1],
            ),
            normal,
        );
        vertex.tangent = vec4(1.0, 0.0, 0.0, 1.0);
        vertices.push(vertex);
    }
}

fn vertices_bytes(vertices: &[VertexPositionTexture]) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts(
            vertices.as_ptr() as *const u8,
            std::mem::size_of_val(vertices),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fan_covers_the_polygon_with_count_minus_two_triangles() {
        assert_eq!(build_fan_indices(3), vec![0, 1, 2]);
        assert_eq!(build_fan_indices(4), vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(build_fan_indices(5).len(), 9);
        assert_eq!(build_fan_indices(32).len(), 30 * 3);
    }

    #[test]
    fn vertices_take_the_points_verbatim_and_are_not_recentered() {
        // An off-origin triangle must stay off-origin: the points ARE the
        // geometry, unlike rectangle/circle which are centered by definition.
        let points = [[10.0, 10.0], [12.0, 10.0], [10.0, 14.0]];
        let mut vertices = Vec::new();
        fill_vertices(&mut vertices, 3, &points);
        assert_eq!(vertices.len(), 3);
        for (vertex, expected) in vertices.iter().zip(points.iter()) {
            assert_eq!(vertex.position.x, expected[0]);
            assert_eq!(vertex.position.y, expected[1]);
            assert_eq!(vertex.position.z, 0.0);
        }
    }

    #[test]
    fn uvs_span_the_bounding_box_and_never_produce_nan_tangents() {
        let points = [[-2.0, -1.0], [2.0, -1.0], [2.0, 3.0], [-2.0, 3.0]];
        let mut vertices = Vec::new();
        fill_vertices(&mut vertices, 4, &points);
        let us: Vec<f32> = vertices.iter().map(|v| v.uv.x).collect();
        let vs: Vec<f32> = vertices.iter().map(|v| v.uv.y).collect();
        assert_eq!(us, vec![0.0, 1.0, 1.0, 0.0]);
        assert_eq!(vs, vec![0.0, 0.0, 1.0, 1.0]);
        for vertex in &vertices {
            assert!(vertex.tangent.x.is_finite(), "tangent must not be NaN");
            assert!(vertex.tangent.y.is_finite(), "tangent must not be NaN");
            assert!(vertex.tangent.z.is_finite(), "tangent must not be NaN");
        }
    }

    #[test]
    fn equal_points_compare_equal_so_static_shapes_skip_re_upload() {
        // The unit ring every circle lowers to must compare equal across calls,
        // which is what makes all circles share one mesh with no re-upload.
        let ring: Vec<[f32; 2]> = (0..32)
            .map(|i| {
                let a = i as f32 / 32.0 * std::f32::consts::TAU;
                [a.cos(), a.sin()]
            })
            .collect();
        assert_eq!(ring, ring.clone());
        let mut moved = ring.clone();
        moved[7][0] += 0.001;
        assert_ne!(ring, moved);
        // Order matters — a reversed outline is a different upload.
        let mut reversed = ring.clone();
        reversed.reverse();
        assert_ne!(ring, reversed);
    }
}
