use std::collections::{HashMap, HashSet};

use cgmath::{InnerSpace, Matrix3, Matrix4, Quaternion, SquareMatrix, Vector3, VectorSpace};

use crate::animation::{Animation, AnimationValue, Keyframe};

/// One joint's local transform, decomposed — the unit poses are made of.
#[derive(Clone, Copy, Debug)]
pub struct JointPose {
    pub translation: Vector3<f32>,
    pub rotation: Quaternion<f32>,
    pub scale: Vector3<f32>,
}

impl JointPose {
    pub(crate) fn from_transform(transform: Matrix4<f32>) -> Self {
        let translation = transform.w.truncate();
        let linear = Matrix3::from_cols(
            transform.x.truncate(),
            transform.y.truncate(),
            transform.z.truncate(),
        );
        let scale = Vector3::new(
            linear.x.magnitude(),
            linear.y.magnitude(),
            linear.z.magnitude(),
        );
        let rotation_matrix = Matrix3::from_cols(
            linear.x / scale.x,
            linear.y / scale.y,
            linear.z / scale.z,
        );

        Self {
            translation,
            rotation: Quaternion::from(rotation_matrix),
            scale,
        }
    }

    pub(crate) fn transform(self) -> Matrix4<f32> {
        Matrix4::from_translation(self.translation)
            * Matrix4::from(self.rotation)
            * Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z)
    }
}

/// A skeleton-shaped set of local joint transforms in TRS form — what clip
/// sampling produces and blending combines, before being applied back onto a
/// [`Skeleton`] for skinning. Indexed by joint slot (the skeleton's
/// parents-first layout). The blending/masking math over poses lives in
/// [`crate::anim`].
#[derive(Clone, Debug)]
pub struct Pose {
    pub(crate) joints: Vec<JointPose>,
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub transform: Matrix4<f32>,
    pub parent: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    num_joints: i32,

    // Use HashMap for joints because they could be sparse
    joint_info: HashMap<i32, Joint>,
    joint_absolute_transform: HashMap<i32, Matrix4<f32>>,

    array_index_to_joint_id: HashMap<usize, i32>,

    inverse_bind_matrices: Vec<Matrix4<f32>>,

    // The evaluator's dense layout, built once in `build`: every joint has a
    // slot, ordered parents-first so absolutes resolve in one forward pass.
    slots: Vec<Slot>,
    /// Joint id -> slot. Ids are glTF node indices, dense enough to index.
    slot_by_id: Vec<Option<usize>>,
    /// Skinning array index -> slot (`inverse_bind_matrices` order).
    skin_slots: Vec<Option<usize>>,
}

#[derive(Clone, Debug)]
struct Slot {
    id: i32,
    parent: Option<usize>,
    /// The bind local transform, decomposed once at load.
    bind: JointPose,
}

impl Skeleton {
    pub fn empty() -> Skeleton {
        Skeleton {
            num_joints: 0,
            joint_info: HashMap::new(),
            joint_absolute_transform: HashMap::new(),
            inverse_bind_matrices: Vec::new(),
            array_index_to_joint_id: HashMap::new(),
            slots: Vec::new(),
            slot_by_id: Vec::new(),
            skin_slots: Vec::new(),
        }
    }

    pub fn animate(skeleton: &Skeleton, animation: &Animation, time: f32) -> Skeleton {
        skeleton.apply_pose(&skeleton.sample_pose(animation, time))
    }

    /// The bind pose: every joint's local transform, decomposed to TRS.
    pub fn base_pose(&self) -> Pose {
        Pose {
            joints: self.slots.iter().map(|slot| slot.bind).collect(),
        }
    }

    /// Sample one clip at `time`: the bind pose with each animated channel's
    /// interpolated value overriding that joint's T, R, or S.
    pub fn sample_pose(&self, animation: &Animation, time: f32) -> Pose {
        let mut pose = self.base_pose();
        for channel in &animation.channels {
            let Some(slot) = self
                .slot_by_id
                .get(channel.target_node_index)
                .copied()
                .flatten()
            else {
                continue;
            };
            if let Some(value) = interpolate_keyframes(&channel.keyframes, time) {
                let jp = &mut pose.joints[slot];
                match value {
                    AnimationValue::Translation(translation) => {
                        jp.translation = translation;
                    }
                    AnimationValue::Rotation(rotation) => {
                        jp.rotation = rotation;
                    }
                    AnimationValue::Scale(scale) => {
                        jp.scale = scale;
                    }
                    _ => {}
                }
            }
        }
        pose
    }

