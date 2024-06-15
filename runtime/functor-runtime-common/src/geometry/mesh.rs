use glow::{HasContext, VertexArray};

use super::Geometry;

struct HydratedContext {
    vao: VertexArray,
    triangle_count: i32,
}

pub struct Mesh {
    vertices: Vec<f32>,

    hydrated_context: Option<HydratedContext>,
}

pub fn create(vertices: Vec<f32>) -> Mesh {
    Mesh {
        vertices,
        hydrated_context: None,
    }
}

impl Geometry for Mesh {
    fn draw(&mut self, gl: &glow::Context) {
        if self.hydrated_context.is_none() {
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

            self.hydrated_context = Some(HydratedContext {
                vao,
                triangle_count: (self.vertices.len() / 5) as i32,
            });
        }

        match &self.hydrated_context {
            Some(ctx) => {
                // We're ready to go!
                unsafe {
                    gl.bind_vertex_array(Some(ctx.vao));
                    gl.draw_arrays(glow::TRIANGLES, 0, ctx.triangle_count);
                }
            }
            None => {
                println!("Failure initializing");
            }
        }
    }
}
