use std::collections::HashMap;
use std::io::Cursor;

use cgmath::{vec2, vec3, vec4, Matrix4, Quaternion};
use gltf::{buffer::Source as BufferSource, image::Source as ImageSource};

use crate::animation::{Animation, AnimationChannel, AnimationProperty, AnimationValue, Keyframe};
use crate::model::{Model, ModelMesh, Skeleton, SkeletonBuilder};
use crate::render::VertexPositionTextureSkinned;
use crate::{
    asset::{AssetCache, AssetPipeline},
    geometry::IndexedMesh,
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

        let animations = process_animations(&document, &buffers_data);

        let skeleton = maybe_skeleton.unwrap_or(Skeleton::empty());

        Model {
            meshes,
            skeleton,
            animations,
        }
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Model {
        Model {
            meshes: vec![],
            skeleton: Skeleton::empty(),
            animations: vec![],
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

            let normals = reader
                .read_normals()
                .map(|v| v.collect::<Vec<_>>())
                .unwrap_or_default();

            // glTF `TANGENT` is a vec4 (xyz + bitangent-handedness w). Many
            // assets omit it (no normal map authored); we compute a fallback
            // below so the tangent frame is always present.
            let tangents = reader
                .read_tangents()
                .map(|v| v.collect::<Vec<_>>())
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

            // Every vertex carries skinning attributes. Meshes without
            // JOINTS_0/WEIGHTS_0 get all their weight on joint 0, which the
            // renderer feeds an identity transform — truncating to the
            // shortest attribute list instead would leave the vertex buffer
            // empty while the index buffer still references it (GL reads out
            // of bounds: a segfault natively, GL_INVALID_OPERATION on WebGL).
            let mut vertices: Vec<VertexPositionTextureSkinned> = (0..positions.len())
                .map(|i| {
                    let uv = tex_coords.get(i).copied().unwrap_or([0.0, 0.0]);
                    let joint = joints.get(i).copied().unwrap_or([0, 0, 0, 0]);
                    let weight = weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]);
                    // glTF authors normals Y-up (our convention), so no axis
                    // conversion. Meshes without NORMAL fall back to +Y — the
                    // sample assets all provide normals.
                    let normal = normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
                    // Use the glTF tangent when present; otherwise a zeroed
                    // placeholder that `compute_tangents` overwrites below.
                    let tangent = tangents.get(i).copied().unwrap_or([0.0, 0.0, 0.0, 0.0]);
                    VertexPositionTextureSkinned {
                        position: vec3(
                            positions[i][0] * scale,
                            positions[i][1] * scale,
                            positions[i][2] * scale,
                        ),
                        uv: vec2(uv[0], uv[1]),
                        normal: vec3(normal[0], normal[1], normal[2]),
                        tangent: vec4(tangent[0], tangent[1], tangent[2], tangent[3]),
                        joint_indices: vec4(
                            joint[0] as f32,
                            joint[1] as f32,
                            joint[2] as f32,
                            joint[3] as f32,
                        ),
                        weights: vec4(weight[0], weight[1], weight[2], weight[3]),
                    }
                })
                .collect();

            // Fallback: derive tangents from positions/uvs/normals when the
            // mesh didn't carry a `TANGENT` attribute (the common case for the
            // sample assets), so normal mapping + the tangent debug view work.
            if tangents.is_empty() {
                crate::geometry::compute_tangents(&mut vertices, &indices);
            }
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

            let (base_color_texture, base_color_factor) =
                if let Some(specular_glossiness_material) = material.pbr_specular_glossiness() {
                    (
                        specular_glossiness_material.diffuse_texture(),
                        specular_glossiness_material.diffuse_factor(),
                    )
                } else {
                    let material = material.pbr_metallic_roughness();
                    (material.base_color_texture(), material.base_color_factor())
                };

            let texture = if let Some(texture) = base_color_texture {
                let texture_info = texture.texture();
                let source = texture_info.source();
                let image = &images[source.index()];

                let texture_data = image.clone();
                Texture2D::init_from_data(texture_data, TextureOptions::default())
            } else {
                // No texture: glTF defines the color as the base color factor
                // alone, so sample a 1x1 solid of it (untextured materials —
                // e.g. Xbot.glb, HVGirl.glb — are flat factor colors).
                let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
                let data = TextureData::solid_color([
                    to_u8(base_color_factor[0]),
                    to_u8(base_color_factor[1]),
                    to_u8(base_color_factor[2]),
                    to_u8(base_color_factor[3]),
                ]);
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
        let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));

        let inverse_bind_matrices = reader
            .read_inverse_bind_matrices()
            .map(|v| {
                v.map(|mat_array| Matrix4::from(mat_array))
                    .collect::<Vec<Matrix4<f32>>>()
            })
            .unwrap_or_default();

        let joints = skin.joints().collect::<Vec<_>>();

        // Figure out the parent index from joint index
        let mut joint_index_to_parent_index: HashMap<usize, usize> = HashMap::new();
        for joint in joints.iter() {
            for children in joint.children() {
                joint_index_to_parent_index.insert(children.index(), joint.index());
            }
        }

        let mut skeleton_builder = SkeletonBuilder::create(inverse_bind_matrices);

        for (i, joint) in joints.iter().enumerate() {
            let name = joint.name().unwrap_or("None");
            let transform = joint.transform().matrix().into();

            let parent_index_i32 = joint_index_to_parent_index
                .get(&joint.index())
                .map(|u| *u as i32);

            skeleton_builder.add_joint(
                i,
                joint.index() as i32,
                name.to_owned(),
                parent_index_i32,
                transform,
            );
        }

        *maybe_skeleton = Some(skeleton_builder.build());
    }

    for child in node.children() {
        process_node(&child, buffers, images, meshes, maybe_skeleton);
    }
}