    /// Rebuild a skeleton with each joint's local transform replaced by the
    /// pose's TRS, and absolute transforms recomputed. The render path uses
    /// [`Skeleton::skinning_transforms_for`], which skips the rebuild.
    pub fn apply_pose(&self, pose: &Pose) -> Skeleton {
        let mut new_skeleton = self.clone();
        // The posed locals become the new skeleton's bind, as `joint_info` does.
        for (slot, jp) in new_skeleton.slots.iter_mut().zip(&pose.joints) {
            if let Some(joint) = new_skeleton.joint_info.get_mut(&slot.id) {
                joint.transform = jp.transform();
                slot.bind = JointPose::from_transform(joint.transform);
            }
        }
        new_skeleton.joint_absolute_transform = new_skeleton.absolute_transform_map();
        new_skeleton
    }

    /// The skinning matrices (`absolute * inverse_bind` per skin joint) for
    /// `pose`, resolving absolutes in one parents-first pass.
    pub fn skinning_transforms_for(&self, pose: &Pose) -> Vec<Matrix4<f32>> {
        let absolutes = self.absolutes(|slot| pose.joints[slot].transform());
        self.skin_slots
            .iter()
            .zip(&self.inverse_bind_matrices)
            .map(|(slot, inverse_bind)| {
                absolutes[slot.expect("skin joint has a slot")] * inverse_bind
            })
            .collect()
    }

    /// Absolute transforms by slot, from each slot's local transform.
    fn absolutes(&self, local: impl Fn(usize) -> Matrix4<f32>) -> Vec<Matrix4<f32>> {
        let mut absolutes: Vec<Matrix4<f32>> = Vec::with_capacity(self.slots.len());
        for (index, slot) in self.slots.iter().enumerate() {
            let local = local(index);
            let absolute = match slot.parent {
                Some(parent) => absolutes[parent] * local,
                None => local,
            };
            absolutes.push(absolute);
        }
        absolutes
    }

