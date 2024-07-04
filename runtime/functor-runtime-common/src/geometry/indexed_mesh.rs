use glow::{Buffer, HasContext, VertexArray};

use crate::render::vertex::Vertex;

use super::Geometry;

struct HydratedContext {
    vao: VertexArray,
    ebo: Buffer,
}

pub struct IndexedMesh<T: Vertex> {
    vertices: Vec<T>,
    indices: Vec<u32>,

    hydrated_context: Option<HydratedContext>,
}

pub fn create<T: Vertex>(vertices: Vec<T>, indices: Vec<u32>) -> IndexedMesh<T> {
    IndexedMesh {
        vertices,
        indices,
        hydrated_context: None,
    }
}

impl<T: Vertex> Geometry for IndexedMesh<T> {
    fn draw(&mut self, gl: &glow::Context) {
        if self.hydrated_context.is_none() {
            let (vao, ebo) = unsafe {
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
                (vao, ebo)
            };

            self.hydrated_context = Some(HydratedContext { vao, ebo });
        }

        match &self.hydrated_context {
            Some(ctx) => {
                // We're ready to go!
                unsafe {
                    gl.bind_vertex_array(Some(ctx.vao));
                    gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ctx.ebo));
                    gl.draw_elements(
                        glow::TRIANGLES,
                        self.indices.len() as i32,
                        glow::UNSIGNED_INT,
                        0,
                    );
                }
            }
            None => {
                println!("Failure initializing");
            }
        }
    }
}
