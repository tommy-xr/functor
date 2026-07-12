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
use serde::Serialize;

use crate::animation::{Animation, AnimationChannel, AnimationProperty, AnimationValue, Keyframe};
use crate::model::{build_skeleton_from_skin, document_hierarchy, HierarchyNode, Skeleton};

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

/// An empty `Aabb` carries infinities (see [`Aabb::empty`]), and malformed/NaN
/// vertex data can leave non-finite coordinates; serde_json renders non-finite
/// floats as JSON `null`, which would corrupt the `[x, y, z]` arrays. So when the
/// box is empty or any coordinate is non-finite, serialize the whole `Aabb` as
/// `null`; otherwise `{ "min": [x, y, z], "max": [x, y, z] }` — matching the
/// codebase's plain `[f32; 3]` convention for geometry (see `light.rs`).
impl Serialize for Aabb {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let coords = [
            self.min.x, self.min.y, self.min.z, self.max.x, self.max.y, self.max.z,
        ];
        if self.is_empty() || coords.iter().any(|c| !c.is_finite()) {
            return serializer.serialize_none();
        }
        let mut s = serializer.serialize_struct("Aabb", 2)?;
        s.serialize_field("min", &[self.min.x, self.min.y, self.min.z])?;
        s.serialize_field("max", &[self.max.x, self.max.y, self.max.z])?;
        s.end()
    }
}

/// A single skinned vertex, kept CPU-side for AABB computation.
struct SkinnedVertex {
    position: Vector3<f32>,
    joint_indices: Vector4<f32>,
    weights: Vector4<f32>,
}

/// Per-primitive inspection report.
#[derive(Clone, Debug, Serialize)]
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
#[derive(Clone, Debug, Serialize)]
pub struct AnimationReport {
    pub name: String,
    pub duration: f32,
}

/// Per-node inspection: the node's name, its local translation, and — for a mesh
/// node — the AABB of its own mesh positions. Kenney-style glbs bake placement
/// offsets into node translations; those render displaced and look like renderer
/// bugs, so surfacing them here is the point of this section.
#[derive(Clone, Debug, Serialize)]
pub struct NodeReport {
    pub name: String,
    /// The node's local translation (the T of its local TRS transform).
    /// Serialized as `[x, y, z]`, or `null` if any component is non-finite —
    /// mirroring [`Aabb`] so a malformed node transform never emits a JSON array
    /// containing `null` (serde renders a non-finite float as `null`).
    #[serde(serialize_with = "serialize_translation")]
    pub translation: [f32; 3],
    /// True when `translation` is not (approximately) the origin — the baked
    /// placement offset worth noticing.
    pub translation_nonzero: bool,
    /// True when this node carries a mesh.
    pub has_mesh: bool,
    /// AABB of this node's own mesh positions, in the mesh's local space (bind
    /// pose; the node's transform is NOT applied). Empty (serializes to `null`)
    /// for a node with no mesh (or a mesh whose primitives carry no positions).
    pub bbox: Aabb,
}

/// Whether a local translation is far enough from the origin to flag as a baked
/// placement offset (vs floating-point noise around zero).
fn is_nonzero_translation(t: [f32; 3]) -> bool {
    const EPS: f32 = 1e-6;
    t.iter().any(|c| c.abs() > EPS)
}

/// Serialize a `[x, y, z]` translation, emitting `null` when any component is
/// non-finite — the same guard [`Aabb`] applies, so a malformed node transform
/// never yields a JSON array containing `null`.
fn serialize_translation<S>(t: &[f32; 3], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if t.iter().any(|c| !c.is_finite()) {
        serializer.serialize_none()
    } else {
        t.serialize(serializer)
    }
}

/// The full model inspection report.
#[derive(Clone, Debug, Serialize)]
pub struct ModelReport {
    pub primitives: Vec<PrimitiveReport>,
    /// Per-node name + local translation (+ mesh bbox) in scene traversal order
    /// (depth-first, parents before children). Reveals baked node offsets.
    pub nodes: Vec<NodeReport>,
    pub mesh_count: usize,
    /// Number of joints in the skeleton (0 if no skeleton).
    pub joint_count: i32,
    pub has_skeleton: bool,
    pub animations: Vec<AnimationReport>,
    /// Model-space AABB of the static (bind-pose) mesh positions.
    pub static_aabb: Aabb,
    /// Skinned AABB at the sampled pose, if a pose was requested (a `time`
    /// and/or an `animation_name`) and the model is skinned with at least one
    /// animation.
    pub skinned_aabb: Option<SkinnedAabbReport>,
}

