use glow::*;

pub trait Geometry {
    fn draw(&self, gl: &glow::Context);
}

pub struct EmptyMesh;

impl Geometry for EmptyMesh {
    fn draw(&self, _gl: &glow::Context) {
        // do nothing, the mesh is empty
    }
}

pub mod mesh {
    use glow::{HasContext, NativeBuffer, NativeVertexArray};
    use once_cell::sync::OnceCell;

    use super::Geometry;

    static CUBE_GEOMETRY: OnceCell<(NativeBuffer, NativeVertexArray)> = OnceCell::new();

    pub struct Mesh {
        vertices: Vec<f32>,
    }

    pub fn create(vertices: Vec<f32>) -> Mesh {
        Mesh { vertices }
    }

    impl Geometry for Mesh {
        fn draw(&self, gl: &glow::Context) {
            let (_vbo, vao) = *CUBE_GEOMETRY.get_or_init(|| {
                let vertices = self.vertices.as_slice();

                let (vbo, vao) = unsafe {
                    let vertices_u8: &[u8] = core::slice::from_raw_parts(
                        vertices.as_ptr() as *const u8,
                        vertices.len() * core::mem::size_of::<f32>(),
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
                        3,
                    );

                    // You can unbind the VAO afterwards so other VAO calls won't accidentally modify this VAO, but this rarely happens. Modifying other
                    // VAOs requires a call to glBindVertexArray anyways so we generally don't unbind VAOs (nor VBOs) when it's not directly necessary.
                    gl.bind_buffer(glow::ARRAY_BUFFER, None);
                    gl.bind_vertex_array(None);
                    (vbo, vao)
                };

                (vbo, vao)
            });

            unsafe {
                gl.bind_vertex_array(Some(vao));
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
            }
        }
    }
}

pub mod cube {
    use super::mesh::{self, Mesh};

    pub fn create() -> Mesh {
        mesh::create(vec![
            -0.5, -0.5, -0.5, 0.0, 0.0, 0.5, -0.5, -0.5, 1.0, 0.0, 0.5, 0.5, -0.5, 1.0, 1.0, 0.5,
            0.5, -0.5, 1.0, 1.0, -0.5, 0.5, -0.5, 0.0, 1.0, -0.5, -0.5, -0.5, 0.0, 0.0, -0.5, -0.5,
            0.5, 0.0, 0.0, 0.5, -0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 0.5, 0.5, 0.5, 1.0,
            1.0, -0.5, 0.5, 0.5, 0.0, 1.0, -0.5, -0.5, 0.5, 0.0, 0.0, -0.5, 0.5, 0.5, 1.0, 0.0,
            -0.5, 0.5, -0.5, 1.0, 1.0, -0.5, -0.5, -0.5, 0.0, 1.0, -0.5, -0.5, -0.5, 0.0, 1.0,
            -0.5, -0.5, 0.5, 0.0, 0.0, -0.5, 0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 1.0, 0.0, 0.5, 0.5,
            -0.5, 1.0, 1.0, 0.5, -0.5, -0.5, 0.0, 1.0, 0.5, -0.5, -0.5, 0.0, 1.0, 0.5, -0.5, 0.5,
            0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 0.0, -0.5, -0.5, -0.5, 0.0, 1.0, 0.5, -0.5, -0.5, 1.0,
            1.0, 0.5, -0.5, 0.5, 1.0, 0.0, 0.5, -0.5, 0.5, 1.0, 0.0, -0.5, -0.5, 0.5, 0.0, 0.0,
            -0.5, -0.5, -0.5, 0.0, 1.0, -0.5, 0.5, -0.5, 0.0, 1.0, 0.5, 0.5, -0.5, 1.0, 1.0, 0.5,
            0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 1.0, 0.0, -0.5, 0.5, 0.5, 0.0, 0.0, -0.5, 0.5, -0.5,
            0.0, 1.0,
        ])
    }
}

pub mod plane {
    use glow::{HasContext, NativeBuffer, NativeVertexArray};
    use once_cell::sync::OnceCell;

    use super::Geometry;

    static CUBE_GEOMETRY: OnceCell<(NativeBuffer, NativeVertexArray)> = OnceCell::new();

    pub struct Plane {}

    pub fn create() -> Plane {
        Plane {}
    }

    impl Geometry for Plane {
        fn draw(&self, gl: &glow::Context) {
            let (_vbo, vao) = *CUBE_GEOMETRY.get_or_init(|| {
                let uv_scale = 100.0;
                let vertices: [f32; 30] = [
                    -0.5, 0.0, -0.5, 0.0, 0.0, 0.5, 0.0, -0.5, uv_scale, 0.0, 0.5, 0.0, 0.5,
                    uv_scale, uv_scale, -0.5, 0.0, -0.5, 0.0, 0.0, -0.5, 0.0, 0.5, 0.0, uv_scale,
                    0.5, 0.0, 0.5, uv_scale, uv_scale,
                ];

                let (vbo, vao) = unsafe {
                    let vertices_u8: &[u8] = core::slice::from_raw_parts(
                        vertices.as_ptr() as *const u8,
                        vertices.len() * core::mem::size_of::<f32>(),
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
                        3,
                    );

                    // You can unbind the VAO afterwards so other VAO calls won't accidentally modify this VAO, but this rarely happens. Modifying other
                    // VAOs requires a call to glBindVertexArray anyways so we generally don't unbind VAOs (nor VBOs) when it's not directly necessary.
                    gl.bind_buffer(glow::ARRAY_BUFFER, None);
                    gl.bind_vertex_array(None);
                    (vbo, vao)
                };

                (vbo, vao)
            });

            unsafe {
                gl.bind_vertex_array(Some(vao));
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
            }
        }
    }
}

