use std::f32::consts::PI;

use cgmath::{Vector2, Vector3};

use crate::render::VertexPositionTexture;

use super::{Geometry, IndexedMesh};

pub struct Cylinder;

#[derive(Debug, Clone, Copy)]
struct Vertex {
    position: Vector3<f32>,
    normal: Vector3<f32>,
    tex_coords: Vector2<f32>,
}

fn generate_cylinder(
    slices: u32,
    stacks: u32,
    height: f32,
    radius: f32,
) -> (Vec<Vertex>, Vec<usize>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Generate the vertices for the side surface
    for stack in 0..=stacks {
        let stack_fraction = stack as f32 / stacks as f32;
        let y = stack_fraction * height - (height / 2.0);

        for slice in 0..=slices {
            let slice_fraction = slice as f32 / slices as f32;
            let angle = slice_fraction * 2.0 * PI;

            let x = angle.cos() * radius;
            let z = angle.sin() * radius;

            let position = Vector3::new(x, y, z);
            let normal = Vector3::new(angle.cos(), 0.0, angle.sin());
            let tex_coords = Vector2::new(slice_fraction, stack_fraction);

            vertices.push(Vertex {
                position,
                normal,
                tex_coords,
            });

            if stack < stacks && slice < slices {
                let first = (stack * (slices + 1) + slice) as usize;
                let second = first + slices as usize + 1;

                indices.push(first);
                indices.push(second);
                indices.push(first + 1);

                indices.push(second);
                indices.push(second + 1);
                indices.push(first + 1);
            }
        }
    }

    // Caps: each is a triangle fan of (center + a contiguous ring). The ring
    // vertices are pushed in their own loop so a fan index is just
    // `ring_start + slice` — the previous version interleaved top/bottom ring
    // vertices but indexed them as one-per-slice, which built broken caps.
    let cap_uv = |angle: f32| Vector2::new(angle.cos() * 0.5 + 0.5, angle.sin() * 0.5 + 0.5);

    // Top cap (faces +Y).
    let top_center_index = vertices.len();
    vertices.push(Vertex {
        position: Vector3::new(0.0, height / 2.0, 0.0),
        normal: Vector3::new(0.0, 1.0, 0.0),
        tex_coords: Vector2::new(0.5, 0.5),
    });
    let top_ring_start = vertices.len();
    for slice in 0..=slices {
        let angle = (slice as f32 / slices as f32) * 2.0 * PI;
        vertices.push(Vertex {
            position: Vector3::new(angle.cos() * radius, height / 2.0, angle.sin() * radius),
            normal: Vector3::new(0.0, 1.0, 0.0),
            tex_coords: cap_uv(angle),
        });
    }
    for slice in 0..slices as usize {
        indices.push(top_center_index);
        indices.push(top_ring_start + slice);
        indices.push(top_ring_start + slice + 1);
    }

    // Bottom cap (faces -Y; reversed winding so it's outward-facing).
    let bottom_center_index = vertices.len();
    vertices.push(Vertex {
        position: Vector3::new(0.0, -height / 2.0, 0.0),
        normal: Vector3::new(0.0, -1.0, 0.0),
        tex_coords: Vector2::new(0.5, 0.5),
    });
    let bottom_ring_start = vertices.len();
    for slice in 0..=slices {
        let angle = (slice as f32 / slices as f32) * 2.0 * PI;
        vertices.push(Vertex {
            position: Vector3::new(angle.cos() * radius, -height / 2.0, angle.sin() * radius),
            normal: Vector3::new(0.0, -1.0, 0.0),
            tex_coords: cap_uv(angle),
        });
    }
    for slice in 0..slices as usize {
        indices.push(bottom_center_index);
        indices.push(bottom_ring_start + slice + 1);
        indices.push(bottom_ring_start + slice);
    }

    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::generate_cylinder;

    #[test]
    fn cap_fans_are_planar_and_in_bounds() {
        let height = 2.0;
        let half = height / 2.0;
        let (verts, indices) = generate_cylinder(8, 4, height, 0.5);

        assert_eq!(indices.len() % 3, 0, "indices must form triangles");

        // The two cap centers are the only vertices on the axis (x = z = 0).
        let is_cap_center =
            |&i: &usize| verts[i].position.x == 0.0 && verts[i].position.z == 0.0;

        for tri in indices.chunks(3) {
            for &i in tri {
                assert!(i < verts.len(), "index {i} out of bounds");
            }
            // A triangle that touches a cap center is a cap fan triangle; all
            // three of its vertices must lie on that cap's plane (the bug pulled
            // in interleaved vertices from the opposite cap).
            if tri.iter().any(is_cap_center) {
                let y0 = verts[tri[0]].position.y;
                assert!(
                    y0.abs() == half && tri.iter().all(|&i| verts[i].position.y == y0),
                    "cap fan triangle spans planes: {:?}",
                    tri.iter().map(|&i| verts[i].position.y).collect::<Vec<_>>()
                );
            }
        }
    }
}

impl Cylinder {
    pub fn create() -> Box<dyn Geometry> {
        let slices = 20;
        let stacks = 20;
        let height = 1.0;
        let radius = 0.5;
        let (cylinder_vertices, cylinder_indices) =
            generate_cylinder(slices, stacks, height, radius);

        let vertices = cylinder_vertices
            .into_iter()
            .map(|v| VertexPositionTexture {
                position: v.position,
                uv: v.tex_coords,
                normal: v.normal,
            })
            .collect();
        let indices = cylinder_indices.into_iter().map(|i| i as u32).collect();

        Box::new(IndexedMesh::create(vertices, indices))
    }
}
