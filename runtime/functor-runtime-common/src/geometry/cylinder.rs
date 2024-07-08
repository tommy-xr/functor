use std::f32::consts::PI;

use cgmath::{Vector2, Vector3};

use super::Mesh;

pub struct Cylinder;

#[derive(Debug, Clone, Copy)]
struct Vertex {
    position: Vector3<f32>,
    #[allow(dead_code)]
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

    // Generate the vertices and indices for the top and bottom caps
    let top_center_index = vertices.len() as usize;
    vertices.push(Vertex {
        position: Vector3::new(0.0, height / 2.0, 0.0),
        normal: Vector3::new(0.0, 1.0, 0.0),
        tex_coords: Vector2::new(0.5, 0.5),
    });

    let bottom_center_index = vertices.len() as usize;
    vertices.push(Vertex {
        position: Vector3::new(0.0, -height / 2.0, 0.0),
        normal: Vector3::new(0.0, -1.0, 0.0),
        tex_coords: Vector2::new(0.5, 0.5),
    });

    for slice in 0..=slices {
        let slice_fraction = slice as f32 / slices as f32;
        let angle = slice_fraction * 2.0 * PI;

        let x = angle.cos() * radius;
        let z = angle.sin() * radius;

        let top_position = Vector3::new(x, height / 2.0, z);
        let bottom_position = Vector3::new(x, -height / 2.0, z);
        let normal = Vector3::new(0.0, 1.0, 0.0);
        let tex_coords = Vector2::new(slice_fraction, 0.0);

        vertices.push(Vertex {
            position: top_position,
            normal,
            tex_coords: Vector2::new(slice_fraction, 1.0),
        });

        vertices.push(Vertex {
            position: bottom_position,
            normal: -normal,
            tex_coords,
        });

        let top_index = (top_center_index + slice as usize + 1) as usize;
        let bottom_index = (bottom_center_index + slice as usize + 1) as usize;

        if slice < slices {
            indices.push(top_center_index);
            indices.push(top_index);
            indices.push(top_index + 1);

            indices.push(bottom_center_index);
            indices.push(bottom_index + 1);
            indices.push(bottom_index);
        }
    }

    (vertices, indices)
}

impl Cylinder {
    pub fn create() -> Mesh {
        let slices = 20;
        let stacks = 20;
        let height = 1.0;
        let radius = 0.5;
        let (sphere_vertices, sphere_indices) = generate_cylinder(slices, stacks, height, radius);

        let mut raw_vertices: Vec<f32> = Vec::new();

        for idx in sphere_indices {
            let vertex = sphere_vertices[idx];
            raw_vertices.push(vertex.position.x);
            raw_vertices.push(vertex.position.y);
            raw_vertices.push(vertex.position.z);
            raw_vertices.push(vertex.tex_coords.x);
            raw_vertices.push(vertex.tex_coords.y);
        }

        Mesh::create(raw_vertices)
    }
}
