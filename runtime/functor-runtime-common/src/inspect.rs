//! Headless, CPU-only glTF/glb inspection.
//!
//! This mirrors the *pure* parsing in
//! [`crate::asset::pipelines::model_pipeline`] (positions / indices / joints /
//! weights and the skeleton), but deliberately avoids anything that needs a GL
//! context (no `Texture2D`, no GL mesh buffers). The result is a plain data
//! struct that can be formatted to text by callers (e.g. the CLI), so models
//! can be inspected from scripts, CI, or LLMs without a GPU window.

use std::collections::HashMap;
use std::io::Cursor;

use cgmath::{vec3, vec4, Matrix4, Quaternion, Vector3, Vector4};
use gltf::buffer::Source as BufferSource;

use crate::animation::{Animation, AnimationChannel, AnimationProperty, AnimationValue, Keyframe};
use crate::model::{Skeleton, SkeletonBuilder};

/// An axis-aligned bounding box in model space.
#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: Vector3<f32>,
    pub max: Vector3<f32>,
}

impl Aabb {
    fn empty() -> Aabb {
        Aabb {
            min: vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY),
            max: vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
        }
    }

    fn extend(&mut self, p: Vector3<f32>) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.min.z = self.min.z.min(p.z);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
        self.max.z = self.max.z.max(p.z);
    }

    /// True if no points were ever added.
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }
}

/// A single skinned vertex, kept CPU-side for AABB computation.
struct SkinnedVertex {
    position: Vector3<f32>,
    joint_indices: Vector4<f32>,
    weights: Vector4<f32>,
}

/// Per-primitive inspection report.
#[derive(Clone, Debug)]
pub struct PrimitiveReport {
    /// The owning mesh's name (glTF names live on the mesh, not the primitive).
    pub mesh_name: String,
    pub vertex_count: usize,
    pub index_count: usize,
    pub joint_count: usize,
    pub weight_count: usize,
    /// True when the primitive carries both JOINTS_0 and WEIGHTS_0.
    pub has_skinning: bool,
}

/// A single animation's summary.
#[derive(Clone, Debug)]
pub struct AnimationReport {
    pub name: String,
    pub duration: f32,
}

/// The full model inspection report.
#[derive(Clone, Debug)]
pub struct ModelReport {
    pub primitives: Vec<PrimitiveReport>,
    pub mesh_count: usize,
    /// Number of joints in the skeleton (0 if no skeleton).
    pub joint_count: i32,
    pub has_skeleton: bool,
    pub animations: Vec<AnimationReport>,
    /// Model-space AABB of the static (bind-pose) mesh positions.
    pub static_aabb: Aabb,
    /// Skinned AABB at the requested time, if `--time` was given and the model
    /// is skinned and has at least one animation.
    pub skinned_aabb: Option<SkinnedAabbReport>,
}

/// The skinned AABB plus the context used to produce it.
#[derive(Clone, Debug)]
pub struct SkinnedAabbReport {
    pub animation_name: String,
    pub requested_time: f32,
    /// The time actually sampled (`requested_time % duration`).
    pub sampled_time: f32,
    pub aabb: Aabb,
}

/// Inspect a glb/glTF byte buffer. When `time` is `Some` and the model is
/// skinned with at least one animation, the skinned AABB at that time is also
/// computed.
pub fn inspect_model(bytes: Vec<u8>, time: Option<f32>) -> Result<ModelReport, String> {
    let cursor = Cursor::new(bytes);
    let gltf = gltf::Gltf::from_slice(cursor.get_ref())
        .map_err(|e| format!("Failed to parse glTF/glb: {}", e))?;
    let document = gltf.document;
    let blob = gltf.blob;

    // Resolve buffers. Only embedded (.glb binary blob) buffers are supported;
    // external .bin URIs would require resolving relative to the file.
    let mut buffers_data = Vec::new();
    for buffer in document.buffers() {
        let data = match buffer.source() {
            BufferSource::Bin => blob
                .as_ref()
                .ok_or_else(|| "No binary blob in GLB file".to_string())?
                .clone(),
            BufferSource::Uri(uri) => {
                return Err(format!(
                    "External buffers are not supported by the inspector: {}",
                    uri
                ));
            }
        };
        buffers_data.push(gltf::buffer::Data(data));
    }

    let mut primitives: Vec<PrimitiveReport> = Vec::new();
    let mut skinned_vertices: Vec<SkinnedVertex> = Vec::new();
    let mut static_aabb = Aabb::empty();
    let mut mesh_count: usize = 0;
    let mut maybe_skeleton: Option<Skeleton> = None;

    for scene in document.scenes() {
        for node in scene.nodes() {
            inspect_node(
                &node,
                &buffers_data,
                &mut primitives,
                &mut skinned_vertices,
                &mut static_aabb,
                &mut mesh_count,
                &mut maybe_skeleton,
            );
        }
    }

    let animations = process_animations(&document, &buffers_data);

    let has_skeleton = maybe_skeleton.is_some();
    let skeleton = maybe_skeleton.unwrap_or_else(Skeleton::empty);
    let joint_count = skeleton.get_joint_count();

    let any_skinned = primitives.iter().any(|p| p.has_skinning);

    // Compute the skinned AABB only when asked for a time, the mesh is skinned,
    // and there is an animation to sample.
    let skinned_aabb = match time {
        Some(t) if any_skinned && has_skeleton && !animations.is_empty() => {
            let animation = &animations[0];
            let sampled_time = if animation.duration > 0.0 {
                t.rem_euclid(animation.duration)
            } else {
                0.0
            };
            let animated = Skeleton::animate(&skeleton, animation, sampled_time);
            let skinning_transforms = animated.get_skinning_transforms();

            let mut aabb = Aabb::empty();
            for v in &skinned_vertices {
                let p = skin_position(v, &skinning_transforms);
                aabb.extend(p);
            }

            Some(SkinnedAabbReport {
                animation_name: animation.name.clone(),
                requested_time: t,
                sampled_time,
                aabb,
            })
        }
        _ => None,
    };

    let animation_reports = animations
        .iter()
        .map(|a| AnimationReport {
            name: a.name.clone(),
            duration: a.duration,
        })
        .collect();

    Ok(ModelReport {
        primitives,
        mesh_count,
        joint_count,
        has_skeleton,
        animations: animation_reports,
        static_aabb,
        skinned_aabb,
    })
}

