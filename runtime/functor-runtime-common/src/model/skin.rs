use std::collections::{HashMap, HashSet};

use cgmath::{Matrix4, SquareMatrix};

pub struct Joint {
    pub name: String,
    pub transform: Matrix4<f32>,
    pub parent: Option<i32>,
}

pub struct Skin {
    num_joints: i32,
    inv_bind_matrix: Vec<Matrix4<f32>>,

    // Use HashMap for joints because they could be sparse
    joint_info: HashMap<i32, Joint>,
    joint_absolute_transform: HashMap<i32, Matrix4<f32>>,
}

impl Skin {
    pub fn get_joint_count(&self) -> i32 {
        self.num_joints
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

    pub fn get_joint_absolute_transform(&self, idx: i32) -> Option<&Matrix4<f32>> {
        self.joint_absolute_transform.get(&idx)
    }
}

pub struct SkinBuilder {
    skin: Skin,
}

impl SkinBuilder {
    pub fn create(inv_bind_matrices: Vec<Matrix4<f32>>) -> SkinBuilder {
        SkinBuilder {
            skin: Skin {
                num_joints: inv_bind_matrices.len() as i32,
                inv_bind_matrix: inv_bind_matrices,
                joint_info: HashMap::new(),
                joint_absolute_transform: HashMap::new(),
            },
        }
    }

    pub fn add_joint(
        mut self,
        joint_index: i32,
        name: String,
        parent_index: Option<i32>,
        transform: Matrix4<f32>,
    ) -> Self {
        let joint = Joint {
            name,
            transform,
            parent: parent_index,
        };
        self.skin.joint_info.insert(joint_index, joint);
        self
    }

    pub fn build(mut self) -> Skin {
        // Compute absolute transforms
        let joint_absolute_transform = compute_absolute_transforms(&self.skin.joint_info);

        // Update the skin with the computed absolute transforms
        self.skin.joint_absolute_transform = joint_absolute_transform;

        // Return the built skin
        self.skin
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

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Matrix4, Vector3};

    #[test]
    fn test_absolute_transforms() {
        // Sample inverse bind matrices (identity matrices for simplicity)
        let inv_bind_matrices = vec![Matrix4::identity(); 3];

        // Define the transforms
        let transform_joint_0 = Matrix4::from_translation(Vector3::new(1.0, 0.0, 0.0));
        let transform_joint_1 = Matrix4::from_translation(Vector3::new(0.0, 2.0, 0.0));
        let transform_joint_2 = Matrix4::from_translation(Vector3::new(0.0, 0.0, 3.0));

        // Create the SkinBuilder
        let skin_builder = SkinBuilder::create(inv_bind_matrices)
            .add_joint(0, "Joint0".to_string(), None, transform_joint_0)
            .add_joint(1, "Joint1".to_string(), Some(0), transform_joint_1)
            .add_joint(2, "Joint2".to_string(), Some(1), transform_joint_2);

        // Build the Skin
        let skin = skin_builder.build();

        // Expected absolute transforms
        let expected_abs_transform_joint_0 = transform_joint_0;
        let expected_abs_transform_joint_1 =
            expected_abs_transform_joint_0 * transform_joint_1 * transform_joint_2;
        let expected_abs_transform_joint_2 = expected_abs_transform_joint_1 * transform_joint_2;

        // Retrieve computed absolute transforms
        let abs_transform_joint_0 = skin
            .get_joint_absolute_transform(0)
            .expect("Joint 0 missing");
        let abs_transform_joint_1 = skin
            .get_joint_absolute_transform(1)
            .expect("Joint 1 missing");
        let abs_transform_joint_2 = skin
            .get_joint_absolute_transform(2)
            .expect("Joint 2 missing");

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
