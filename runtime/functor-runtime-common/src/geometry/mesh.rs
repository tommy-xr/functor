use glow::{HasContext, VertexArray};

use crate::asset::{RenderableAsset, RuntimeRenderableAsset};

use super::Geometry;

pub struct RuntimeMeshData {
    vao: VertexArray,
    triangle_count: i32,
}

pub struct MeshData {
    vertices: Vec<f32>,
}

impl MeshData {
    fn create(vertices: Vec<f32>) -> MeshData {
        MeshData { vertices }
    }
}

pub struct Mesh {
    ora: RuntimeRenderableAsset<MeshData>,
}

impl Mesh {
    pub fn create(vertices: Vec<f32>) -> Mesh {
        Mesh {
            ora: RuntimeRenderableAsset::new(MeshData::create(vertices), ()),
        }
    }
}

impl RenderableAsset for MeshData {
    type HydratedType = RuntimeMeshData;
    type OptionsType = ();

    fn hydrate(&self, gl: &glow::Context, _options: &Self::OptionsType) -> Self::HydratedType {
        let vao = unsafe {
            let vertices_u8: &[u8] = core::slice::from_raw_parts(
                self.vertices.as_ptr() as *const u8,
                self.vertices.len() * core::mem::size_of::<f32>(),
            );

            let vbo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices_u8, glow::STATIC_DRAW);

            let vao = gl.create_vertex_array().unwrap();
            gl.bind_vertex_array(Some(vao));

            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(
                0,
                3,
                glow::FLOAT,
                false,
                (5 * core::mem::size_of::<f32>()) as i32,
                0,
            );

            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(
                1,
                2,
                glow::FLOAT,
                false,
                (5 * core::mem::size_of::<f32>()) as i32,
                (3 * core::mem::size_of::<f32>()) as i32,
            );

            // You can unbind the VAO afterwards so other VAO calls won't accidentally modify this VAO, but this rarely happens. Modifying other
            // VAOs requires a call to glBindVertexArray anyways so we generally don't unbind VAOs (nor VBOs) when it's not directly necessary.
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);
            vao
        };

        RuntimeMeshData {
            vao,
            triangle_count: (self.vertices.len() / 5) as i32,
        }
    }
}

impl Geometry for Mesh {
    fn draw(&mut self, gl: &glow::Context) {
        let hydrated_info = self.ora.get(gl);

        unsafe {
            gl.bind_vertex_array(Some(hydrated_info.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, hydrated_info.triangle_count);
        }
    }
}