fn process_animations(document: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Vec<Animation> {
    let mut animations = Vec::new();
    // Load animations
    // From: https://whoisryosuke.com/blog/2022/importing-gltf-with-wgpu-and-rust
    for animation in document.animations() {
        let animation_name = animation.name().unwrap_or("Unnamed Animation").to_owned();
        let mut channels = Vec::new();
        let mut max_time = 0.0;

        for channel in animation.channels() {
            // TODO: Proper interpolation
            //let sampler = channel.sampler();
            let target = channel.target();
            let node_index = target.node().index();
            let property = match target.property() {
                gltf::animation::Property::Translation => AnimationProperty::Translation,
                gltf::animation::Property::Rotation => AnimationProperty::Rotation,
                gltf::animation::Property::Scale => AnimationProperty::Scale,
                gltf::animation::Property::MorphTargetWeights => AnimationProperty::Weights,
            };

            let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));

            let input_times: Vec<f32> = reader
                .read_inputs()
                .expect("Failed to read animation input")
                .collect();

            let output_values = reader
                .read_outputs()
                .expect("Failed to read animation output");

            let mut keyframes = Vec::new();
            max_time = input_times
                .iter()
                .cloned()
                .fold(max_time, |a: f32, b: f32| a.max(b));

            match output_values {
                gltf::animation::util::ReadOutputs::Translations(translations) => {
                    for (i, translation) in translations.enumerate() {
                        let time = input_times[i];
                        keyframes.push(Keyframe {
                            time,
                            value: AnimationValue::Translation(vec3(
                                translation[0],
                                translation[1],
                                translation[2],
                            )),
                        });
                    }
                }
                gltf::animation::util::ReadOutputs::Rotations(rotations) => {
                    for (i, rotation) in rotations.into_f32().enumerate() {
                        let time = input_times[i];
                        keyframes.push(Keyframe {
                            time,
                            // TODO: Does w come first or last?
                            value: AnimationValue::Rotation(Quaternion {
                                v: vec3(rotation[0], rotation[1], rotation[2]),
                                s: rotation[3],
                            }),
                        });
                    }
                }
                gltf::animation::util::ReadOutputs::Scales(scales) => {
                    for (i, scale) in scales.enumerate() {
                        let time = input_times[i];
                        keyframes.push(Keyframe {
                            time,
                            value: AnimationValue::Scale(vec3(scale[0], scale[1], scale[2])),
                        });
                    }
                }
                gltf::animation::util::ReadOutputs::MorphTargetWeights(_weights) => {
                    // TODO:
                    println!("WARN: ignoring morph target weights; not implemented");
                    // for (i, weight) in weights.enumerate() {
                    //     let time = input_times[i];
                    //     keyframes.push(Keyframe {
                    //         time,
                    //         value: AnimationValue::Weights(weight.to_vec()),
                    //     });
                    // }
                }
            }

            channels.push(AnimationChannel {
                target_node_index: node_index,
                target_property: property,
                keyframes,
                // TODO:
                //interpolation,
            });
        }

        animations.push(Animation {
            name: animation_name,
            channels,
            duration: max_time,
        });
    }
    // panic!("animations");
    animations
}
