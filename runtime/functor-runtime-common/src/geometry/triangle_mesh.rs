use cgmath::{vec2, Vector3};
use glow::{Buffer, HasContext, VertexArray};

use crate::render::vertex::{Vertex, VertexAttributeType};
use crate::render::VertexPositionTexture;
use crate::scene3d::MeshData;

/// GPU buffers for a [`MeshData`] (`Shape::Mesh`), flat-shaded: each triangle
/// gets its own three vertices carrying its face normal, so faceted solids
/// read as crisp CAD surfaces instead of smoothed blobs.
///
/// Unlike the per-size meshes, these are evicted once undrawn for a frame
/// (e.g. a hot reload rebuilt the part), so [`TriangleMesh::delete`] frees the
/// GL objects explicitly — there is no `Drop`, which would have no context.
pub struct TriangleMesh {
    vao: VertexArray,
    vbo: Buffer,
    count: i32,
}

impl TriangleMesh {
    pub fn create(gl: &glow::Context, mesh: &MeshData) -> TriangleMesh {
        let vertices: Vec<VertexPositionTexture> = mesh
            .triangles
            .iter()
            .flat_map(|tri| {
                let [a, b, c] = tri.map(|i| Vector3::from(mesh.positions[i as usize]));
                let normal = functor_solid::face_normal(a, b, c);
                [a, b, c].map(|p| VertexPositionTexture::new(p, vec2(0.0, 0.0), normal))
            })
            .collect();
        unsafe {
            let bytes = crate::terrain_renderer::slice_bytes(&vertices);
            let counters = crate::gpu_counters::gpu_counters();
            let vbo = gl.create_buffer().unwrap();
            counters.buffer_created();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            counters.uploaded(bytes.len());

            let vao = gl.create_vertex_array().unwrap();
            counters.vao_created();
            gl.bind_vertex_array(Some(vao));
            let stride = VertexPositionTexture::get_total_size() as i32;
            for (i, attribute) in VertexPositionTexture::get_vertex_attributes()
                .iter()
                .enumerate()
            {
                gl.enable_vertex_attrib_array(i as u32);
                match attribute.attribute_type {
                    VertexAttributeType::Float => gl.vertex_attrib_pointer_f32(
                        i as u32,
                        attribute.size as i32,
                        glow::FLOAT,
                        false,
                        stride,
                        attribute.offset as i32,
                    ),
                }
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);
            TriangleMesh {
                vao,
                vbo,
                count: vertices.len() as i32,
            }
        }
    }

    pub fn draw(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, self.count);
        }
    }

    pub fn delete(self, gl: &glow::Context) {
        let counters = crate::gpu_counters::gpu_counters();
        unsafe {
            gl.delete_vertex_array(self.vao);
            counters.vao_deleted();
            gl.delete_buffer(self.vbo);
            counters.buffer_deleted();
        }
    }
}
