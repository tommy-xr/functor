use glow::{Buffer, HasContext, VertexArray};

use crate::{
    asset::{RenderableAsset, RuntimeRenderableAsset},
    render::vertex::{Vertex, VertexAttributeType},
};

use super::Geometry;

pub struct IndexedMesh<T: Vertex> {
    ora: RuntimeRenderableAsset<IndexedMeshData<T>>,
}

pub struct IndexedMeshData<T: Vertex> {
    vertices: Vec<T>,
    indices: Vec<u32>,
}

pub struct IndexedMeshRuntimeData {
    vao: VertexArray,
    ebo: Buffer,
    len: i32,
}

impl<T: Vertex> IndexedMesh<T> {
    pub fn create(vertices: Vec<T>, indices: Vec<u32>) -> IndexedMesh<T> {
        let data = IndexedMeshData { vertices, indices };
        let ora = RuntimeRenderableAsset::new(data, ());

        IndexedMesh { ora }
    }
}

impl<T: Vertex> RenderableAsset for IndexedMeshData<T> {
    type HydratedType = IndexedMeshRuntimeData;
    type OptionsType = ();

    fn hydrate(&self, gl: &glow::Context, _options: &Self::OptionsType) -> Self::HydratedType {
        unsafe {
            let len = self.indices.len() as i32;
            let vertices_u8: &[u8] = core::slice::from_raw_parts(
                self.vertices.as_ptr() as *const u8,
                self.vertices.len() * T::get_total_size(),
            );

            let indices_u8: &[u8] = core::slice::from_raw_parts(
                self.indices.as_ptr() as *const u8,
                self.indices.len() * core::mem::size_of::<u32>(),
            );
            let ebo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, indices_u8, glow::STATIC_DRAW);

            let vbo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices_u8, glow::STATIC_DRAW);

            let vao = gl.create_vertex_array().unwrap();
            gl.bind_vertex_array(Some(vao));

            let attributes = <T>::get_vertex_attributes();
            let attr_len = attributes.len() as u32;
            let total_size = <T>::get_total_size() as i32;

            for i in 0..attr_len {
                let attribute = &attributes[i as usize];

                match attribute.attribute_type {
                    VertexAttributeType::Float => {
                        gl.vertex_attrib_pointer_f32(
                            i as u32,
                            attribute.size as i32,
                            glow::FLOAT,
                            false,
                            total_size as i32,
                            attribute.offset as i32,
                        );
                    }
                    // Handle other attribute types here
                    _ => panic!("Unsupported attribute type"),
                }
            }

            // You can unbind the VAO afterwards so other VAO calls won't accidentally modify this VAO, but this rarely happens. Modifying other
            // VAOs requires a call to glBindVertexArray anyways so we generally don't unbind VAOs (nor VBOs) when it's not directly necessary.
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);
            IndexedMeshRuntimeData { vao, ebo, len }
        }
    }
}

impl<T: Vertex> Geometry for IndexedMesh<T> {
    fn draw(&self, gl: &glow::Context) {
        let ctx = self.ora.get(gl);
        unsafe {
            gl.bind_vertex_array(Some(ctx.vao));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ctx.ebo));
            gl.draw_elements(glow::TRIANGLES, ctx.len, glow::UNSIGNED_INT, 0);
        }
    }
}
