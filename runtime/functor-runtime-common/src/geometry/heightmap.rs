use cgmath::{vec2, vec3, InnerSpace};
use glow::{Buffer, HasContext, VertexArray};

use crate::render::vertex::{Vertex, VertexAttributeType};
use crate::render::VertexPositionTexture;

use super::compute_tangents;

/// A subdivided grid in the XZ plane (Y-up), centered on the origin and spanning
/// the unit square (-0.5..0.5); size it with `Transform.scale`. Each vertex's Y
/// comes from `heights` (row-major, length `rows * cols`). UVs tile one full
/// texture cell per grid quad (textures load with REPEAT wrap).
///
/// Unlike the static primitives, a heightmap is usually **animated** — its
/// `heights` change every frame. So the scene keeps one persistent
/// `HeightmapMesh` per `(rows, cols)` and re-uploads its vertex buffer in place
/// (`buffer_sub_data`) only when the heights actually change; the index buffer is
/// fixed for a given grid size and never re-uploaded. This avoids allocating a
/// fresh VAO/VBO/EBO every frame (which, with the never-evicted cache, leaked a
/// full GPU mesh per frame).
///
/// Like the primitive meshes (`Cube`/`Plane`/…), a `HeightmapMesh` is persistent
/// for the scene's lifetime — one per grid size, never evicted — so its GL
/// objects have no `Drop` (the GL context isn't available there) and are freed
/// with the context at teardown.
pub struct HeightmapMesh {
    cols: usize,
    /// Reused CPU-side vertex scratch, recomputed in place when heights change.
    vertices: Vec<VertexPositionTexture>,
    /// Triangle indices — constant for the grid size, so built once and kept for
    /// re-tangenting on each vertex update (never re-uploaded after `create`).
    indices: Vec<u32>,
    vao: VertexArray,
    vbo: Buffer,
    ebo: Buffer,
    /// Hash of the heights currently uploaded to the VBO; re-upload is skipped
    /// when the incoming heights match (static terrain uploads exactly once).
    heights_hash: u64,
}

impl HeightmapMesh {
    /// Build the grid mesh for `(rows, cols)` and upload it, seeded with `heights`.
    pub fn create(gl: &glow::Context, rows: usize, cols: usize, heights: &[f32]) -> HeightmapMesh {
        // Need at least a 2x2 grid to form a quad.
        let rows = rows.max(2);
        let cols = cols.max(2);

        let indices = build_indices(rows, cols);
        let mut vertices = Vec::with_capacity(rows * cols);
        fill_vertices(&mut vertices, rows, cols, heights, &indices);

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

            // DYNAMIC_DRAW: the vertex buffer is re-uploaded in place whenever the
            // heights change (see `update`).
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

            HeightmapMesh {
                cols,
                vertices,
                indices,
                vao,
                vbo,
                ebo,
                heights_hash: hash_heights(heights),
            }
        }
    }

    /// Re-upload the vertex buffer if `heights` changed since the last upload.
    /// `heights.len()` must equal the mesh's `rows * cols` — guaranteed by the
    /// caller, which keys the mesh by `(rows, cols)`.
    pub fn update(&mut self, gl: &glow::Context, heights: &[f32]) {
        let hash = hash_heights(heights);
        if hash == self.heights_hash {
            return;
        }
        let rows = self.vertices.len() / self.cols;
        fill_vertices(&mut self.vertices, rows, self.cols, heights, &self.indices);
        unsafe {
            let vertices_u8 = vertices_bytes(&self.vertices);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertices_u8);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            crate::gpu_counters::gpu_counters().uploaded(vertices_u8.len());
        }
        self.heights_hash = hash;
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

/// Triangle indices for a `rows × cols` grid — a function of the grid size only,
/// so it's built once per mesh and never re-uploaded.
fn build_indices(rows: usize, cols: usize) -> Vec<u32> {
    let mut indices = Vec::with_capacity((rows - 1) * (cols - 1) * 6);
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let i = (r * cols + c) as u32;
            let right = i + 1;
            let down = i + cols as u32;
            let down_right = down + 1;
            // Two triangles per cell. (Backface culling is off, so winding
            // only matters if it's enabled later.)
            indices.extend_from_slice(&[i, down, down_right, i, down_right, right]);
        }
    }
    indices
}

/// (Re)compute the grid's vertices from `heights` into `vertices`, reusing its
/// allocation. Positions, normals (central differences of the height field) and
/// tangents are all derived from `heights`, so this runs whenever the heights
/// change; `indices` (constant for the grid size) drive the tangent pass.
/// Produces byte-identical output to the previous per-frame rebuild.
fn fill_vertices(
    vertices: &mut Vec<VertexPositionTexture>,
    rows: usize,
    cols: usize,
    heights: &[f32],
    indices: &[u32],
) {
    let height_at = |r: usize, c: usize| heights.get(r * cols + c).copied().unwrap_or(0.0);

    // World spacing between adjacent grid samples (the grid spans 1 unit).
    let cell_dx = 1.0 / (cols - 1) as f32;
    let cell_dz = 1.0 / (rows - 1) as f32;

    vertices.clear();
    for r in 0..rows {
        for c in 0..cols {
            let x = c as f32 / (cols - 1) as f32 - 0.5;
            let z = r as f32 / (rows - 1) as f32 - 0.5;

            // Normal from central differences of the height field, clamped
            // to one-sided differences at the edges.
            let (left, right) = (c.saturating_sub(1), (c + 1).min(cols - 1));
            let (up, down) = (r.saturating_sub(1), (r + 1).min(rows - 1));
            let dhdx =
                (height_at(r, right) - height_at(r, left)) / ((right - left) as f32 * cell_dx);
            let dhdz = (height_at(down, c) - height_at(up, c)) / ((down - up) as f32 * cell_dz);
            let normal = vec3(-dhdx, 1.0, -dhdz).normalize();

            vertices.push(VertexPositionTexture::new(
                vec3(x, height_at(r, c), z),
                vec2(c as f32, r as f32),
                normal,
            ));
        }
    }

    compute_tangents(vertices, indices);
}

/// Content hash of a heightmap's `heights`, to skip re-uploading unchanged terrain.
fn hash_heights(heights: &[f32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for h in heights {
        h.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn vertices_bytes(vertices: &[VertexPositionTexture]) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts(
            vertices.as_ptr() as *const u8,
            std::mem::size_of_val(vertices),
        )
    }
}