    /// Absolutes of the joints' current local transforms, keyed by id.
    fn absolute_transform_map(&self) -> HashMap<i32, Matrix4<f32>> {
        let absolutes = self.absolutes(|slot| self.joint_info[&self.slots[slot].id].transform);
        self.slots
            .iter()
            .map(|slot| slot.id)
            .zip(absolutes)
            .collect()
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn slot_parent(&self, slot: usize) -> Option<usize> {
        self.slots[slot].parent
    }

    pub(crate) fn slot_bind_pose(&self, slot: usize) -> JointPose {
        self.slots[slot].bind
    }

    /// The slot of the joint named `name` (see [`Skeleton::joint_id_by_name`]).
    pub(crate) fn slot_by_name(&self, name: &str) -> Option<usize> {
        let id = self.joint_id_by_name(name)?;
        self.slot_by_id[id as usize]
    }

    pub fn get_joint_count(&self) -> i32 {
        self.num_joints
    }

    /// The id of the joint named `name`, if any (exact match). glTF allows
    /// duplicate node names; the smallest id wins so the choice is
    /// deterministic across runs (HashMap iteration order is not).
    pub fn joint_id_by_name(&self, name: &str) -> Option<i32> {
        self.joint_info
            .iter()
            .filter(|(_, joint)| joint.name == name)
            .map(|(&id, _)| id)
            .min()
    }

    /// Per slot, whether its self-or-ancestor's name is in `roots` — the
    /// subtree semantics `Anim.mask` uses (naming a hand masks its fingers).
    pub(crate) fn subtree_slots(&self, roots: &[String]) -> Vec<bool> {
        let mut kept: Vec<bool> = Vec::with_capacity(self.slots.len());
        for slot in &self.slots {
            let name = &self.joint_info[&slot.id].name;
            let inherited = slot.parent.is_some_and(|parent| kept[parent]);
            kept.push(inherited || roots.iter().any(|root| root == name));
        }
        kept
    }

    pub fn get_joint_name(&self, idx: i32) -> Option<&str> {
        self.joint_info.get(&idx).map(|m| m.name.as_str())
    }

    pub fn get_joint_relative_transform(&self, idx: i32) -> Matrix4<f32> {
        self.joint_info
            .get(&idx)
            .map(|m| m.transform)
            .unwrap_or(Matrix4::identity())
    }

    pub fn get_joint_absolute_transform(&self, idx: i32) -> Matrix4<f32> {
        self.joint_absolute_transform
            .get(&idx)
            .map(|m| *m)
            .unwrap_or(Matrix4::identity())
    }

    pub fn get_transforms(&self) -> Vec<Matrix4<f32>> {
        let mut vec = Vec::new();
        for i in 0..self.num_joints {
            vec.push(self.get_joint_absolute_transform(i))
        }
        vec
    }

    pub fn get_skinning_transforms(&self) -> Vec<Matrix4<f32>> {
        let mut vec = Vec::new();
        for i in 0..self.inverse_bind_matrices.len() {
            let joint_idx = self.array_index_to_joint_id.get(&i).unwrap();
            vec.push(
                self.get_joint_absolute_transform(*joint_idx)
                    * self.inverse_bind_matrices.get(i as usize).unwrap(),
            )
        }
        vec
    }
}

/// Flatten a joint palette into the column-major f32 stream
/// `set_uniform_matrix4fv` expects — the one shared upload shape for every
/// skinned program (the stamped materials and the instanced shared-pose
/// path).
pub fn flatten_joint_matrices(joints: &[Matrix4<f32>]) -> Vec<f32> {
    let mut joint_matrices = Vec::with_capacity(joints.len() * 16);
    for joint in joints {
        let matrix_array: &[f32; 16] = joint.as_ref();
        joint_matrices.extend_from_slice(matrix_array);
    }
    joint_matrices
}

/// One node of the full glTF document hierarchy (every node, not just skin
/// joints) — enough to resolve joint parents and the ancestor chain above a
/// skin's root joints.
#[derive(Clone, Debug)]
pub struct HierarchyNode {
    pub parent: Option<usize>,
    pub name: String,
    pub transform: Matrix4<f32>,
}

/// The full document hierarchy, keyed by node index. Parent edges come from
/// every node's `children()` (a glTF node has at most one parent).
pub fn document_hierarchy(document: &gltf::Document) -> HashMap<usize, HierarchyNode> {
    let mut hierarchy: HashMap<usize, HierarchyNode> = document
        .nodes()
        .map(|node| {
            (
                node.index(),
                HierarchyNode {
                    parent: None,
                    name: node.name().unwrap_or("None").to_owned(),
                    transform: Matrix4::from(node.transform().matrix()),
                },
            )
        })
        .collect();
    for node in document.nodes() {
        for child in node.children() {
            if let Some(entry) = hierarchy.get_mut(&child.index()) {
                entry.parent = Some(node.index());
            }
        }
    }
    hierarchy
}

/// Build a [`Skeleton`] from a skin's joints (glTF node indices, in
/// `skin.joints()` order — the order the inverse bind matrices index) plus
/// the document hierarchy.
///
/// Per glTF 2.0, a joint's global transform is the product of the node
/// transforms from the scene root down to the joint — including ancestor
/// nodes ABOVE the skin's root joints (e.g. Mixamo's Armature node carrying
/// a cm→m scale + orientation). Those ancestors are added as extra
/// NON-skinning joints: they shape their descendants' absolute transforms
/// but own no inverse-bind-matrix slot. Keeping them as joints — rather than
/// folding their product into the root joints' local transforms — keeps
/// animation correct: a channel targets a node index and replaces that
/// joint's LOCAL transform (Mixamo clips animate the root joint, Hips), and
/// the ancestor transform above it must survive that override.
pub fn build_skeleton_from_skin(
    inverse_bind_matrices: Vec<Matrix4<f32>>,
    joint_node_indices: &[usize],
    hierarchy: &HashMap<usize, HierarchyNode>,
) -> Skeleton {
    let mut builder = SkeletonBuilder::create(inverse_bind_matrices);

    for (array_index, &node_index) in joint_node_indices.iter().enumerate() {
        let node = &hierarchy[&node_index];
        builder.add_joint(
            array_index,
            node_index as i32,
            node.name.clone(),
            node.parent.map(|p| p as i32),
            node.transform,
        );
    }

    // Walk each joint's ancestor chain, adding every node not already
    // present. Non-root joints break immediately (their parent is a skin
    // joint); root joints pull in the chain up to the scene root.
    let mut present: HashSet<usize> = joint_node_indices.iter().copied().collect();
    for &node_index in joint_node_indices {
        let mut current = hierarchy[&node_index].parent;
        while let Some(index) = current {
            if !present.insert(index) {
                break; // Already added — its own chain is done too.
            }
            let node = &hierarchy[&index];
            builder.add_ancestor_joint(
                index as i32,
                node.name.clone(),
                node.parent.map(|p| p as i32),
                node.transform,
            );
            current = node.parent;
        }
    }

    builder.build()
}

pub struct SkeletonBuilder {
    skeleton: Skeleton,
}

impl SkeletonBuilder {
    pub fn create(inverse_bind_matrices: Vec<Matrix4<f32>>) -> SkeletonBuilder {
        SkeletonBuilder {
            skeleton: Skeleton {
                array_index_to_joint_id: HashMap::new(),
                num_joints: inverse_bind_matrices.len() as i32,
                inverse_bind_matrices,
                joint_info: HashMap::new(),
                joint_absolute_transform: HashMap::new(),
                slots: Vec::new(),
                slot_by_id: Vec::new(),
                skin_slots: Vec::new(),
            },
        }
    }

