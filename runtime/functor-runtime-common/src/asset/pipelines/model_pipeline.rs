use std::io::Cursor;

use cgmath::{vec2, vec3, Vector2, Vector3};

use crate::{
    asset::{AssetCache, AssetPipeline},
    geometry::{Geometry, IndexedMesh},
    render::vertex::VertexPositionTexture,
    texture::{Texture2D, TextureData, TextureFormat, TextureOptions, PNG},
};

pub struct ModelPipeline;

pub struct Model {
    pub meshes: Vec<IndexedMesh<VertexPositionTexture>>,
}

impl Geometry for Model {
    fn draw(&self, gl: &glow::Context) {
        for mesh in self.meshes.iter() {
            // println!("Drawing mesh!");
            mesh.draw(&gl)
        }
    }
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

        Model { meshes }
    }

    fn unloaded_asset(&self, context: crate::asset::AssetPipelineContext) -> Model {
        Model { meshes: vec![] }
    }
}

fn process_node(
    node: &gltf::Node,
    buffers: &[gltf::buffer::Data],
    meshes: &mut Vec<IndexedMesh<VertexPositionTexture>>,
) {
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

            let scale = 1.0;

            let vertices: Vec<VertexPositionTexture> = positions
                .iter()
                .zip(tex_coords.into_iter())
                .map(|(pos, tex)| VertexPositionTexture {
                    position: vec3(pos[0] * scale, pos[1] * scale, pos[2] * scale),
                    uv: vec2(tex[0], tex[1]),
                })
                .collect();
            println!(
                "-- Mesh: {:?} vertices: {} indices: {}",
                mesh.name(),
                vertices.len(),
                indices.len()
            );

            let index_mesh = IndexedMesh::create(vertices, indices);
            meshes.push(index_mesh);
        }
    }
    for child in node.children() {
        process_node(&child, buffers, meshes);
    }
}