/// Transform a skinned vertex's position by the weighted sum of its joint
/// matrices. This mirrors the skinned vertex shader in
/// [`crate::material::skinned_material`]:
/// `skinMatrix = sum(weight_i * jointTransforms[jointIndex_i])`.
fn skin_position(v: &SkinnedVertex, skinning_transforms: &[Matrix4<f32>]) -> Vector3<f32> {
    let identity = Matrix4::from_scale(1.0);
    let joint = |i: f32| -> Matrix4<f32> {
        let idx = i as usize;
        skinning_transforms.get(idx).copied().unwrap_or(identity)
    };

    let skin_matrix = joint(v.joint_indices.x) * v.weights.x
        + joint(v.joint_indices.y) * v.weights.y
        + joint(v.joint_indices.z) * v.weights.z
        + joint(v.joint_indices.w) * v.weights.w;

    let p = skin_matrix * vec4(v.position.x, v.position.y, v.position.z, 1.0);
    vec3(p.x, p.y, p.z)
}

fn inspect_node(
    node: &gltf::Node,
    buffers: &[gltf::buffer::Data],
    primitives: &mut Vec<PrimitiveReport>,
    skinned_vertices: &mut Vec<SkinnedVertex>,
    static_aabb: &mut Aabb,
    mesh_count: &mut usize,
    maybe_skeleton: &mut Option<Skeleton>,
) {
    if let Some(mesh) = node.mesh() {
        *mesh_count += 1;
        let mesh_name = mesh.name().unwrap_or("<no name>").to_owned();

        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions = reader
                .read_positions()
                .map(|v| v.collect::<Vec<_>>())
                .unwrap_or_default();

            let indices = reader
                .read_indices()
                .map(|v| v.into_u32().collect::<Vec<_>>())
                .unwrap_or_default();

            let joints = reader
                .read_joints(0)
                .map(|v| v.into_u16().collect::<Vec<_>>())
                .unwrap_or_default();

            let weights = reader
                .read_weights(0)
                .map(|v| v.into_f32().collect::<Vec<_>>())
                .unwrap_or_default();

            let has_skinning = !joints.is_empty() && !weights.is_empty();

            // Static AABB from raw bind-pose positions, and keep skinned
            // vertices for the optional animated AABB. We replicate the
            // pipeline's defaults (weight all on joint 0 when absent) so the
            // skinned math matches the runtime.
            for (i, pos) in positions.iter().enumerate() {
                let position = vec3(pos[0], pos[1], pos[2]);
                static_aabb.extend(position);

                let joint = joints.get(i).copied().unwrap_or([0, 0, 0, 0]);
                let weight = weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]);
                skinned_vertices.push(SkinnedVertex {
                    position,
                    joint_indices: vec4(
                        joint[0] as f32,
                        joint[1] as f32,
                        joint[2] as f32,
                        joint[3] as f32,
                    ),
                    weights: vec4(weight[0], weight[1], weight[2], weight[3]),
                });
            }

            primitives.push(PrimitiveReport {
                mesh_name: mesh_name.clone(),
                vertex_count: positions.len(),
                index_count: indices.len(),
                joint_count: joints.len(),
                weight_count: weights.len(),
                has_skinning,
            });
        }
    }

    if let Some(skin) = node.skin() {
        let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));

        let inverse_bind_matrices = reader
            .read_inverse_bind_matrices()
            .map(|v| v.map(Matrix4::from).collect::<Vec<Matrix4<f32>>>())
            .unwrap_or_default();

        let joints = skin.joints().collect::<Vec<_>>();

        let mut joint_index_to_parent_index: HashMap<usize, usize> = HashMap::new();
        for joint in joints.iter() {
            for child in joint.children() {
                joint_index_to_parent_index.insert(child.index(), joint.index());
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
        inspect_node(
            &child,
            buffers,
            primitives,
            skinned_vertices,
            static_aabb,
            mesh_count,
            maybe_skeleton,
        );
    }
}

