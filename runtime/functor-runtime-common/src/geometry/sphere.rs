use std::f32::consts::PI;

use cgmath::{vec2, vec3, Vector2, Vector3};

use super::{
    mesh::{self, MeshData},
    Mesh,
};

pub struct Sphere;

#[derive(Debug, Clone, Copy)]
struct Vertex {
    position: Vector3<f32>,
    #[allow(dead_code)]
    normal: Vector3<f32>,
    tex_coords: Vector2<f32>,
}

fn generate_unit_sphere(slices: u32, stacks: u32) -> (Vec<Vertex>, Vec<usize>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for stack in 0..=stacks {
        let stack_fraction = stack as f32 / stacks as f32;
        let stack_angle = stack_fraction * PI;

        for slice in 0..=slices {
            let slice_fraction = slice as f32 / slices as f32;
            let slice_angle = slice_fraction * 2.0 * PI;

            let x = slice_angle.sin() * stack_angle.sin();
            let y = stack_angle.cos();
            let z = slice_angle.cos() * stack_angle.sin();

            let position = vec3(x, y, z);
            let normal = vec3(x, y, z);
            let tex_coords = vec2(slice_fraction, stack_fraction);

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

    (vertices, indices)
}

impl Sphere {
    pub fn create() -> Mesh {
        let slices = 20;
        let stacks = 20;
        let (sphere_vertices, sphere_indices) = generate_unit_sphere(slices, stacks);

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
