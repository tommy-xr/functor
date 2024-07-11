use std::io::{self, Write};
use std::{fs::File, io::Cursor};

use cgmath::{vec2, vec3, vec4, Matrix4, Vector2, Vector3};
use fable_library_rust::System::Text;
use gltf::image::Format;
use gltf::Scene;

use crate::texture::PixelFormat;
use crate::{
    asset::{AssetCache, AssetPipeline},
    geometry::{Geometry, IndexedMesh},
    render::vertex::VertexPositionTexture,
    texture::{Texture2D, TextureData, TextureFormat, TextureOptions, PNG},
    Scene3D,
};

pub struct ModelPipeline;

pub struct ModelMesh {
    // Material info
    pub base_color_texture: Texture2D,

    pub mesh: IndexedMesh<VertexPositionTexture>,

    pub transform: Matrix4<f32>,
}

pub struct Model {
    pub meshes: Vec<ModelMesh>,
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
                process_node(&node, &buffers, &images, &mut meshes);
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
    images: &[gltf::image::Data],
    meshes: &mut Vec<ModelMesh>,
) {
    if let Some(mesh) = node.mesh() {
        let transform_array = node.transform().matrix();
        let transform = Matrix4::from(transform_array);

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

            // Parse material
            // Parse material
            let material = primitive.material();

            let base_color_texture =
                if let Some(specular_glossiness_material) = material.pbr_specular_glossiness() {
                    println!(
                        "Diffuse factor: {:?}",
                        specular_glossiness_material.diffuse_factor()
                    );
                    specular_glossiness_material.diffuse_texture()
                } else {
                    let material = material.pbr_metallic_roughness();
                    material.base_color_texture()
                };

            let texture = if let Some(texture) = base_color_texture {
                let texture_info = texture.texture();
                let source = texture_info.source();
                let image = &images[source.index()];

                println!("Material base color texture index: {:?}", source.index());
                println!("Texture width: {:?}", image.width);
                println!("Texture height: {:?}", image.height);
                println!("Texture format: {:?}", image.format);
                println!("Texture data length: {:?}", image.pixels.len());

                // Access the bytes and format
                let texture_bytes = &image.pixels;

                let format = match image.format {
                    Format::R8G8B8 => PixelFormat::RGB,
                    Format::R8G8B8A8 => PixelFormat::RGBA,
                    _ => unimplemented!("Pixel format: {:?} not implemented", image.format),
                };

                let texture_data = TextureData {
                    bytes: texture_bytes.clone(),
                    width: image.width,
                    height: image.height,
                    format,
                };

                Texture2D::init_from_data(texture_data, TextureOptions::default())
            } else {
                let data = TextureData::checkerboard_pattern(4, 4, [255, 0, 255, 255]);
                Texture2D::init_from_data(data, TextureOptions::default())
            };

            let mesh = IndexedMesh::create(vertices, indices);
            let model_mesh = ModelMesh {
                mesh,
                base_color_texture: texture,
                transform,
            };

            meshes.push(model_mesh);
        }
    }
    for child in node.children() {
        process_node(&child, buffers, images, meshes);
    }
}