// pub mod cube {
//     use glow::NativeBuffer;
//     use once_cell::sync::OnceCell;

//     static CUBE_GEOMETRY: OnceCell<NativeBuffer> = OnceCell::new();

//     pub struct Cube {}

//     pub fn create() -> Cube {
//         Cube {}
//     }

//     impl Geometry for Cube {
//         fn draw(&self, gl: &glow::Context) {
//             let vao = *CUBE_GEOMETRY.get_or_init(|| {
//                 let vertices: [f32; 180] = [
//                     -0.5, -0.5, -0.5, 0.0, 0.0, 0.5, -0.5, -0.5, 1.0, 0.0, 0.5, 0.5, -0.5, 1.0,
//                     1.0, 0.5, 0.5, -0.5, 1.0, 1.0, -0.5, 0.5, -0.5, 0.0, 1.0, -0.5, -0.5, -0.5,
//                     0.0, 0.0, -0.5, -0.5, 0.5, 0.0, 0.0, 0.5, -0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 0.5,
//                     1.0, 1.0, 0.5, 0.5, 0.5, 1.0, 1.0, -0.5, 0.5, 0.5, 0.0, 1.0, -0.5, -0.5, 0.5,
//                     0.0, 0.0, -0.5, 0.5, 0.5, 1.0, 0.0, -0.5, 0.5, -0.5, 1.0, 1.0, -0.5, -0.5,
//                     -0.5, 0.0, 1.0, -0.5, -0.5, -0.5, 0.0, 1.0, -0.5, -0.5, 0.5, 0.0, 0.0, -0.5,
//                     0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 1.0, 0.0, 0.5, 0.5, -0.5, 1.0, 1.0, 0.5,
//                     -0.5, -0.5, 0.0, 1.0, 0.5, -0.5, -0.5, 0.0, 1.0, 0.5, -0.5, 0.5, 0.0, 0.0, 0.5,
//                     0.5, 0.5, 1.0, 0.0, -0.5, -0.5, -0.5, 0.0, 1.0, 0.5, -0.5, -0.5, 1.0, 1.0, 0.5,
//                     -0.5, 0.5, 1.0, 0.0, 0.5, -0.5, 0.5, 1.0, 0.0, -0.5, -0.5, 0.5, 0.0, 0.0, -0.5,
//                     -0.5, -0.5, 0.0, 1.0, -0.5, 0.5, -0.5, 0.0, 1.0, 0.5, 0.5, -0.5, 1.0, 1.0, 0.5,
//                     0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 1.0, 0.0, -0.5, 0.5, 0.5, 0.0, 0.0, -0.5,
//                     0.5, -0.5, 0.0, 1.0,
//                 ];
//                 let indices = [
//                     0, 1, 3, // first Triangle
//                     1, 2, 3, // second Triangle
//                 ];

//                 let (mut VBO, mut VAO, mut EBO) = (0, 0, 0);
//                 unsafe {
//                     gl::GenVertexArrays(1, &mut VAO);
//                     gl::GenBuffers(1, &mut VBO);
//                     gl::GenBuffers(1, &mut EBO);
//                     // bind the Vertex Array Object first, then bind and set vertex buffer(s), and then configure vertex attributes(s).
//                     gl::BindVertexArray(VAO);

//                     gl::BindBuffer(gl::ARRAY_BUFFER, VBO);
//                     gl::BufferData(
//                         gl::ARRAY_BUFFER,
//                         (vertices.len() * mem::size_of::<GLfloat>()) as GLsizeiptr,
//                         &vertices[0] as *const f32 as *const c_void,
//                         gl::STATIC_DRAW,
//                     );

//                     gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, EBO);
//                     gl::BufferData(
//                         gl::ELEMENT_ARRAY_BUFFER,
//                         (indices.len() * mem::size_of::<GLfloat>()) as GLsizeiptr,
//                         &indices[0] as *const i32 as *const c_void,
//                         gl::STATIC_DRAW,
//                     );

//                     gl::VertexAttribPointer(
//                         0,
//                         3,
//                         gl::FLOAT,
//                         gl::FALSE,
//                         5 * mem::size_of::<GLfloat>() as GLsizei,
//                         ptr::null(),
//                     );
//                     gl::EnableVertexAttribArray(0);
//                     gl::VertexAttribPointer(
//                         1,
//                         2,
//                         gl::FLOAT,
//                         gl::FALSE,
//                         5 * mem::size_of::<GLfloat>() as GLsizei,
//                         (3 * mem::size_of::<GLfloat>() as GLsizei) as *const c_void,
//                     );
//                     gl::EnableVertexAttribArray(1);

//                     // note that this is allowed, the call to gl::VertexAttribPointer registered VBO as the vertex attribute's bound vertex buffer object so afterwards we can safely unbind
//                     gl::BindBuffer(gl::ARRAY_BUFFER, 0);
//                     gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, 0);

//                     // You can unbind the VAO afterwards so other VAO calls won't accidentally modify this VAO, but this rarely happens. Modifying other
//                     // VAOs requires a call to glBindVertexArray anyways so we generally don't unbind VAOs (nor VBOs) when it's not directly necessary.
//                     gl::BindVertexArray(0);
//                 }
//                 VAO
//             });

//             unsafe {
//                 // uncomment this call to draw in wireframe polygons.
//                 // gl::PolygonMode(gl::FRONT_AND_BACK, gl::LINE);
//                 gl::BindVertexArray(vao);
//                 //    gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.ebo);
//                 gl::DrawArrays(gl::TRIANGLES, 0, 36);
//             }
//         }
//     }
// }