/// CPU parse of animation channels. Identical in structure to the model
/// pipeline's `process_animations`, kept here so the inspector does not depend
/// on the GL-bound pipeline.
fn process_animations(document: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Vec<Animation> {
    let mut animations = Vec::new();

    for animation in document.animations() {
        let animation_name = animation.name().unwrap_or("Unnamed Animation").to_owned();
        let mut channels = Vec::new();
        let mut max_time = 0.0;

        for channel in animation.channels() {
            let target = channel.target();
            let node_index = target.node().index();
            let property = match target.property() {
                gltf::animation::Property::Translation => AnimationProperty::Translation,
                gltf::animation::Property::Rotation => AnimationProperty::Rotation,
                gltf::animation::Property::Scale => AnimationProperty::Scale,
                gltf::animation::Property::MorphTargetWeights => AnimationProperty::Weights,
            };

            let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));

            let input_times: Vec<f32> = match reader.read_inputs() {
                Some(inputs) => inputs.collect(),
                None => continue,
            };

            let output_values = match reader.read_outputs() {
                Some(outputs) => outputs,
                None => continue,
            };

            let mut keyframes = Vec::new();
            max_time = input_times
                .iter()
                .cloned()
                .fold(max_time, |a: f32, b: f32| a.max(b));

            match output_values {
                gltf::animation::util::ReadOutputs::Translations(translations) => {
                    for (i, translation) in translations.enumerate() {
                        keyframes.push(Keyframe {
                            time: input_times[i],
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
                        keyframes.push(Keyframe {
                            time: input_times[i],
                            value: AnimationValue::Rotation(Quaternion {
                                v: vec3(rotation[0], rotation[1], rotation[2]),
                                s: rotation[3],
                            }),
                        });
                    }
                }
                gltf::animation::util::ReadOutputs::Scales(scales) => {
                    for (i, scale) in scales.enumerate() {
                        keyframes.push(Keyframe {
                            time: input_times[i],
                            value: AnimationValue::Scale(vec3(scale[0], scale[1], scale[2])),
                        });
                    }
                }
                gltf::animation::util::ReadOutputs::MorphTargetWeights(_weights) => {
                    // Morph targets are not modeled by the runtime; skip.
                }
            }

            channels.push(AnimationChannel {
                target_node_index: node_index,
                target_property: property,
                keyframes,
            });
        }

        animations.push(Animation {
            name: animation_name,
            channels,
            duration: max_time,
        });
    }

    animations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_extend_tracks_min_max() {
        let mut aabb = Aabb::empty();
        assert!(aabb.is_empty());
        aabb.extend(vec3(1.0, -2.0, 3.0));
        aabb.extend(vec3(-1.0, 5.0, 0.0));
        assert!(!aabb.is_empty());
        assert_eq!(aabb.min, vec3(-1.0, -2.0, 0.0));
        assert_eq!(aabb.max, vec3(1.0, 5.0, 3.0));
    }

    #[test]
    fn skin_position_identity_passthrough() {
        // A vertex fully weighted to joint 0 with an identity transform should
        // come back unchanged.
        let v = SkinnedVertex {
            position: vec3(2.0, 3.0, 4.0),
            joint_indices: vec4(0.0, 0.0, 0.0, 0.0),
            weights: vec4(1.0, 0.0, 0.0, 0.0),
        };
        let transforms = vec![Matrix4::from_scale(1.0)];
        let p = skin_position(&v, &transforms);
        assert_eq!(p, vec3(2.0, 3.0, 4.0));
    }

    #[test]
    fn skin_position_applies_weighted_translation() {
        // Two joints, each translating, with equal weight: position should be
        // the average of the two translations applied.
        let v = SkinnedVertex {
            position: vec3(0.0, 0.0, 0.0),
            joint_indices: vec4(0.0, 1.0, 0.0, 0.0),
            weights: vec4(0.5, 0.5, 0.0, 0.0),
        };
        let transforms = vec![
            Matrix4::from_translation(vec3(10.0, 0.0, 0.0)),
            Matrix4::from_translation(vec3(0.0, 20.0, 0.0)),
        ];
        let p = skin_position(&v, &transforms);
        assert_eq!(p, vec3(5.0, 10.0, 0.0));
    }
}
