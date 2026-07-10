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

/// A skeleton-shaped set of local joint transforms in TRS form — what clip
/// sampling produces and blending combines, before being applied back onto a
/// [`Skeleton`] for skinning. Keyed by joint id (sparse, like `joint_info`).
/// The blending/masking math over poses lives in [`crate::anim`].
#[derive(Clone, Debug)]
pub struct Pose {
    pub(crate) joints: HashMap<i32, JointPose>,
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
}

impl Skeleton {
    pub fn empty() -> Skeleton {
        Skeleton {
            num_joints: 0,
            joint_info: HashMap::new(),
            joint_absolute_transform: HashMap::new(),
            inverse_bind_matrices: Vec::new(),
            array_index_to_joint_id: HashMap::new(),
        }
    }

    pub fn animate(skeleton: &Skeleton, animation: &Animation, time: f32) -> Skeleton {
        skeleton.apply_pose(&skeleton.sample_pose(animation, time))
    }

    /// The bind pose: every joint's local transform, decomposed to TRS.
    pub fn base_pose(&self) -> Pose {
        let mut joints = HashMap::new();
        for (&joint_index, joint) in &self.joint_info {
            // Extract translation (last column)
            let translation = joint.transform.w.truncate();

            // Extract the upper-left 3x3 matrix
            let m = Matrix3::from_cols(
                joint.transform.x.truncate(),
                joint.transform.y.truncate(),
                joint.transform.z.truncate(),
            );

            // Extract scale factors
            let scale_x = m.x.magnitude();
            let scale_y = m.y.magnitude();
            let scale_z = m.z.magnitude();
            let scale = Vector3::new(scale_x, scale_y, scale_z);

            // Normalize the columns to get the rotation matrix, then a quaternion
            let rotation_matrix = Matrix3::from_cols(m.x / scale_x, m.y / scale_y, m.z / scale_z);
            let rotation = Quaternion::from(rotation_matrix);

            joints.insert(
                joint_index,
                JointPose {
                    translation,
                    rotation,
                    scale,
                },
            );
        }
        Pose { joints }
    }

    /// Sample one clip at `time`: the bind pose with each animated channel's
    /// interpolated value overriding that joint's T, R, or S.
    pub fn sample_pose(&self, animation: &Animation, time: f32) -> Pose {
        let mut pose = self.base_pose();
        for channel in &animation.channels {
            let target_node_index = channel.target_node_index as i32;
            if let Some(value) = interpolate_keyframes(&channel.keyframes, time) {
                if let Some(jp) = pose.joints.get_mut(&target_node_index) {
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
        }
        pose
    }

    /// Rebuild a skeleton with each joint's local transform replaced by the
    /// pose's TRS, and absolute transforms recomputed — the step between a
    /// (possibly blended) pose and `get_skinning_transforms`.
    pub fn apply_pose(&self, pose: &Pose) -> Skeleton {
        let mut new_skeleton = self.clone();
        for (&joint_index, joint) in new_skeleton.joint_info.iter_mut() {
            if let Some(jp) = pose.joints.get(&joint_index) {
                joint.transform = Matrix4::from_translation(jp.translation)
                    * Matrix4::from(jp.rotation)
                    * Matrix4::from_nonuniform_scale(jp.scale.x, jp.scale.y, jp.scale.z);
            }
        }
        new_skeleton.joint_absolute_transform =
            compute_absolute_transforms(&new_skeleton.joint_info);
        new_skeleton
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

    /// All joint ids whose self-or-ancestor's name is in `roots` — the
    /// subtree semantics `Anim.mask` uses (naming a hand masks its fingers).
    pub fn subtree_joint_ids(&self, roots: &[String]) -> HashSet<i32> {
        let mut ids = HashSet::new();
        for &joint_id in self.joint_info.keys() {
            let mut current = Some(joint_id);
            while let Some(id) = current {
                let Some(joint) = self.joint_info.get(&id) else {
                    break;
                };
                if roots.iter().any(|root| *root == joint.name) {
                    ids.insert(joint_id);
                    break;
                }
                current = joint.parent;
            }
        }
        ids
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

    pub fn build(mut self) -> Skeleton {
        // Compute absolute transforms
        let joint_absolute_transform = compute_absolute_transforms(&self.skeleton.joint_info);

        // Update the skeleton with the computed absolute transforms
        self.skeleton.joint_absolute_transform = joint_absolute_transform;

        let num_joints = self.skeleton.joint_info.keys().max();

        // Return the built skeleton
        Skeleton {
            num_joints: num_joints.map(|n| n + 1).unwrap_or(0),
            ..self.skeleton
        }
    }
}

fn compute_absolute_transforms(joint_info: &HashMap<i32, Joint>) -> HashMap<i32, Matrix4<f32>> {
    let mut joint_absolute_transform = HashMap::new();

    for &joint_index in joint_info.keys() {
        compute_joint_absolute_transform(
            joint_index,
            joint_info,
            &mut joint_absolute_transform,
            &mut HashSet::new(), // For cycle detection
        );
    }

    joint_absolute_transform
}

fn compute_joint_absolute_transform(
    joint_index: i32,
    joint_info: &HashMap<i32, Joint>,
    joint_absolute_transform: &mut HashMap<i32, Matrix4<f32>>,
    visited: &mut HashSet<i32>,
) -> Matrix4<f32> {
    // Check for cycles
    if !visited.insert(joint_index) {
        panic!("Cycle detected in joint hierarchy at joint {}", joint_index);
    }

    // Return cached value if already computed
    if let Some(&abs_transform) = joint_absolute_transform.get(&joint_index) {
        visited.remove(&joint_index);
        return abs_transform;
    }

    // Get the joint
    let joint = joint_info.get(&joint_index).expect("Joint not found");

    // Compute the absolute transform
    let abs_transform = if let Some(parent_index) = joint.parent {
        let parent_abs_transform = compute_joint_absolute_transform(
            parent_index,
            joint_info,
            joint_absolute_transform,
            visited,
        );
        parent_abs_transform * joint.transform
    } else {
        // No parent, so absolute transform is the joint's transform
        joint.transform
    };

    // Store the computed absolute transform
    joint_absolute_transform.insert(joint_index, abs_transform);

    // Remove from visited set
    visited.remove(&joint_index);

    abs_transform
}

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
    // Find the keyframes surrounding the given time
    for i in 0..keyframes.len() - 1 {
        let kf0 = &keyframes[i];
        let kf1 = &keyframes[i + 1];
        if time >= kf0.time && time <= kf1.time {
            let t = (time - kf0.time) / (kf1.time - kf0.time);
            return Some(interpolate_values(&kf0.value, &kf1.value, t));
        }
    }
    None
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
}
