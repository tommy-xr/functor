use cgmath::{vec2, vec3, InnerSpace};

use crate::render::VertexPositionTexture;

use super::{Geometry, IndexedMesh};

/// A subdivided grid in the XZ plane (Y-up), centered on the origin and spanning
/// the unit square (-0.5..0.5); size it with `Transform.scale`. Each vertex's Y
/// comes from `heights` (row-major, length `rows * cols`). UVs tile one full
/// texture cell per grid quad (textures load with REPEAT wrap).
pub struct Heightmap;

impl Heightmap {
    pub fn create(rows: usize, cols: usize, heights: &[f32]) -> Box<dyn Geometry> {
        // Need at least a 2x2 grid to form a quad.
        let rows = rows.max(2);
        let cols = cols.max(2);
        let height_at = |r: usize, c: usize| heights.get(r * cols + c).copied().unwrap_or(0.0);

        // World spacing between adjacent grid samples (the grid spans 1 unit).
        let cell_dx = 1.0 / (cols - 1) as f32;
        let cell_dz = 1.0 / (rows - 1) as f32;

        let mut vertices = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let x = c as f32 / (cols - 1) as f32 - 0.5;
                let z = r as f32 / (rows - 1) as f32 - 0.5;

                // Normal from central differences of the height field, clamped
                // to one-sided differences at the edges.
                let (left, right) = (c.saturating_sub(1), (c + 1).min(cols - 1));
                let (up, down) = (r.saturating_sub(1), (r + 1).min(rows - 1));
                let dhdx = (height_at(r, right) - height_at(r, left))
                    / ((right - left) as f32 * cell_dx);
                let dhdz = (height_at(down, c) - height_at(up, c))
                    / ((down - up) as f32 * cell_dz);
                let normal = vec3(-dhdx, 1.0, -dhdz).normalize();

                vertices.push(VertexPositionTexture::new(
                    vec3(x, height_at(r, c), z),
                    vec2(c as f32, r as f32),
                    normal,
                ));
            }
        }

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

        super::compute_tangents(&mut vertices, &indices);
        Box::new(IndexedMesh::create(vertices, indices))
    }
}
