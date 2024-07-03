use std::io::Cursor;

use cgmath::{vec2, vec3, Vector2, Vector3};

use crate::{
    asset::{AssetCache, AssetPipeline},
    texture::{Texture2D, TextureData, TextureFormat, TextureOptions, PNG},
};

pub struct ModelPipeline;

#[derive(Debug)]
pub struct Model {
    pub meshes: Vec<Mesh>,
}

#[derive(Debug)]
pub struct Mesh {
    // pub transform: na::Matrix4<f32>,
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Debug)]
pub struct Vertex {
    pub position: Vector3<f32>,
    pub tex_coords: Vector2<f32>,
}
impl AssetPipeline<Model> for ModelPipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        asset_cache: &AssetCache,
        context: crate::asset::AssetPipelineContext,
    ) -> Model {
        let cursor = Cursor::new(bytes);
        let (document, buffers, images) = gltf::import_slice(cursor.get_ref()).unwrap();

        let mut meshes = Vec::new();

        for scene in document.scenes() {
            print!("Scene {}", scene.index());
            println!();
            for node in scene.nodes() {
                println!("- Node: {:?}", node.name());
                process_node(&node, &buffers, &mut meshes);
            }
        }

        //println!("{:#?}", meshes);

        panic!();
    }

    fn unloaded_asset(&self, context: crate::asset::AssetPipelineContext) -> Model {
        Model { meshes: vec![] }
    }
}

fn process_node(node: &gltf::Node, buffers: &[gltf::buffer::Data], meshes: &mut Vec<Mesh>) {
    if let Some(mesh) = node.mesh() {
        let transform = node.transform().matrix();
        // TODO: Convert to cgmath?

        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let name = mesh.name().unwrap_or(&"<no name>").to_owned();

            let positions = reader
                .read_positions()
                .map(|v| v.collect::<Vec<_>>())
                .unwrap_or_default();

            let indices = reader
                .read_indices()
                .map(|v| v.into_u32().collect::<Vec<_>>())
                .unwrap_or_default();

            let tex_coords = reader
                .read_tex_coords(0)
                .map(|v| v.into_f32().collect::<Vec<_>>())
                .unwrap_or_default();

            let vertices: Vec<Vertex> = positions
                .iter()
                .zip(tex_coords.into_iter())
                .map(|(pos, tex)| Vertex {
                    position: vec3(pos[0], pos[1], pos[2]),
                    tex_coords: vec2(tex[0], tex[1]),
                })
                .collect();
            println!(
                "-- Mesh: {:?} vertices: {} indices: {}",
                mesh.name(),
                vertices.len(),
                indices.len()
            );

            meshes.push(Mesh {
                //transform,
                name,
                vertices,
                indices,
            });
        }
    }

    for child in node.children() {
        process_node(&child, buffers, meshes);
    }
}