/// The skinned AABB plus the context used to produce it.
#[derive(Clone, Debug, Serialize)]
pub struct SkinnedAabbReport {
    pub animation_name: String,
    pub requested_time: f32,
    /// The time actually sampled (`requested_time % duration`).
    pub sampled_time: f32,
    pub aabb: Aabb,
}

/// Inspect a glb/glTF byte buffer. The skinned AABB is computed when the model
/// is skinned with at least one animation and the caller asks to sample a pose —
/// i.e. either `time` is `Some` or `animation_name` is `Some` (an omitted `time`
/// defaults to `0.0`). `animation_name` selects which animation to sample; when
/// `None`, the first animation is used. An `animation_name` that doesn't exist
/// is an error that lists the available animations.
pub fn inspect_model(
    bytes: Vec<u8>,
    time: Option<f32>,
    animation_name: Option<&str>,
) -> Result<ModelReport, String> {
    // Reject non-finite sample times up front: they would otherwise flow into
    // `requested_time`/`sampled_time` and serialize as JSON `null`, yielding a
    // "successful" but semantically broken report.
    if let Some(t) = time {
        if !t.is_finite() {
            return Err(format!("time must be a finite number, got {}", t));
        }
    }

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
    let mut nodes: Vec<NodeReport> = Vec::new();
    let mut skinned_vertices: Vec<SkinnedVertex> = Vec::new();
    let mut static_aabb = Aabb::empty();
    let mut mesh_count: usize = 0;
    let mut maybe_skeleton: Option<Skeleton> = None;

    // The full node hierarchy, so the skeleton can include ancestor nodes
    // above the skin root (see `build_skeleton_from_skin`).
    let hierarchy = document_hierarchy(&document);

    for scene in document.scenes() {
        for node in scene.nodes() {
            inspect_node(
                &node,
                &buffers_data,
                &hierarchy,
                &mut primitives,
                &mut nodes,
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

    // Pick the animation to sample. If a selector is given, prefer an exact name
    // match, then fall back to a numeric index — so unnamed or duplicate-named
    // animations (glTF allows both) stay addressable. With no selector, use the
    // first animation. An unresolvable selector errors with an index-annotated
    // list of what's available.
    let selected_animation = match animation_name {
        Some(selector) => {
            let by_name = animations.iter().find(|a| a.name == selector);
            let by_index = || selector.parse::<usize>().ok().and_then(|i| animations.get(i));
            match by_name.or_else(by_index) {
                Some(a) => Some(a),
                None => {
                    let available = if animations.is_empty() {
                        "<none>".to_string()
                    } else {
                        animations
                            .iter()
                            .enumerate()
                            .map(|(i, a)| format!("{}: {}", i, a.name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    return Err(format!(
                        "Animation '{}' not found (use a name or index). Available: {}",
                        selector, available
                    ));
                }
            }
        }
        None => animations.first(),
    };

    // Compute the skinned AABB only when the caller asked to sample a pose
    // (a time and/or an animation name), the mesh is skinned, and there is an
    // animation to sample. An omitted time defaults to 0.0.
    let want_pose = time.is_some() || animation_name.is_some();
    let skinned_aabb = match selected_animation {
        Some(animation) if want_pose && any_skinned && has_skeleton => {
            let t = time.unwrap_or(0.0);
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
        nodes,
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

#[allow(clippy::too_many_arguments)]
fn inspect_node(
    node: &gltf::Node,
    buffers: &[gltf::buffer::Data],
    hierarchy: &HashMap<usize, HierarchyNode>,
    primitives: &mut Vec<PrimitiveReport>,
    nodes: &mut Vec<NodeReport>,
    skinned_vertices: &mut Vec<SkinnedVertex>,
    static_aabb: &mut Aabb,
    mesh_count: &mut usize,
    maybe_skeleton: &mut Option<Skeleton>,
) {
    // This node's own mesh bbox (local space), accumulated alongside the global
    // `static_aabb` as we walk its primitives' positions.
    let mut node_bbox = Aabb::empty();
    let has_mesh = node.mesh().is_some();

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
                node_bbox.extend(position);

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

    let translation = node.transform().decomposed().0;
    nodes.push(NodeReport {
        name: node.name().unwrap_or("<no name>").to_owned(),
        translation,
        translation_nonzero: is_nonzero_translation(translation),
        has_mesh,
        bbox: node_bbox,
    });

    if let Some(skin) = node.skin() {
        let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));

        let inverse_bind_matrices = reader
            .read_inverse_bind_matrices()
            .map(|v| v.map(Matrix4::from).collect::<Vec<Matrix4<f32>>>())
            .unwrap_or_default();

        let joint_node_indices = skin.joints().map(|j| j.index()).collect::<Vec<_>>();

        *maybe_skeleton = Some(build_skeleton_from_skin(
            inverse_bind_matrices,
            &joint_node_indices,
            hierarchy,
        ));
    }

    for child in node.children() {
        inspect_node(
            &child,
            buffers,
            hierarchy,
            primitives,
            nodes,
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

    #[test]
    fn aabb_serializes_finite_box_as_min_max() {
        let mut aabb = Aabb::empty();
        aabb.extend(vec3(-1.0, -2.0, -3.0));
        aabb.extend(vec3(1.0, 2.0, 3.0));
        let v = serde_json::to_value(aabb).unwrap();
        assert_eq!(v["min"], serde_json::json!([-1.0, -2.0, -3.0]));
        assert_eq!(v["max"], serde_json::json!([1.0, 2.0, 3.0]));
    }

    #[test]
    fn aabb_serializes_empty_or_nonfinite_as_null() {
        // Empty (never extended) -> null, not a struct full of infinities.
        assert!(serde_json::to_value(Aabb::empty()).unwrap().is_null());

        // Any non-finite coordinate -> null, so the [x,y,z] arrays never contain
        // JSON `null` (serde_json's rendering of non-finite floats). Constructed
        // directly: `extend`'s min/max would otherwise swallow NaN and clamp
        // infinities, so this exercises the serialize guard in isolation.
        let nan_box = Aabb {
            min: vec3(f32::NAN, 0.0, 0.0),
            max: vec3(1.0, 1.0, 1.0),
        };
        assert!(serde_json::to_value(nan_box).unwrap().is_null());

        let inf_box = Aabb {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(f32::INFINITY, 1.0, 1.0),
        };
        assert!(serde_json::to_value(inf_box).unwrap().is_null());
    }

    #[test]
    fn inspect_model_rejects_non_finite_time() {
        // The time guard runs before any glTF parsing, so empty bytes are fine.
        let err = inspect_model(Vec::new(), Some(f32::NAN), None).unwrap_err();
        assert!(err.contains("finite"), "unexpected error: {}", err);
    }

    #[test]
    fn node_translation_serializes_nonfinite_as_null() {
        // A non-finite component makes the whole translation serialize as null,
        // never a JSON array containing `null` (matching the `Aabb` guard).
        let node = NodeReport {
            name: "n".to_string(),
            translation: [f32::NAN, 0.0, 0.0],
            translation_nonzero: false,
            has_mesh: false,
            bbox: Aabb::empty(),
        };
        assert!(serde_json::to_value(&node).unwrap()["translation"].is_null());

        // A finite translation serializes as the `[x, y, z]` array.
        let finite = NodeReport {
            translation: [1.0, 2.0, 3.0],
            ..node
        };
        assert_eq!(
            serde_json::to_value(&finite).unwrap()["translation"],
            serde_json::json!([1.0, 2.0, 3.0])
        );
    }

    #[test]
    fn nonzero_translation_flags_offsets_not_noise() {
        assert!(!is_nonzero_translation([0.0, 0.0, 0.0]));
        assert!(!is_nonzero_translation([1e-9, 0.0, -1e-9])); // below epsilon → noise
        assert!(is_nonzero_translation([0.0, 0.5, 0.0]));
        assert!(is_nonzero_translation([-3.0, 0.0, 0.0]));
    }

    #[test]
    fn inspect_reports_per_node_translations_and_bbox() {
        // Use the committed skinned glove model (a real glb with a skeleton, so
        // its joint nodes carry non-origin local translations).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/glove/vr_glove_model.glb");
        let bytes = std::fs::read(&path).expect("read committed glove glb");
        let report = inspect_model(bytes, None, None).expect("inspect glove");

        // Every scene node is reported.
        assert!(!report.nodes.is_empty(), "expected per-node reports");

        // At least one node carries the mesh, with a populated local bbox.
        let mesh_nodes: Vec<&NodeReport> = report.nodes.iter().filter(|n| n.has_mesh).collect();
        assert!(!mesh_nodes.is_empty(), "expected a mesh-bearing node");
        assert!(
            mesh_nodes.iter().all(|n| !n.bbox.is_empty()),
            "a mesh node's local bbox should be populated"
        );

        // The skeleton's joints sit at non-origin local translations — exactly
        // the baked-offset signal this report exists to surface.
        assert!(
            report.nodes.iter().any(|n| n.translation_nonzero),
            "expected at least one node with a non-zero translation"
        );

        // `translation_nonzero` is consistent with the reported translation, and
        // a node with no mesh has an empty (null-serializing) bbox.
        for n in &report.nodes {
            assert_eq!(n.translation_nonzero, is_nonzero_translation(n.translation));
            if !n.has_mesh {
                assert!(n.bbox.is_empty(), "no-mesh node should have an empty bbox");
            }
        }
    }
}
