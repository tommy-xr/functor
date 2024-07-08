pub trait Geometry {
    fn draw(&self, gl: &glow::Context);
}

mod cube;
mod cylinder;
mod empty_mesh;
mod indexed_mesh;
mod mesh;
mod sphere;
pub use cube::*;
pub use cylinder::*;
pub use empty_mesh::*;
pub use indexed_mesh::*;
pub use mesh::*;
pub use sphere::*;

// pub mod plane {
//     use glow::{HasContext, Buffer, VertexArray};
//     use once_cell::sync::OnceCell;

//     use super::Geometry;

//     static CUBE_GEOMETRY: OnceCell<(NativeBuffer, NativeVertexArray)> = OnceCell::new();

//     pub struct Plane {}

//     pub fn create() -> Plane {
//         Plane {}
//     }

//     impl Geometry for Plane {
//         fn draw(&mut self, gl: &glow::Context) {
//             let (_vbo, vao) = *CUBE_GEOMETRY.get_or_init(|| {
//                 let uv_scale = 100.0;
//                 let vertices: [f32; 30] = [
//                     -0.5, 0.0, -0.5, 0.0, 0.0, 0.5, 0.0, -0.5, uv_scale, 0.0, 0.5, 0.0, 0.5,
//                     uv_scale, uv_scale, -0.5, 0.0, -0.5, 0.0, 0.0, -0.5, 0.0, 0.5, 0.0, uv_scale,
//                     0.5, 0.0, 0.5, uv_scale, uv_scale,
//                 ];

//                 let (vbo, vao) = unsafe {
//                     let vertices_u8: &[u8] = core::slice::from_raw_parts(
//                         vertices.as_ptr() as *const u8,
//                         vertices.len() * core::mem::size_of::<f32>(),
//                     );

//                     let vbo = gl.create_buffer().unwrap();
//                     gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
//                     gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices_u8, glow::STATIC_DRAW);

//                     let vao = gl.create_vertex_array().unwrap();
//                     gl.bind_vertex_array(Some(vao));

//                     gl.enable_vertex_attrib_array(0);
//                     gl.vertex_attrib_pointer_f32(
//                         0,
//                         3,
//                         glow::FLOAT,
//                         false,
//                         (5 * core::mem::size_of::<f32>()) as i32,
//                         0,
//                     );

//                     gl.enable_vertex_attrib_array(1);
//                     gl.vertex_attrib_pointer_f32(
//                         1,
//                         2,
//                         glow::FLOAT,
//                         false,
//                         (5 * core::mem::size_of::<f32>()) as i32,
//                         3,
//                     );

//                     // You can unbind the VAO afterwards so other VAO calls won't accidentally modify this VAO, but this rarely happens. Modifying other
//                     // VAOs requires a call to glBindVertexArray anyways so we generally don't unbind VAOs (nor VBOs) when it's not directly necessary.
//                     gl.bind_buffer(glow::ARRAY_BUFFER, None);
//                     gl.bind_vertex_array(None);
//                     (vbo, vao)
//                 };

//                 (vbo, vao)
//             });

//             unsafe {
//                 gl.bind_vertex_array(Some(vao));
//                 gl.draw_arrays(glow::TRIANGLES, 0, 6);
//             }
//         }
//     }
// }
