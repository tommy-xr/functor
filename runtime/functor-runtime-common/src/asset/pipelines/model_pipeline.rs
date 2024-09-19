use std::io::Cursor;

use cgmath::num_traits::ToPrimitive;
use cgmath::{vec2, vec3, Matrix4};
use gltf::camera::Projection;
use gltf::{buffer::Source as BufferSource, image::Source as ImageSource};

use crate::model::{Model, ModelMesh, Skeleton, SkeletonBuilder};
use crate::{
    asset::{AssetCache, AssetPipeline},
    geometry::IndexedMesh,
    render::vertex::VertexPositionTexture,
    texture::{Texture2D, TextureData, TextureOptions},
};

pub struct ModelPipeline;

impl AssetPipeline<Model> for ModelPipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        _asset_cache: &AssetCache,
        _context: crate::asset::AssetPipelineContext,
    ) -> Model {
        let cursor = Cursor::new(bytes);
        let gltf = gltf::Gltf::from_slice(cursor.get_ref()).unwrap();
        let document = gltf.document;
        let blob = gltf.blob;

        // Manually process buffers
        let mut buffers_data = Vec::new();
        for buffer in document.buffers() {
            let data = match buffer.source() {
                BufferSource::Bin => blob.as_ref().expect("No binary blob in GLB file").clone(),
                BufferSource::Uri(uri) => {
                    panic!("External buffer: {}", uri);
                }
            };
            buffers_data.push(gltf::buffer::Data(data));
        }

        // Manually process images
        let mut images_data = Vec::new();
        for image in document.images() {
            let data = match image.source() {
                ImageSource::View { view, .. } => {
                    // Get data from buffer view
                    let buffer = &buffers_data[view.buffer().index()];
                    let start = view.offset();
                    let end = start + view.length();
                    println!("Random image loaded: {} {}", start, end);
                    let buf = buffer[start..end].to_vec();
                    let maybe_image = image::load_from_memory(&buf);

                    if let Ok(image) = maybe_image {
                        TextureData::from_image(image)
                    } else {
                        TextureData::checkerboard_pattern(4, 4, [0, 255, 0, 255])
                    }
                }
                ImageSource::Uri { uri, .. } => {
                    // Manually resolve the image data
                    println!("External image: {}", uri);
                    TextureData::checkerboard_pattern(4, 4, [0, 0, 255, 255])
                }
            };
            images_data.push(data);
        }

        let mut meshes = Vec::new();

        let mut maybe_skeleton: Option<Skeleton> = None;

        for scene in document.scenes() {
            println!("Scene {}", scene.index());
            for node in scene.nodes() {
                println!("- Node: {:?}", node.name());
                process_node(
                    &node,
                    &buffers_data,
                    &images_data,
                    &mut meshes,
                    &mut maybe_skeleton,
                );
            }
        }

        process_animations(&document, &buffers_data);

        let skeleton = maybe_skeleton.unwrap_or(Skeleton::empty());

        Model { meshes, skeleton }
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Model {
        Model {
            meshes: vec![],
            skeleton: Skeleton::empty(),
        }
    }
}

fn process_node(
    node: &gltf::Node,
    buffers: &[gltf::buffer::Data],
    images: &[TextureData],
    meshes: &mut Vec<ModelMesh>,
    maybe_skeleton: &mut Option<Skeleton>,
) {
    if let Some(mesh) = node.mesh() {
        let transform_array = node.transform().matrix();
        let transform = Matrix4::from(transform_array);

        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let _name = mesh.name().unwrap_or(&"<no name>").to_owned();

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

            let joints = reader
                .read_joints(0)
                .map(|v| v.into_u16().collect::<Vec<_>>())
                .unwrap_or_default();

            let weights = reader
                .read_weights(0)
                .map(|v| v.into_f32().collect::<Vec<_>>())
                .unwrap_or_default();

            let vertices: Vec<VertexPositionTexture> = positions
                .iter()
                .zip(tex_coords.into_iter())
                .map(|(pos, tex)| VertexPositionTexture {
                    position: vec3(pos[0] * scale, pos[1] * scale, pos[2] * scale),
                    uv: vec2(tex[0], tex[1]),
                })
                .collect();
            println!(
                "-- Mesh: {:?} vertices: {} indices: {} joints: {} weights: {}",
                mesh.name(),
                vertices.len(),
                indices.len(),
                joints.len(),
                weights.len(),
            );

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

                let texture_data = image.clone();
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

    // Process skinning data
    if let Some(skin) = node.skin() {
        println!("Skin: {:?}", skin.name());
        let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));
        let inverse_bind_matrices = reader
            .read_inverse_bind_matrices()
            .map(|v| v.collect::<Vec<_>>())
            .unwrap_or_default();

        let mut skeleton_builder = SkeletonBuilder::create();

        let maybe_root = skin.skeleton();

        if let Some(root) = maybe_root {
            println!("Root: {:?}", root.name());
            process_joints(&root, None, &mut skeleton_builder);
        }

        *maybe_skeleton = Some(skeleton_builder.build());

        for (i, joint) in skin.joints().enumerate() {
            println!(
                "Joint {} -> {}: {:?} -> {:?}",
                i,
                joint.index(),
                joint.name().unwrap_or("<no name>"),
                joint.transform(),
            );
            // Process joint transformation and hierarchy
        }
    }

    for child in node.children() {
        process_node(&child, buffers, images, meshes, maybe_skeleton);
    }
}

fn process_joints(
    node: &gltf::Node,
    parent_id: Option<i32>,
    skeleton_builder: &mut SkeletonBuilder,
) {
    println!("visiting node: {:?} : {:?}", node.name(), node.transform());

    let id = node.index().to_i32().unwrap();
    let name = node.name().unwrap_or("None");
    let transform = node.transform().matrix().into();
    skeleton_builder.add_joint(id, name.to_owned(), parent_id, transform);

    for node in node.children() {
        process_joints(&node, Some(id), skeleton_builder);
    }
}

fn process_animations(document: &gltf::Document, buffers: &[gltf::buffer::Data]) {
    // Load animations
    // From: https://whoisryosuke.com/blog/2022/importing-gltf-with-wgpu-and-rust
    for animation in document.animations() {
        // println!("!! Animation: {:?}", animation.name());
        for channel in animation.channels() {
            let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
            let keyframe_timestamps = if let Some(inputs) = reader.read_inputs() {
                match inputs {
                    gltf::accessor::Iter::Standard(times) => {
                        let times: Vec<f32> = times.collect();
                        // println!("Time: {}", times.len());
                        // dbg!(times);
                    }
                    gltf::accessor::Iter::Sparse(_) => {
                        println!("Sparse keyframes not supported");
                    }
                }
            };

            let mut keyframes_vec: Vec<Vec<f32>> = Vec::new();
            let keyframes = if let Some(outputs) = reader.read_outputs() {
                match outputs {
                    gltf::animation::util::ReadOutputs::Translations(translation) => {
                        translation.for_each(|tr| {
                            // println!("Translation:");
                            // dbg!(tr);
                            let vector: Vec<f32> = tr.into();
                            keyframes_vec.push(vector);
                        });
                    }
                    other => (), // gltf::animation::util::ReadOutputs::Rotations(_) => todo!(),
                                 // gltf::animation::util::ReadOutputs::Scales(_) => todo!(),
                                 // gltf::animation::util::ReadOutputs::MorphTargetWeights(_) => todo!(),
                }
            };

            // println!("Keyframes: {}", keyframes_vec.len());
        }
    }
}