    pub fn add_joint(
        &mut self,
        array_index: usize,
        joint_index: i32,
        name: String,
        parent_index: Option<i32>,
        transform: Matrix4<f32>,
    ) {
        self.skeleton
            .array_index_to_joint_id
            .insert(array_index, joint_index);
        let joint = Joint {
            name,
            transform,
            parent: parent_index,
        };
        self.skeleton.joint_info.insert(joint_index, joint);
    }

    /// Add a non-skinning joint — an ancestor node above the skin root. It
    /// shapes its descendants' absolute transforms but has no inverse bind
    /// matrix and no slot in the skinning array.
    pub fn add_ancestor_joint(
        &mut self,
        joint_index: i32,
        name: String,
        parent_index: Option<i32>,
        transform: Matrix4<f32>,
    ) {
        self.skeleton.joint_info.insert(
            joint_index,
            Joint {
                name,
                transform,
                parent: parent_index,
            },
        );
    }

    pub fn build(mut self) -> Skeleton {
        let skeleton = &mut self.skeleton;
        let order = parents_first(&skeleton.joint_info);
        assert!(
            order.iter().all(|&id| id >= 0),
            "joint ids are glTF node indices (non-negative)"
        );
        let max_id = order.iter().copied().max();
        skeleton.slot_by_id = vec![None; max_id.map_or(0, |id| id as usize + 1)];
        for (slot, &id) in order.iter().enumerate() {
            skeleton.slot_by_id[id as usize] = Some(slot);
        }
        skeleton.slots = order
            .iter()
            .map(|id| {
                let joint = &skeleton.joint_info[id];
                Slot {
                    id: *id,
                    parent: joint
                        .parent
                        .map(|parent| skeleton.slot_by_id[parent as usize].unwrap()),
                    bind: JointPose::from_transform(joint.transform),
                }
            })
            .collect();
        skeleton.skin_slots = (0..skeleton.inverse_bind_matrices.len())
            .map(|index| {
                let id = *skeleton.array_index_to_joint_id.get(&index)?;
                skeleton.slot_by_id[id as usize]
            })
            .collect();
        skeleton.joint_absolute_transform = skeleton.absolute_transform_map();

        // Return the built skeleton
        Skeleton {
            num_joints: max_id.map(|n| n + 1).unwrap_or(0),
            ..self.skeleton
        }
    }
}

/// Joint ids ordered parents-first, unrelated chains by ascending id so the
/// layout is deterministic. Panics on a cycle or a missing parent.
fn parents_first(joint_info: &HashMap<i32, Joint>) -> Vec<i32> {
    let mut ids: Vec<i32> = joint_info.keys().copied().collect();
    ids.sort_unstable();
    let mut placed = HashSet::new();
    let mut order = Vec::with_capacity(ids.len());
    let mut chain = Vec::new();
    for id in ids {
        // Walk up to the first placed ancestor, then place the chain root-first.
        let mut current = Some(id);
        while let Some(joint_id) = current {
            if placed.contains(&joint_id) {
                break;
            }
            if chain.contains(&joint_id) {
                panic!("Cycle detected in joint hierarchy at joint {}", joint_id);
            }
            chain.push(joint_id);
            current = joint_info.get(&joint_id).expect("Joint not found").parent;
        }
        for joint_id in chain.drain(..).rev() {
            placed.insert(joint_id);
            order.push(joint_id);
        }
    }
    order
}

/// Keys must be in non-decreasing time order (glTF requires strictly
/// increasing input), which the binary search below relies on.
fn interpolate_keyframes(keyframes: &Vec<Keyframe>, time: f32) -> Option<AnimationValue> {
    if keyframes.is_empty() {
        return None;
    }
    // If time is before the first keyframe, return the first value
    if time <= keyframes[0].time {
        return Some(keyframes[0].value.clone());
    }
    // If time is after the last keyframe, return the last value
    if time >= keyframes[keyframes.len() - 1].time {
        return Some(keyframes[keyframes.len() - 1].value.clone());
    }
    // The surrounding pair: the first key at or after `time` and the one
    // before it (an exact key time interpolates t = 1 from its predecessor).
    let next = keyframes.partition_point(|kf| kf.time < time);
    // A NaN `time` has no surrounding pair.
    if next == 0 || next == keyframes.len() {
        return None;
    }
    let kf0 = &keyframes[next - 1];
    let kf1 = &keyframes[next];
    let t = (time - kf0.time) / (kf1.time - kf0.time);
    Some(interpolate_values(&kf0.value, &kf1.value, t))
}
fn interpolate_values(v0: &AnimationValue, v1: &AnimationValue, t: f32) -> AnimationValue {
    match (v0, v1) {
        (AnimationValue::Translation(tr0), AnimationValue::Translation(tr1)) => {
            let value = tr0.lerp(*tr1, t);
            AnimationValue::Translation(value)
        }
        (AnimationValue::Rotation(r0), AnimationValue::Rotation(r1)) => {
            let value = r0.slerp(*r1, t);
            AnimationValue::Rotation(value)
        }
        (AnimationValue::Scale(s0), AnimationValue::Scale(s1)) => {
            let value = s0.lerp(*s1, t);
            AnimationValue::Scale(value)
        }
        _ => {
            // Unsupported interpolation
            v0.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Matrix4, Vector3};

    #[test]
    fn test_absolute_transforms() {
        // Define the transforms
        let transform_joint_0 = Matrix4::from_translation(Vector3::new(1.0, 0.0, 0.0));
        let transform_joint_1 = Matrix4::from_translation(Vector3::new(0.0, 2.0, 0.0));
        let transform_joint_2 = Matrix4::from_translation(Vector3::new(0.0, 0.0, 3.0));

        let inv_bind_matrices = vec![
            Matrix4::identity(),
            Matrix4::identity(),
            Matrix4::identity(),
        ];

        // Create the SkinBuilder
        let mut skin_builder = SkeletonBuilder::create(inv_bind_matrices);

        skin_builder.add_joint(0, 0, "Joint0".to_string(), None, transform_joint_0);
        skin_builder.add_joint(1, 1, "Joint1".to_string(), Some(0), transform_joint_1);
        skin_builder.add_joint(2, 2, "Joint2".to_string(), Some(1), transform_joint_2);

        // Build the Skin
        let skin = skin_builder.build();

        // Expected absolute transforms: each joint's absolute transform is its
        // parent's absolute transform composed with its own local transform.
        let expected_abs_transform_joint_0 = transform_joint_0;
        let expected_abs_transform_joint_1 = expected_abs_transform_joint_0 * transform_joint_1;
        let expected_abs_transform_joint_2 = expected_abs_transform_joint_1 * transform_joint_2;

        // Retrieve computed absolute transforms
        let abs_transform_joint_0 = skin.get_joint_absolute_transform(0);
        let abs_transform_joint_1 = skin.get_joint_absolute_transform(1);
        let abs_transform_joint_2 = skin.get_joint_absolute_transform(2);

        // Helper function to compare matrices
        fn matrices_approx_equal(a: &Matrix4<f32>, b: &Matrix4<f32>) -> bool {
            a.eq(b)
        }

        // Assert that computed transforms match expected transforms
        assert!(
            matrices_approx_equal(&abs_transform_joint_0, &expected_abs_transform_joint_0),
            "Joint 0 absolute transform does not match expected value."
        );
        assert!(
            matrices_approx_equal(&abs_transform_joint_1, &expected_abs_transform_joint_1),
            "Joint 1 absolute transform does not match expected value."
        );
        assert!(
            matrices_approx_equal(&abs_transform_joint_2, &expected_abs_transform_joint_2),
            "Joint 2 absolute transform does not match expected value."
        );
    }

    /// A synthetic Mixamo-shaped hierarchy: node 0 is an "Armature" ABOVE the
    /// skin root, carrying a cm→m scale + orientation; nodes 1 ("Hips") and
    /// 2 ("Spine") are the skin's joints.
    fn mixamo_like_hierarchy(
        armature: Matrix4<f32>,
        hips: Matrix4<f32>,
        spine: Matrix4<f32>,
    ) -> HashMap<usize, HierarchyNode> {
        let mut hierarchy = HashMap::new();
        hierarchy.insert(
            0,
            HierarchyNode {
                parent: None,
                name: "Armature".to_string(),
                transform: armature,
            },
        );
        hierarchy.insert(
            1,
            HierarchyNode {
                parent: Some(0),
                name: "Hips".to_string(),
                transform: hips,
            },
        );
        hierarchy.insert(
            2,
            HierarchyNode {
                parent: Some(1),
                name: "Spine".to_string(),
                transform: spine,
            },
        );
        hierarchy
    }

    #[test]
    fn ancestors_above_skin_root_shape_absolute_transforms() {
        // The Armature's transform (a scale + rotation, like Mixamo's cm→m
        // conversion) must flow into every joint's absolute transform even
        // though it is not one of the skin's joints.
        let armature =
            Matrix4::from_scale(0.01) * Matrix4::from_angle_x(cgmath::Deg(-90.0_f32));
        let hips = Matrix4::from_translation(Vector3::new(0.0, 100.0, 0.0));
        let spine = Matrix4::from_translation(Vector3::new(0.0, 10.0, 0.0));
        let hierarchy = mixamo_like_hierarchy(armature, hips, spine);

        let skeleton = build_skeleton_from_skin(
            vec![Matrix4::identity(), Matrix4::identity()],
            &[1, 2],
            &hierarchy,
        );

        assert_eq!(skeleton.get_joint_absolute_transform(1), armature * hips);
        assert_eq!(
            skeleton.get_joint_absolute_transform(2),
            armature * hips * spine
        );
        // The ancestor is a joint too (so it survives pose application), but
        // it owns no skinning-array slot: the skinning transforms are exactly
        // the two bind matrices' worth.
        assert_eq!(skeleton.get_skinning_transforms().len(), 2);
    }

    #[test]
    fn animating_the_root_joint_keeps_the_ancestor_transform() {
        // A clip channel targets the ROOT joint (Mixamo animates Hips) and
        // replaces its LOCAL transform. The Armature's scale above it must
        // still apply — this is why the ancestor chain is kept as joints
        // instead of being folded into the root joint's local transform.
        let armature = Matrix4::from_scale(0.01);
        let hips = Matrix4::from_translation(Vector3::new(0.0, 100.0, 0.0));
        let spine = Matrix4::from_translation(Vector3::new(0.0, 10.0, 0.0));
        let hierarchy = mixamo_like_hierarchy(armature, hips, spine);

        let skeleton = build_skeleton_from_skin(
            vec![Matrix4::identity(), Matrix4::identity()],
            &[1, 2],
            &hierarchy,
        );

        // One channel: Hips (node 1) translated to (0, 50, 0), in cm space.
        let animation = Animation {
            name: "test".to_string(),
            duration: 1.0,
            channels: vec![crate::animation::AnimationChannel {
                target_node_index: 1,
                target_property: crate::animation::AnimationProperty::Translation,
                keyframes: vec![Keyframe {
                    time: 0.0,
                    value: AnimationValue::Translation(Vector3::new(0.0, 50.0, 0.0)),
                }],
            }],
        };

        let animated = Skeleton::animate(&skeleton, &animation, 0.0);

        // The animated hips translation lands under the Armature's 0.01
        // scale: 50 cm → 0.5 world units (not 50).
        let abs = animated.get_joint_absolute_transform(1);
        let origin = abs * cgmath::Vector4::new(0.0, 0.0, 0.0, 1.0);
        assert!(
            (origin.y - 0.5).abs() < 1e-5,
            "expected hips at y=0.5, got {:?}",
            origin
        );
    }
}
