//! Declarative animation expressions — the pose algebra a scene `Model` node
//! carries (`ModelDescription::animation`).
//!
//! An [`AnimExpr`] is plain serializable data: the engine never owns a
//! playhead or a blend weight. Game code derives them (typically from `tts`
//! and model state) and bakes them into the expression each frame; evaluation
//! here is a pure function of `(expression, loaded model)`, so a frame
//! re-rendered from time-travel history reproduces the exact pose.
//!
//! The node set (the Bevy-RFC-51-shaped primitives):
//! - `Clip` samples a named glTF clip at a playhead in seconds (looping).
//! - `Rest` is the bind pose — the base for purely programmatic posing
//!   (e.g. a hand model with no authored clips).
//! - `Blend` mixes sub-poses by normalized weights.
//! - `Add` layers a clip's delta-from-bind on top of a base (a head shake
//!   over a walk).
//! - `Mask` restricts an expression's influence to the subtrees rooted at
//!   the named joints (per-finger poses, upper-body-only layers).
//! - `Rotate` post-multiplies an additive local rotation onto one joint —
//!   the programmatic per-joint control (finger curl, authored offsets).
//! - `LookAt` aims one joint's local +Z axis at a model-space target after
//!   evaluating the expression below it.
//! - `Reach` rotates a direct two-bone chain so its end joint approaches a
//!   model-space target, preserving the evaluated pose's elbow bend side.
//! - `Orient` sets one joint's complete model-space rotation after
//!   evaluating the expression below it (a wrist matching a controller).
//!
//! Evaluation carries a per-joint influence weight so `Mask` composes
//! through `Blend`/`Add`: a joint no input drives falls back to the bind
//! pose, and partial influence blends toward it.

use cgmath::{
    Euler, InnerSpace, Matrix3, Matrix4, One, Quaternion, Rad, Rotation, SquareMatrix, Vector3,
    Vector4, Zero,
};
use serde::{Deserialize, Serialize};

use crate::animation::Animation;
use crate::model::{JointPose, Model, Pose, Skeleton};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnimExpr {
    /// Sample the clip named `name` at `playhead` seconds. The playhead wraps
    /// by the clip's duration (negative playheads wrap backwards from the
    /// end), so a monotone `tts` plays a loop.
    Clip { name: String, playhead: f32 },
    /// The bind (rest) pose — the base expression for purely programmatic
    /// posing via `Rotate` on models with no authored clips.
    Rest,
    /// Weighted mix of sub-expressions. Non-positive weights are skipped;
    /// the rest are normalized per joint (so a `Mask`ed input simply drops
    /// out of the joints it does not cover). A joint driven by no input is
    /// the bind pose.
    Blend(Vec<(AnimExpr, f32)>),
    /// Additive layer: `base + weight * (layer - bind)` per joint —
    /// rotations compose as `base_r * slerp(identity, bind_r⁻¹ * layer_r, w)`,
    /// translations/scales add the weighted delta. The effective weight
    /// (`weight * layer joint weight`) clamps to `[0, 1]`, and the delta
    /// only applies where the BASE has influence — a joint the base does
    /// not drive stays at bind (use `Rotate` for unconditional per-joint
    /// control).
    Add {
        base: Box<AnimExpr>,
        layer: Box<AnimExpr>,
        weight: f32,
    },
    /// Restrict `expr`'s influence to the subtrees rooted at the named
    /// joints (a name covers itself and every descendant).
    Mask {
        joints: Vec<String>,
        expr: Box<AnimExpr>,
    },
    /// Post-multiply an additive local rotation (XYZ Euler, radians) onto
    /// one joint of `expr` — the programmatic per-joint control. The joint
    /// becomes fully driven (weight 1), so the rotation survives masks and
    /// zero-weight blends BENEATH this node (an enclosing `Mask` that
    /// excludes the joint still drops it).
    Rotate {
        joint: String,
        /// XYZ Euler angles in radians, applied in the joint's local frame.
        euler: [f32; 3],
        expr: Box<AnimExpr>,
    },
    /// Aim one joint's local +Z axis at a model-space target after evaluating
    /// `expr`. The shortest correction is clamped to `max_angle` radians and
    /// blended by `weight`.
    LookAt {
        joint: String,
        target: [f32; 3],
        max_angle: f32,
        weight: f32,
        expr: Box<AnimExpr>,
    },
    /// Rotate a direct `root -> middle -> end` chain so the end joint reaches
    /// a model-space target after evaluating `expr`. Unreachable targets clamp
    /// to the chain's nearest possible extension; the evaluated middle joint
    /// supplies the bend side.
    Reach {
        root: String,
        middle: String,
        end: String,
        target: [f32; 3],
        weight: f32,
        expr: Box<AnimExpr>,
    },
    /// Set one joint's model-space rotation (`[x, y, z, w]`, normalized at
    /// evaluation; `q` and `-q` are the same) after evaluating `expr`,
    /// keeping its translation and scale. `weight` slerps from the
    /// evaluated rotation.
    Orient {
        joint: String,
        rotation: [f32; 4],
        weight: f32,
        expr: Box<AnimExpr>,
    },
}

/// A missing reference discovered during evaluation — the caller surfaces
/// these (deduped) to the developer; evaluation falls back to the bind pose
/// (missing clip) or ignores the node (missing joint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimWarning<'a> {
    MissingClip(&'a str),
    MissingJoint(&'a str),
    InvalidChain {
        root: &'a str,
        middle: &'a str,
        end: &'a str,
    },
    NonUniformRootScale {
        root: &'a str,
    },
}

/// A pose with a per-joint influence weight in `[0, 1]` — how `Mask`
/// composes through the tree. `finalize` folds it back onto the bind pose.
/// Indexed by skeleton slot; `None` is a joint no input produced.
struct WeightedPose {
    joints: Vec<Option<(JointPose, f32)>>,
}

/// Evaluate an expression against a loaded model and return the skinning
/// matrices (`absolute * inverse_bind` per joint) the skinned materials
/// consume. `on_warning` fires for every unknown clip/joint name (each
/// frame — the caller dedups, e.g. `SceneContext::warn_once`).
pub fn skinning_transforms(
    model: &Model,
    expr: &AnimExpr,
    on_warning: &mut dyn FnMut(AnimWarning),
) -> Vec<Matrix4<f32>> {
    rig_skinning_transforms(&model.skeleton, &model.animations, expr, on_warning)
}

/// [`skinning_transforms`] for a bare skeleton and clips — `Model.pose`
/// poses on the sim side, which has no GPU [`Model`].
pub fn rig_skinning_transforms(
    skeleton: &Skeleton,
    animations: &[Animation],
    expr: &AnimExpr,
    on_warning: &mut dyn FnMut(AnimWarning),
) -> Vec<Matrix4<f32>> {
    let rig = Rig {
        skeleton,
        animations,
    };
    let weighted = eval(rig, expr, on_warning);
    let pose = finalize(skeleton, weighted);
    skeleton.skinning_transforms_for(&pose)
}

/// What evaluation reads from a model.
#[derive(Clone, Copy)]
struct Rig<'a> {
    skeleton: &'a Skeleton,
    animations: &'a [Animation],
}

/// Fold a weighted pose onto the bind pose: fully-driven joints take the
/// evaluated pose, undriven joints stay at bind, partial influence nlerps
/// between the two.
fn finalize(skeleton: &Skeleton, weighted: WeightedPose) -> Pose {
    let mut pose = skeleton.base_pose();
    for (bind, joint) in pose.joints.iter_mut().zip(weighted.joints) {
        let Some((jp, weight)) = joint else {
            continue;
        };
        if weight >= 1.0 {
            *bind = jp;
        } else if weight > 0.0 {
            *bind = mix_joint(bind, &jp, weight);
        }
    }
    pose
}

/// nlerp two joint poses (`t` toward `b`), sign-aligning the quaternions.
fn mix_joint(a: &JointPose, b: &JointPose, t: f32) -> JointPose {
    let rotation_b = if a.rotation.dot(b.rotation) < 0.0 {
        -b.rotation
    } else {
        b.rotation
    };
    let rotation = (a.rotation * (1.0 - t) + rotation_b * t).normalize();
    JointPose {
        translation: a.translation * (1.0 - t) + b.translation * t,
        rotation,
        scale: a.scale * (1.0 - t) + b.scale * t,
    }
}

fn eval(model: Rig, expr: &AnimExpr, on_warning: &mut dyn FnMut(AnimWarning)) -> WeightedPose {
    match expr {
        AnimExpr::Clip { name, playhead } => {
            match model.animations.iter().find(|a| a.name == *name) {
                Some(animation) => {
                    let time = if animation.duration > 0.0 {
                        playhead.rem_euclid(animation.duration)
                    } else {
                        0.0
                    };
                    full_weight(model.skeleton.sample_pose(animation, time))
                }
                None => {
                    on_warning(AnimWarning::MissingClip(name));
                    full_weight(model.skeleton.base_pose())
                }
            }
        }
        AnimExpr::Rest => full_weight(model.skeleton.base_pose()),
        AnimExpr::Blend(items) => {
            let inputs: Vec<(WeightedPose, f32)> = items
                .iter()
                .filter(|(_, weight)| *weight > 0.0)
                .map(|(sub, weight)| (eval(model, sub, on_warning), *weight))
                .collect();
            blend(&inputs, model.skeleton.slot_count())
        }
        AnimExpr::Add {
            base,
            layer,
            weight,
        } => {
            let mut base = eval(model, base, on_warning);
            if *weight > 0.0 {
                let layer = eval(model, layer, on_warning);
                add_layer(&mut base, &layer, &model.skeleton, *weight);
            }
            base
        }
        AnimExpr::Mask { joints, expr } => {
            let mut inner = eval(model, expr, on_warning);
            let kept = model.skeleton.subtree_slots(joints);
            for name in joints {
                if model.skeleton.joint_id_by_name(name).is_none() {
                    on_warning(AnimWarning::MissingJoint(name));
                }
            }
            for (joint, kept) in inner.joints.iter_mut().zip(kept) {
                if let Some((_, weight)) = joint {
                    if !kept {
                        *weight = 0.0;
                    }
                }
            }
            inner
        }
        AnimExpr::Rotate { joint, euler, expr } => {
            let mut inner = eval(model, expr, on_warning);
            match model.skeleton.slot_by_name(joint) {
                Some(slot) => {
                    // The inner expression may not have produced this joint
                    // (an all-zero-weight blend evaluates empty): rotate over
                    // the bind pose so the joint never silently snaps.
                    let (jp, weight) = inner.joints[slot]
                        .get_or_insert((model.skeleton.slot_bind_pose(slot), 0.0));
                    let q =
                        Quaternion::from(Euler::new(Rad(euler[0]), Rad(euler[1]), Rad(euler[2])));
                    jp.rotation = (jp.rotation * q).normalize();
                    // The joint is explicitly driven — the rotation
                    // survives a mask/blend BENEATH this node (an
                    // enclosing Mask that excludes the joint still
                    // zeroes it, deliberately).
                    *weight = 1.0;
                }
                None => on_warning(AnimWarning::MissingJoint(joint)),
            }
            inner
        }
        AnimExpr::LookAt {
            joint,
            target,
            max_angle,
            weight,
            expr,
        } => {
            let mut inner = eval(model, expr, on_warning);
            match model.skeleton.slot_by_name(joint) {
                Some(slot) => {
                    apply_look_at(
                        &model.skeleton,
                        &mut inner,
                        slot,
                        Vector3::from(*target),
                        *max_angle,
                        *weight,
                    );
                }
                None => on_warning(AnimWarning::MissingJoint(joint)),
            }
            inner
        }
        AnimExpr::Reach {
            root,
            middle,
            end,
            target,
            weight,
            expr,
        } => {
            let mut inner = eval(model, expr, on_warning);
            let root_id = model.skeleton.slot_by_name(root);
            let middle_id = model.skeleton.slot_by_name(middle);
            let end_id = model.skeleton.slot_by_name(end);
            for (name, id) in [
                (root.as_str(), root_id),
                (middle.as_str(), middle_id),
                (end.as_str(), end_id),
            ] {
                if id.is_none() {
                    on_warning(AnimWarning::MissingJoint(name));
                }
            }
            if let (Some(root_id), Some(middle_id), Some(end_id)) = (root_id, middle_id, end_id) {
                match apply_reach(
                    &model.skeleton,
                    &mut inner,
                    root_id,
                    middle_id,
                    end_id,
                    Vector3::from(*target),
                    *weight,
                ) {
                    Ok(()) => {}
                    Err(ReachFailure::InvalidChain) => {
                        on_warning(AnimWarning::InvalidChain { root, middle, end });
                    }
                    Err(ReachFailure::NonUniformRootScale) => {
                        on_warning(AnimWarning::NonUniformRootScale { root });
                    }
                }
            }
            inner
        }
        AnimExpr::Orient {
            joint,
            rotation,
            weight,
            expr,
        } => {
            let mut inner = eval(model, expr, on_warning);
            match model.skeleton.slot_by_name(joint) {
                Some(slot) => apply_orient(&model.skeleton, &mut inner, slot, *rotation, *weight),
                None => on_warning(AnimWarning::MissingJoint(joint)),
            }
            inner
        }
    }
}

/// Resolve one joint exactly as `finalize` would, without allocating a full
/// pose. Look-at only needs the target joint and its ancestor chain, so this
/// keeps the post-pass proportional to hierarchy depth.
fn resolved_joint_pose(skeleton: &Skeleton, pose: &WeightedPose, slot: usize) -> JointPose {
    match &pose.joints[slot] {
        Some((joint, weight)) if *weight >= 1.0 => *joint,
        Some((joint, weight)) if *weight > 0.0 => {
            mix_joint(&skeleton.slot_bind_pose(slot), joint, *weight)
        }
        _ => skeleton.slot_bind_pose(slot),
    }
}

struct ResolvedAbsolute {
    transform: Matrix4<f32>,
    parent_transform: Matrix4<f32>,
}

fn resolved_absolute(skeleton: &Skeleton, pose: &WeightedPose, slot: usize) -> ResolvedAbsolute {
    let local = resolved_joint_pose(skeleton, pose, slot);
    let parent_transform = match skeleton.slot_parent(slot) {
        Some(parent) => resolved_absolute(skeleton, pose, parent).transform,
        None => Matrix4::one(),
    };
    ResolvedAbsolute {
        transform: parent_transform * local.transform(),
        parent_transform,
    }
}

fn scaled_shortest_arc(
    from: Vector3<f32>,
    to: Vector3<f32>,
    rear_pole: Vector3<f32>,
    max_angle: f32,
    weight: f32,
) -> Quaternion<f32> {
    // At the exact antipode there are infinitely many shortest arcs. Supplying
    // the joint's current up axis makes that otherwise singular choice stable:
    // rearward targets turn around local up instead of whichever arbitrary
    // world axis happens to survive the cross product.
    let correction = Quaternion::from_arc(from, to, Some(rear_pole));
    let angle = 2.0 * correction.s.clamp(-1.0, 1.0).acos();
    if angle <= 1e-6 {
        return Quaternion::one();
    }
    let fraction = (max_angle.max(0.0) / angle).min(1.0) * weight.clamp(0.0, 1.0);
    Quaternion::one().slerp(correction, fraction)
}

fn apply_look_at(
    skeleton: &Skeleton,
    pose: &mut WeightedPose,
    slot: usize,
    target: Vector3<f32>,
    max_angle: f32,
    weight: f32,
) {
    if max_angle <= 0.0 || weight <= 0.0 {
        return;
    }
    let mut joint = resolved_joint_pose(skeleton, pose, slot);
    let absolute = resolved_absolute(skeleton, pose, slot);
    let offset = target - absolute.transform.w.truncate();
    if offset.magnitude2() <= 1e-12 {
        return;
    }

    let parent_linear = Matrix3::from_cols(
        absolute.parent_transform.x.truncate(),
        absolute.parent_transform.y.truncate(),
        absolute.parent_transform.z.truncate(),
    );
    let Some(parent_inverse) = parent_linear.invert() else {
        return;
    };
    let desired_in_parent = parent_inverse * offset;
    if desired_in_parent.magnitude2() <= 1e-12 {
        return;
    }
    let current_in_parent = joint.rotation.rotate_vector(Vector3::unit_z());
    let rear_pole = joint.rotation.rotate_vector(Vector3::unit_y());
    let local_correction = scaled_shortest_arc(
        current_in_parent,
        desired_in_parent.normalize(),
        rear_pole,
        max_angle,
        weight,
    );
    joint.rotation = (local_correction * joint.rotation).normalize();
    pose.joints[slot] = Some((joint, 1.0));
}

/// The rotation part of a settled transform: its X and Y axes
/// Gram-Schmidt-orthonormalized (Z = X × Y). Exact for rotation times a
/// per-axis scale; a sheared ancestor (non-uniform scale above a rotation)
/// gets the orthonormal frame nearest its X axis.
fn rotation_of(transform: Matrix4<f32>) -> Option<Quaternion<f32>> {
    let x = normalized_f64(transform.x.truncate())?;
    let y = transform.y.truncate();
    let y = normalized_f64(y - x * y.dot(x))?;
    Some(Quaternion::from(Matrix3::from_cols(x, y, x.cross(y))).normalize())
}

/// Set `slot`'s model-space rotation over the settled parent, keeping its
/// local translation and scale. A zero/non-finite quaternion is ignored (the
/// prelude rejects one at the call).
fn apply_orient(
    skeleton: &Skeleton,
    pose: &mut WeightedPose,
    slot: usize,
    rotation: [f32; 4],
    weight: f32,
) {
    if !(weight > 0.0) {
        return;
    }
    // Normalize in f64 so a huge-but-finite quaternion cannot overflow.
    let [x, y, z, w] = rotation.map(f64::from);
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if !length.is_finite() || length <= 1e-12 {
        return;
    }
    let target = Quaternion::new(
        (w / length) as f32,
        (x / length) as f32,
        (y / length) as f32,
        (z / length) as f32,
    );
    let parent = match skeleton.slot_parent(slot) {
        Some(parent) => match rotation_of(resolved_absolute(skeleton, pose, parent).transform) {
            Some(parent) => parent,
            None => return,
        },
        None => Quaternion::one(),
    };
    let mut joint = resolved_joint_pose(skeleton, pose, slot);
    // Local = parent⁻¹ · target, sign-aligned with the evaluated rotation so
    // `q` and `-q` take the same shortest path.
    let mut local = (parent.conjugate() * target).normalize();
    if joint.rotation.dot(local) < 0.0 {
        local = -local;
    }
    joint.rotation = if weight >= 1.0 {
        local
    } else {
        joint.rotation.slerp(local, weight)
    };
    pose.joints[slot] = Some((joint, 1.0));
}

/// A deterministic direction perpendicular to `axis`, preferring `hint`
/// when it already carries a useful bend side.
fn perpendicular(axis: Vector3<f32>, hint: Vector3<f32>) -> Vector3<f32> {
    let hint = normalized_f64(hint).unwrap_or_else(Vector3::zero);
    let projected = hint - axis * hint.dot(axis);
    if let Some(direction) = normalized_f64(projected) {
        return direction;
    }
    let basis = if axis.x.abs() <= axis.y.abs() && axis.x.abs() <= axis.z.abs() {
        Vector3::unit_x()
    } else if axis.y.abs() <= axis.z.abs() {
        Vector3::unit_y()
    } else {
        Vector3::unit_z()
    };
    normalized_f64(basis - axis * basis.dot(axis)).unwrap_or_else(Vector3::unit_x)
}

fn transform_point(transform: Matrix4<f32>, point: Vector3<f32>) -> Option<Vector3<f32>> {
    let transformed = transform * Vector4::new(point.x, point.y, point.z, 1.0);
    if transformed.w.abs() <= 1e-12 {
        None
    } else {
        Some(transformed.truncate() / transformed.w)
    }
}

fn length_f64(vector: Vector3<f32>) -> f64 {
    let x = f64::from(vector.x);
    let y = f64::from(vector.y);
    let z = f64::from(vector.z);
    (x * x + y * y + z * z).sqrt()
}

fn normalized_f64(vector: Vector3<f32>) -> Option<Vector3<f32>> {
    let length = length_f64(vector);
    if !length.is_finite() || length <= 1e-12 {
        return None;
    }
    Some(Vector3::new(
        (f64::from(vector.x) / length) as f32,
        (f64::from(vector.y) / length) as f32,
        (f64::from(vector.z) / length) as f32,
    ))
}

/// Rotate one joint so the model-space segment below it turns from `current`
/// toward `desired`. Converting both vectors through the settled parent
/// transform keeps the local correction correct under animated ancestors.
fn rotate_segment_toward(
    skeleton: &Skeleton,
    pose: &mut WeightedPose,
    slot: usize,
    current: Vector3<f32>,
    desired: Vector3<f32>,
    bend_normal: Vector3<f32>,
    weight: f32,
) -> bool {
    let mut joint = resolved_joint_pose(skeleton, pose, slot);
    let absolute = resolved_absolute(skeleton, pose, slot);
    let parent_linear = Matrix3::from_cols(
        absolute.parent_transform.x.truncate(),
        absolute.parent_transform.y.truncate(),
        absolute.parent_transform.z.truncate(),
    );
    let Some(parent_inverse) = parent_linear.invert() else {
        return false;
    };
    let from = parent_inverse * current;
    let to = parent_inverse * desired;
    let Some(from) = normalized_f64(from) else {
        return false;
    };
    let Some(to) = normalized_f64(to) else {
        return false;
    };
    let pole = perpendicular(from, parent_inverse * bend_normal);
    let correction = Quaternion::from_arc(from, to, Some(pole));
    let correction = Quaternion::one().slerp(correction, weight.clamp(0.0, 1.0));
    joint.rotation = (correction * joint.rotation).normalize();
    pose.joints[slot] = Some((joint, 1.0));
    true
}

/// Analytic two-bone reach over a direct `root -> middle -> end` chain.
///
/// The law of cosines chooses the desired middle position. The middle joint
/// from the evaluated pose acts as the pole point, which preserves the
/// authored/animated elbow side without adding hidden state or another API
/// parameter. Each local correction is derived after its parent has settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReachFailure {
    InvalidChain,
    NonUniformRootScale,
}

fn apply_reach(
    skeleton: &Skeleton,
    pose: &mut WeightedPose,
    root_id: usize,
    middle_id: usize,
    end_id: usize,
    target: Vector3<f32>,
    weight: f32,
) -> Result<(), ReachFailure> {
    if skeleton.slot_parent(middle_id) != Some(root_id)
        || skeleton.slot_parent(end_id) != Some(middle_id)
    {
        return Err(ReachFailure::InvalidChain);
    }
    if weight <= 0.0 {
        return Ok(());
    }
    let root_joint = resolved_joint_pose(skeleton, pose, root_id);
    let scale = root_joint.scale.map(f32::abs);
    let largest_scale = scale.x.max(scale.y).max(scale.z);
    let smallest_scale = scale.x.min(scale.y).min(scale.z);
    if largest_scale - smallest_scale > largest_scale * 1e-4 {
        return Err(ReachFailure::NonUniformRootScale);
    }

    let root_absolute = resolved_absolute(skeleton, pose, root_id);
    let middle_absolute = resolved_absolute(skeleton, pose, middle_id);
    let end_absolute = resolved_absolute(skeleton, pose, end_id);
    let Some(model_to_solve) = root_absolute.parent_transform.invert() else {
        return Err(ReachFailure::InvalidChain);
    };
    let solve_to_model = root_absolute.parent_transform;
    let Some(root) = transform_point(model_to_solve, root_absolute.transform.w.truncate()) else {
        return Err(ReachFailure::InvalidChain);
    };
    let Some(middle) = transform_point(model_to_solve, middle_absolute.transform.w.truncate())
    else {
        return Err(ReachFailure::InvalidChain);
    };
    let Some(end) = transform_point(model_to_solve, end_absolute.transform.w.truncate()) else {
        return Err(ReachFailure::InvalidChain);
    };
    let Some(target) = transform_point(model_to_solve, target) else {
        return Err(ReachFailure::InvalidChain);
    };
    let upper = middle - root;
    let lower = end - middle;
    let upper_length = length_f64(upper);
    let lower_length = length_f64(lower);
    if upper_length <= 1e-6 || lower_length <= 1e-6 {
        return Err(ReachFailure::InvalidChain);
    }

    let target_offset = target - root;
    let target_distance = length_f64(target_offset);
    let current_offset = end - root;
    let target_direction = if target_distance > 1e-6 {
        normalized_f64(target_offset).ok_or(ReachFailure::InvalidChain)?
    } else if let Some(direction) = normalized_f64(current_offset) {
        direction
    } else {
        normalized_f64(upper).ok_or(ReachFailure::InvalidChain)?
    };
    let maximum = upper_length + lower_length;
    let minimum = (upper_length - lower_length).abs().max(maximum * 1e-5);
    let distance = target_distance.clamp(minimum, maximum);

    // Use the evaluated middle joint as the pole point. Its projection onto
    // the plane normal to the target direction says which side the elbow was
    // already on; a straight pose gets a deterministic model-axis fallback.
    let bend_direction = perpendicular(target_direction, upper);
    let along = (upper_length * upper_length - lower_length * lower_length + distance * distance)
        / (2.0 * distance);
    let height = (upper_length * upper_length - along * along)
        .max(0.0)
        .sqrt();
    let desired_middle =
        root + target_direction * along as f32 + bend_direction * height as f32;
    let desired_end = root + target_direction * distance as f32;
    let Some(root_model) = transform_point(solve_to_model, root) else {
        return Err(ReachFailure::InvalidChain);
    };
    let Some(desired_middle_model) = transform_point(solve_to_model, desired_middle) else {
        return Err(ReachFailure::InvalidChain);
    };
    let Some(desired_end_model) = transform_point(solve_to_model, desired_end) else {
        return Err(ReachFailure::InvalidChain);
    };
    let desired_upper = desired_middle_model - root_model;
    let desired_lower = desired_end_model - desired_middle_model;
    let desired_upper_direction =
        normalized_f64(desired_upper).ok_or(ReachFailure::InvalidChain)?;
    let desired_lower_direction =
        normalized_f64(desired_lower).ok_or(ReachFailure::InvalidChain)?;
    let bend_normal = desired_upper_direction.cross(desired_lower_direction);
    let bend_normal = if let Some(direction) = normalized_f64(bend_normal) {
        direction
    } else {
        normalized_f64(target_direction.cross(bend_direction))
            .ok_or(ReachFailure::InvalidChain)?
    };

    if !rotate_segment_toward(
        skeleton,
        pose,
        root_id,
        middle_absolute.transform.w.truncate() - root_absolute.transform.w.truncate(),
        desired_upper,
        bend_normal,
        weight,
    ) {
        return Err(ReachFailure::InvalidChain);
    }

    // Root rotation moved both descendants. Resolve the settled second
    // segment before computing the middle-joint correction.
    let middle_after = resolved_absolute(skeleton, pose, middle_id);
    let end_after = resolved_absolute(skeleton, pose, end_id);
    let middle_after = middle_after.transform.w.truncate();
    let lower_after = end_after.transform.w.truncate() - middle_after;
    if rotate_segment_toward(
        skeleton,
        pose,
        middle_id,
        lower_after,
        desired_end_model - middle_after,
        bend_normal,
        weight,
    ) {
        Ok(())
    } else {
        Err(ReachFailure::InvalidChain)
    }
}

fn full_weight(pose: Pose) -> WeightedPose {
    WeightedPose {
        joints: pose.joints.into_iter().map(|jp| Some((jp, 1.0))).collect(),
    }
}

/// Per-joint weighted nlerp mixing (sign-aligned accumulation, as in Bevy's
/// RFC-51 blending). Each input's effective weight at a joint is
/// `input_weight * joint_weight`, so a masked input drops out of joints it
/// does not cover and the remaining inputs renormalize. The result's joint
/// weight is the coverage-weighted average of the covering inputs' joint
/// weights — 1 for a plain blend (weights fully normalize, whatever they
/// sum to), and fractional only when every covering input was itself
/// partially driven (nested masks), which `finalize` folds toward bind.
/// Degenerate accumulations (zero/non-finite totals, opposed rotations
/// cancelling) fall back to the first input's joint.
fn blend(inputs: &[(WeightedPose, f32)], slot_count: usize) -> WeightedPose {
    let mut joints = vec![None; slot_count];
    let Some((first, _)) = inputs.first() else {
        return WeightedPose { joints };
    };
    for (slot, joint) in joints.iter_mut().enumerate() {
        let reference = first.joints[slot]
            .map(|(jp, _)| jp.rotation)
            .unwrap_or_else(|| Quaternion::new(1.0, 0.0, 0.0, 0.0));
        let mut translation = Vector3::zero();
        let mut scale = Vector3::zero();
        let mut rotation = Quaternion::zero();
        let mut total = 0.0_f32;
        // The weight sum of the inputs that cover this joint at all — the
        // denominator that renormalizes per joint (a masked-out input drops
        // out entirely instead of diluting toward bind).
        let mut covering = 0.0_f32;
        let mut fallback: Option<JointPose> = None;
        for (input, input_weight) in inputs {
            let Some((jp, joint_weight)) = &input.joints[slot] else {
                continue;
            };
            fallback.get_or_insert(*jp);
            let effective = input_weight * joint_weight;
            if effective <= 0.0 {
                continue;
            }
            total += effective;
            covering += input_weight;
            translation += jp.translation * effective;
            scale += jp.scale * effective;
            // Align hemispheres so opposite-sign encodings of the same
            // rotation reinforce instead of cancelling.
            let q = if reference.dot(jp.rotation) < 0.0 {
                -jp.rotation
            } else {
                jp.rotation
            };
            rotation += q * effective;
        }
        let magnitude2 = rotation.magnitude2();
        if total > 0.0
            && total.is_finite()
            && covering > 0.0
            && covering.is_finite()
            && magnitude2.is_finite()
            && magnitude2 > 1e-12
        {
            let blended = JointPose {
                translation: translation / total,
                rotation: rotation.normalize(),
                scale: scale / total,
            };
            *joint = Some((blended, (total / covering).min(1.0)));
        } else if let Some(jp) = fallback {
            *joint = Some((jp, 0.0));
        }
    }
    WeightedPose { joints }
}

/// Apply an additive layer: per joint, `base + weight * (layer - bind)`,
/// scaled additionally by the layer's own joint weight (so a masked layer
/// only adds where it covers). Rotations compose as
/// `base_r * slerp(identity, bind_r⁻¹ * layer_r, w)`.
fn add_layer(base: &mut WeightedPose, layer: &WeightedPose, skeleton: &Skeleton, weight: f32) {
    for (slot, (base, layer)) in base.joints.iter_mut().zip(&layer.joints).enumerate() {
        let (Some((base_jp, _)), Some((layer_jp, layer_weight))) = (base, layer) else {
            continue;
        };
        let bind_jp = skeleton.slot_bind_pose(slot);
        let w = (weight * layer_weight).clamp(0.0, 1.0);
        if w <= 0.0 {
            continue;
        }
        let mut delta = bind_jp.rotation.invert() * layer_jp.rotation;
        // Shortest arc: identity's hemisphere is positive scalar part.
        if delta.s < 0.0 {
            delta = -delta;
        }
        let identity = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let scaled = if delta.s > 0.9995 {
            // Nearly identity — nlerp to dodge slerp's tiny-angle instability.
            (identity * (1.0 - w) + delta * w).normalize()
        } else {
            identity.slerp(delta, w)
        };
        base_jp.rotation = (base_jp.rotation * scaled).normalize();
        base_jp.translation += (layer_jp.translation - bind_jp.translation) * w;
        base_jp.scale += (layer_jp.scale - bind_jp.scale) * w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{
        Animation, AnimationChannel, AnimationProperty, AnimationValue, Keyframe,
    };
    use crate::model::SkeletonBuilder;
    use cgmath::{vec3, Deg, Matrix4, Rotation3, SquareMatrix};

    fn clip(name: &str, playhead: f32) -> AnimExpr {
        AnimExpr::Clip {
            name: name.to_string(),
            playhead,
        }
    }

    /// A one-joint model with two clips: `slide` translates the joint from
    /// x=0 to x=2 over 2s; `lift` translates it from y=0 to y=4 over 2s.
    fn test_model() -> Model {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity()]);
        builder.add_joint(0, 0, "root".to_string(), None, Matrix4::identity());
        let skeleton = builder.build();

        let translation_clip = |name: &str, to: Vector3<f32>| Animation {
            name: name.to_string(),
            duration: 2.0,
            channels: vec![AnimationChannel {
                target_node_index: 0,
                target_property: AnimationProperty::Translation,
                keyframes: vec![
                    Keyframe {
                        time: 0.0,
                        value: AnimationValue::Translation(vec3(0.0, 0.0, 0.0)),
                    },
                    Keyframe {
                        time: 2.0,
                        value: AnimationValue::Translation(to),
                    },
                ],
            }],
        };

        Model {
            meshes: vec![],
            skeleton,
            animations: vec![
                translation_clip("slide", vec3(2.0, 0.0, 0.0)),
                translation_clip("lift", vec3(0.0, 4.0, 0.0)),
            ],
        }
    }

    /// A three-joint chain (root -> arm -> hand), all at identity bind, with
    /// a `raise` clip translating every joint to y=2 over 2s.
    fn chain_model() -> Model {
        let mut builder =
            SkeletonBuilder::create(vec![Matrix4::identity(); 3]);
        builder.add_joint(0, 0, "root".to_string(), None, Matrix4::identity());
        builder.add_joint(1, 1, "arm".to_string(), Some(0), Matrix4::identity());
        builder.add_joint(2, 2, "hand".to_string(), Some(1), Matrix4::identity());
        let skeleton = builder.build();
        let channel = |joint: usize| AnimationChannel {
            target_node_index: joint,
            target_property: AnimationProperty::Translation,
            keyframes: vec![
                Keyframe {
                    time: 0.0,
                    value: AnimationValue::Translation(vec3(0.0, 0.0, 0.0)),
                },
                Keyframe {
                    time: 2.0,
                    value: AnimationValue::Translation(vec3(0.0, 2.0, 0.0)),
                },
            ],
        };
        Model {
            meshes: vec![],
            skeleton,
            animations: vec![Animation {
                name: "raise".to_string(),
                duration: 2.0,
                channels: vec![channel(0), channel(1), channel(2)],
            }],
        }
    }

    /// A direct two-bone chain with unit segments along +X:
    /// upper(root at origin) -> lower(at x=1) -> end(at x=2).
    fn reach_model() -> Model {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 3]);
        builder.add_joint(0, 0, "upper".to_string(), None, Matrix4::identity());
        builder.add_joint(
            1,
            1,
            "lower".to_string(),
            Some(0),
            Matrix4::from_translation(Vector3::unit_x()),
        );
        builder.add_joint(
            2,
            2,
            "end".to_string(),
            Some(1),
            Matrix4::from_translation(Vector3::unit_x()),
        );
        Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        }
    }

    fn joint_translation(transforms: &[Matrix4<f32>], index: usize) -> Vector3<f32> {
        transforms[index].w.truncate()
    }

    fn no_warning(warning: AnimWarning) {
        panic!("unexpected warning: {warning:?}");
    }

    #[test]
    fn clip_samples_at_playhead_and_loops() {
        let model = test_model();
        let at = |playhead: f32| {
            joint_translation(
                &skinning_transforms(&model, &clip("slide", playhead), &mut no_warning),
                0,
            )
        };
        assert_eq!(at(0.0), vec3(0.0, 0.0, 0.0));
        assert_eq!(at(1.0), vec3(1.0, 0.0, 0.0));
        // Wraps: 3s into a 2s clip is 1s in; negative playheads wrap backwards.
        assert_eq!(at(3.0), vec3(1.0, 0.0, 0.0));
        assert_eq!(at(-0.5), vec3(1.5, 0.0, 0.0));
    }

    #[test]
    fn rest_is_the_bind_pose() {
        let model = test_model();
        let t = joint_translation(
            &skinning_transforms(&model, &AnimExpr::Rest, &mut no_warning),
            0,
        );
        assert_eq!(t, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn blend_mixes_by_normalized_weight() {
        let model = test_model();
        let expr = AnimExpr::Blend(vec![(clip("slide", 1.0), 1.0), (clip("lift", 1.0), 3.0)]);
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_warning), 0);
        // At 1s, slide is at (1,0,0) and lift at (0,2,0); weights 1:3
        // normalize to 0.25/0.75.
        assert_eq!(t, vec3(0.25, 1.5, 0.0));
    }

    #[test]
    fn blend_weights_fully_normalize_regardless_of_total() {
        // Weights that don't sum to 1 must NOT leak the bind pose — a
        // single 0.5-weight input IS that pose (the #287 semantics).
        let model = test_model();
        let expr = AnimExpr::Blend(vec![(clip("slide", 1.0), 0.5)]);
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_warning), 0);
        assert_eq!(t, vec3(1.0, 0.0, 0.0));
    }

    #[test]
    fn masked_input_drops_out_instead_of_diluting() {
        // A sub-1 total with a mask: joints the mask excludes get the OTHER
        // input at full strength, not a mix toward bind.
        let model = chain_model();
        let expr = AnimExpr::Blend(vec![
            (clip("raise", 1.0), 0.5),
            (
                AnimExpr::Mask {
                    joints: vec!["hand".to_string()],
                    expr: Box::new(clip("raise", 1.0)),
                },
                0.5,
            ),
        ]);
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        // root is covered only by input 1 (weight 0.5): full clip pose, no
        // bind dilution — local y=1.
        assert_eq!(joint_translation(&transforms, 0), vec3(0.0, 1.0, 0.0));
    }

    #[test]
    fn rotate_applies_over_an_empty_blend() {
        // An all-zero-weight blend evaluates to nothing — Rotate must still
        // drive its joint (over the bind pose) instead of silently no-oping.
        let model = chain_model();
        let expr = AnimExpr::Rotate {
            joint: "hand".to_string(),
            euler: [0.0, 0.0, std::f32::consts::FRAC_PI_2],
            expr: Box::new(AnimExpr::Blend(vec![(clip("raise", 1.0), 0.0)])),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let x_axis = transforms[2].x.truncate();
        assert!((x_axis.y - 1.0).abs() < 1e-5, "x axis: {x_axis:?}");
    }

    #[test]
    fn blend_skips_non_positive_weights_and_empty_is_bind_pose() {
        let model = test_model();
        let expr = AnimExpr::Blend(vec![(clip("slide", 1.0), 1.0), (clip("slide", 1.0), 0.0)]);
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_warning), 0);
        assert_eq!(t, vec3(1.0, 0.0, 0.0));

        let empty = AnimExpr::Blend(vec![(clip("slide", 1.0), 0.0)]);
        let t = joint_translation(&skinning_transforms(&model, &empty, &mut no_warning), 0);
        assert_eq!(t, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn blend_with_overflowing_weights_stays_finite() {
        let model = test_model();
        let expr = AnimExpr::Blend(vec![
            (clip("slide", 1.0), f32::MAX),
            (clip("lift", 1.0), f32::MAX),
        ]);
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_warning), 0);
        // Falls back to the first input ("slide" at 1s) with weight 0 → bind
        // pose at the top level; either way, finite.
        assert!(t.x.is_finite() && t.y.is_finite() && t.z.is_finite());
    }

    #[test]
    fn missing_clip_reports_and_falls_back_to_bind_pose() {
        let model = test_model();
        let mut missing = Vec::new();
        let transforms = skinning_transforms(&model, &clip("nope", 0.0), &mut |warning| {
            if let AnimWarning::MissingClip(name) = warning {
                missing.push(name.to_string());
            }
        });
        assert_eq!(missing, vec!["nope".to_string()]);
        assert_eq!(joint_translation(&transforms, 0), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn mask_restricts_to_the_named_subtree() {
        let model = chain_model();
        // Mask the raise clip to the arm subtree: root stays at bind, arm and
        // hand (its descendant) follow the clip.
        let expr = AnimExpr::Mask {
            joints: vec!["arm".to_string()],
            expr: Box::new(clip("raise", 1.0)),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        assert_eq!(joint_translation(&transforms, 0), vec3(0.0, 0.0, 0.0));
        // arm absolute = root(bind, identity) * arm(local y=1)
        assert_eq!(joint_translation(&transforms, 1), vec3(0.0, 1.0, 0.0));
        // hand absolute = arm(1) + hand local(1)
        assert_eq!(joint_translation(&transforms, 2), vec3(0.0, 2.0, 0.0));
    }

    #[test]
    fn masked_blend_leaves_uncovered_joints_to_other_inputs() {
        let model = chain_model();
        // A full-body raise blended with a hand-masked raise at double
        // playhead: hand joints see both inputs, root/arm only the first.
        let expr = AnimExpr::Blend(vec![
            (clip("raise", 1.0), 1.0),
            (
                AnimExpr::Mask {
                    joints: vec!["hand".to_string()],
                    expr: Box::new(clip("raise", 2.0)),
                },
                1.0,
            ),
        ]);
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        // root local: only input 1 → y=1. arm local: y=1. hand local: mix of
        // y=1 and y=0 (playhead 2.0 wraps to 0 on a 2s clip) → 0.5.
        assert_eq!(joint_translation(&transforms, 0), vec3(0.0, 1.0, 0.0));
        assert_eq!(joint_translation(&transforms, 1), vec3(0.0, 2.0, 0.0));
        assert_eq!(joint_translation(&transforms, 2), vec3(0.0, 2.5, 0.0));
    }

    #[test]
    fn rotate_post_multiplies_and_survives_masking() {
        let model = chain_model();
        // Rotate the hand 90° about Z over the rest pose.
        let expr = AnimExpr::Rotate {
            joint: "hand".to_string(),
            euler: [0.0, 0.0, std::f32::consts::FRAC_PI_2],
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        // The hand's rotated X axis points along +Y.
        let x_axis = transforms[2].x.truncate();
        assert!((x_axis.y - 1.0).abs() < 1e-5, "x axis: {x_axis:?}");
        // Root/arm untouched.
        assert_eq!(transforms[0], Matrix4::identity());
    }

    #[test]
    fn rotate_on_unknown_joint_warns_and_is_ignored() {
        let model = chain_model();
        let expr = AnimExpr::Rotate {
            joint: "tail".to_string(),
            euler: [0.0, 0.0, 1.0],
            expr: Box::new(AnimExpr::Rest),
        };
        let mut missing = Vec::new();
        let transforms = skinning_transforms(&model, &expr, &mut |warning| {
            if let AnimWarning::MissingJoint(name) = warning {
                missing.push(name.to_string());
            }
        });
        assert_eq!(missing, vec!["tail".to_string()]);
        assert_eq!(transforms[0], Matrix4::identity());
    }

    #[test]
    fn look_at_aims_after_the_parent_pose_has_settled() {
        let model = chain_model();
        let expr = AnimExpr::LookAt {
            joint: "hand".to_string(),
            target: [0.0, 2.0, 0.0],
            max_angle: std::f32::consts::PI,
            weight: 1.0,
            expr: Box::new(AnimExpr::Rotate {
                joint: "arm".to_string(),
                euler: [0.0, std::f32::consts::FRAC_PI_2, 0.0],
                expr: Box::new(AnimExpr::Rest),
            }),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let forward = transforms[2].z.truncate().normalize();
        assert!(
            (forward - Vector3::unit_y()).magnitude() < 1e-5,
            "forward: {forward:?}"
        );
    }

    #[test]
    fn look_at_clamps_the_correction_and_blends_its_weight() {
        let model = chain_model();
        let expr = AnimExpr::LookAt {
            joint: "hand".to_string(),
            target: [1.0, 0.0, 0.0],
            max_angle: 60.0_f32.to_radians(),
            weight: 0.5,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let forward = transforms[2].z.truncate().normalize();
        let angle = Vector3::unit_z().dot(forward).clamp(-1.0, 1.0).acos();
        assert!(
            (angle - 30.0_f32.to_radians()).abs() < 1e-5,
            "angle: {} degrees",
            angle.to_degrees()
        );
    }

    #[test]
    fn look_at_uses_joint_up_as_the_rear_singularity_pole() {
        let max_angle = 60.0_f32.to_radians();
        let expected = vec3(-max_angle.sin(), 0.0, max_angle.cos());

        // Tiny floating-point jitter around the exact antipode must not choose
        // opposite arbitrary axes and make a clamped head snap side-to-side.
        for x in [-1e-8, 0.0, 1e-8] {
            let target = vec3(x, 0.0, -1.0).normalize();
            let correction = scaled_shortest_arc(
                Vector3::unit_z(),
                target,
                Vector3::unit_y(),
                max_angle,
                1.0,
            );
            let forward = correction.rotate_vector(Vector3::unit_z());
            assert!(
                (forward - expected).magnitude() < 1e-5,
                "target {target:?} produced {forward:?}"
            );
        }
    }

    #[test]
    fn look_at_aims_through_non_uniform_parent_scale() {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 2]);
        builder.add_joint(
            0,
            0,
            "root".to_string(),
            None,
            Matrix4::from_nonuniform_scale(2.0, 1.0, 0.5),
        );
        builder.add_joint(
            1,
            1,
            "head".to_string(),
            Some(0),
            Matrix4::from_translation(Vector3::unit_y()),
        );
        let model = Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        };
        let expr = AnimExpr::LookAt {
            joint: "head".to_string(),
            // The joint is at (0,1,0), so the requested model-space
            // direction is (1,1,0).
            target: [1.0, 2.0, 0.0],
            max_angle: std::f32::consts::PI,
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let forward = transforms[1].z.truncate().normalize();
        let expected = vec3(1.0, 1.0, 0.0).normalize();
        assert!(
            (forward - expected).magnitude() < 1e-5,
            "forward: {forward:?}"
        );
    }

    #[test]
    fn look_at_on_unknown_joint_warns_and_is_ignored() {
        let model = chain_model();
        let expr = AnimExpr::LookAt {
            joint: "tail".to_string(),
            target: [0.0, 0.0, 1.0],
            max_angle: std::f32::consts::FRAC_PI_2,
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let mut missing = Vec::new();
        let transforms = skinning_transforms(&model, &expr, &mut |warning| {
            if let AnimWarning::MissingJoint(name) = warning {
                missing.push(name.to_string());
            }
        });
        assert_eq!(missing, vec!["tail".to_string()]);
        assert_eq!(transforms[0], Matrix4::identity());
    }

    #[test]
    fn reach_places_the_end_joint_at_a_reachable_target() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [0.0, 2.0_f32.sqrt(), 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let root = joint_translation(&transforms, 0);
        let middle = joint_translation(&transforms, 1);
        let end = joint_translation(&transforms, 2);
        let target = vec3(0.0, 2.0_f32.sqrt(), 0.0);
        assert!((end - target).magnitude() < 1e-5, "end: {end:?}");
        assert!(((middle - root).magnitude() - 1.0).abs() < 1e-5);
        assert!(((end - middle).magnitude() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn reach_clamps_an_unreachable_target_to_full_extension() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [5.0, 0.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        assert!((joint_translation(&transforms, 2) - vec3(2.0, 0.0, 0.0)).magnitude() < 1e-5);
    }

    #[test]
    fn reach_clamps_a_large_finite_target_without_overflow() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [0.0, 1e30, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let end = joint_translation(&transforms, 2);
        assert!((end - vec3(0.0, 2.0, 0.0)).magnitude() < 1e-5, "end: {end:?}");
    }

    #[test]
    fn reach_folds_safely_when_the_target_is_at_the_root() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [0.0, 0.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        assert!(transforms.iter().all(|transform| {
            [
                transform.x.x,
                transform.x.y,
                transform.x.z,
                transform.x.w,
                transform.y.x,
                transform.y.y,
                transform.y.z,
                transform.y.w,
                transform.z.x,
                transform.z.y,
                transform.z.z,
                transform.z.w,
                transform.w.x,
                transform.w.y,
                transform.w.z,
                transform.w.w,
            ]
            .into_iter()
            .all(f32::is_finite)
        }));
        assert!(joint_translation(&transforms, 2).magnitude() < 3e-5);
    }

    #[test]
    fn reach_solves_through_non_uniform_parent_scale() {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 4]);
        builder.add_joint(
            0,
            0,
            "scaled-parent".to_string(),
            None,
            Matrix4::from_nonuniform_scale(2.0, 1.0, 0.5),
        );
        builder.add_joint(1, 1, "upper".to_string(), Some(0), Matrix4::identity());
        builder.add_joint(
            2,
            2,
            "lower".to_string(),
            Some(1),
            Matrix4::from_translation(Vector3::unit_x()),
        );
        builder.add_joint(
            3,
            3,
            "end".to_string(),
            Some(2),
            Matrix4::from_translation(Vector3::unit_x()),
        );
        let model = Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        };
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            // In the scaled parent's local solve space this is (1, 1, 0),
            // a reachable target for two unit segments.
            target: [2.0, 1.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let end = joint_translation(&transforms, 3);
        assert!(
            (end - vec3(2.0, 1.0, 0.0)).magnitude() < 1e-5,
            "end: {end:?}"
        );
    }

    #[test]
    fn reach_warns_for_non_uniform_root_scale_and_is_ignored() {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 3]);
        builder.add_joint(
            0,
            0,
            "upper".to_string(),
            None,
            Matrix4::from_nonuniform_scale(2.0, 1.0, 0.5),
        );
        builder.add_joint(
            1,
            1,
            "lower".to_string(),
            Some(0),
            Matrix4::from_translation(Vector3::unit_x()),
        );
        builder.add_joint(
            2,
            2,
            "end".to_string(),
            Some(1),
            Matrix4::from_translation(Vector3::unit_x()),
        );
        let model = Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        };
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [0.0, 2.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let rest = skinning_transforms(&model, &AnimExpr::Rest, &mut no_warning);
        let mut warnings = Vec::new();
        let reached = skinning_transforms(&model, &expr, &mut |warning| {
            if let AnimWarning::NonUniformRootScale { root } = warning {
                warnings.push(root.to_string());
            }
        });
        assert_eq!(warnings, vec!["upper"]);
        assert_eq!(reached, rest);
    }

    #[test]
    fn reach_warns_for_small_non_uniform_root_scale_and_is_ignored() {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 3]);
        builder.add_joint(
            0,
            0,
            "upper".to_string(),
            None,
            Matrix4::from_nonuniform_scale(0.00009, 0.00001, 0.00001),
        );
        builder.add_joint(
            1,
            1,
            "lower".to_string(),
            Some(0),
            Matrix4::from_translation(vec3(10000.0, 0.0, 0.0)),
        );
        builder.add_joint(
            2,
            2,
            "end".to_string(),
            Some(1),
            Matrix4::from_translation(vec3(10000.0, 0.0, 0.0)),
        );
        let model = Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        };
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [0.0, 1.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let rest = skinning_transforms(&model, &AnimExpr::Rest, &mut no_warning);
        let mut warnings = Vec::new();
        let reached = skinning_transforms(&model, &expr, &mut |warning| {
            if let AnimWarning::NonUniformRootScale { root } = warning {
                warnings.push(root.to_string());
            }
        });
        assert_eq!(warnings, vec!["upper"]);
        assert_eq!(reached, rest);
    }

    #[test]
    fn reach_normalizes_large_finite_bone_vectors_without_overflow() {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 3]);
        builder.add_joint(0, 0, "upper".to_string(), None, Matrix4::identity());
        builder.add_joint(
            1,
            1,
            "lower".to_string(),
            Some(0),
            Matrix4::from_translation(vec3(1e30, 1e30, 0.0)),
        );
        builder.add_joint(
            2,
            2,
            "end".to_string(),
            Some(1),
            Matrix4::from_translation(vec3(1e30, 0.0, 0.0)),
        );
        let model = Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        };
        let target = vec3(0.0, 1e30, 0.0);
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: target.into(),
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let end = joint_translation(&transforms, 2);
        assert!(
            length_f64(end - target) / 1e30 < 1e-5,
            "target: {target:?}, end: {end:?}"
        );
    }

    #[test]
    fn reach_preserves_the_evaluated_bend_side() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [1.0, 1.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        let middle = joint_translation(&transforms, 1);
        let end = joint_translation(&transforms, 2);
        // The rest pose's elbow starts on +X, so it remains on that side of
        // the root->target line instead of choosing the mirrored solution.
        assert!(middle.x > middle.y, "middle: {middle:?}");
        assert!((end - vec3(1.0, 1.0, 0.0)).magnitude() < 1e-5);
    }

    #[test]
    fn reach_zero_weight_leaves_the_pose_unchanged() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target: [0.0, 2.0, 0.0],
            weight: 0.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        assert_eq!(joint_translation(&transforms, 1), vec3(1.0, 0.0, 0.0));
        assert_eq!(joint_translation(&transforms, 2), vec3(2.0, 0.0, 0.0));
    }

    #[test]
    fn reach_warns_for_a_non_direct_chain_and_is_ignored() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "end".to_string(),
            end: "lower".to_string(),
            target: [0.0, 1.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let mut invalid = Vec::new();
        let transforms = skinning_transforms(&model, &expr, &mut |warning| {
            if let AnimWarning::InvalidChain { root, middle, end } = warning {
                invalid.push((root.to_string(), middle.to_string(), end.to_string()));
            }
        });
        assert_eq!(
            invalid,
            vec![("upper".to_string(), "end".to_string(), "lower".to_string())]
        );
        assert_eq!(joint_translation(&transforms, 2), vec3(2.0, 0.0, 0.0));
    }

    #[test]
    fn reach_warns_for_each_missing_joint_and_is_ignored() {
        let model = reach_model();
        let expr = AnimExpr::Reach {
            root: "shoulder".to_string(),
            middle: "elbow".to_string(),
            end: "hand".to_string(),
            target: [0.0, 1.0, 0.0],
            weight: 1.0,
            expr: Box::new(AnimExpr::Rest),
        };
        let mut missing = Vec::new();
        let transforms = skinning_transforms(&model, &expr, &mut |warning| {
            if let AnimWarning::MissingJoint(name) = warning {
                missing.push(name.to_string());
            }
        });
        assert_eq!(missing, vec!["shoulder", "elbow", "hand"]);
        assert_eq!(joint_translation(&transforms, 2), vec3(2.0, 0.0, 0.0));
    }

    fn orient(joint: &str, q: Quaternion<f32>, weight: f32, expr: AnimExpr) -> AnimExpr {
        AnimExpr::Orient {
            joint: joint.to_string(),
            rotation: [q.v.x, q.v.y, q.v.z, q.s],
            weight,
            expr: Box::new(expr),
        }
    }

    fn rotate(joint: &str, euler: [f32; 3], expr: AnimExpr) -> AnimExpr {
        AnimExpr::Rotate {
            joint: joint.to_string(),
            euler,
            expr: Box::new(expr),
        }
    }

    /// The angle between two rotations, sign-blind (`q` and `-q` are 0 apart).
    fn angle_between(a: Quaternion<f32>, b: Quaternion<f32>) -> f32 {
        2.0 * a.normalize().dot(b.normalize()).abs().min(1.0).acos()
    }

    fn joint_rotation(transforms: &[Matrix4<f32>], index: usize) -> Quaternion<f32> {
        rotation_of(transforms[index]).expect("a rotation")
    }

    /// `root` with two children: `left` at +X and `right` at -X.
    fn branch_model() -> Model {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 3]);
        builder.add_joint(0, 0, "root".to_string(), None, Matrix4::identity());
        builder.add_joint(
            1,
            1,
            "left".to_string(),
            Some(0),
            Matrix4::from_translation(Vector3::unit_x()),
        );
        builder.add_joint(
            2,
            2,
            "right".to_string(),
            Some(0),
            Matrix4::from_translation(-Vector3::unit_x()),
        );
        Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        }
    }

    fn wrist_target() -> Quaternion<f32> {
        Quaternion::from_angle_x(Deg(30.0)) * Quaternion::from_angle_y(Deg(-50.0))
    }

    fn reach_to(target: [f32; 3], expr: AnimExpr) -> AnimExpr {
        AnimExpr::Reach {
            root: "upper".to_string(),
            middle: "lower".to_string(),
            end: "end".to_string(),
            target,
            weight: 1.0,
            expr: Box::new(expr),
        }
    }

    #[test]
    fn orient_sets_the_model_space_rotation_under_rotated_parents() {
        let model = reach_model();
        let base = rotate(
            "upper",
            [0.3, 0.7, -0.4],
            rotate("lower", [1.1, 0.0, 0.2], AnimExpr::Rest),
        );
        let before = skinning_transforms(&model, &base, &mut no_warning);
        let q = wrist_target();
        let after = skinning_transforms(&model, &orient("end", q, 1.0, base), &mut no_warning);
        assert!(angle_between(joint_rotation(&after, 2), q) < 1e-5);
        // Only the end joint changed, and its origin did not move.
        assert_eq!(&after[..2], &before[..2]);
        assert!((joint_translation(&after, 2) - joint_translation(&before, 2)).magnitude() < 1e-6);
    }

    #[test]
    fn orient_keeps_scale_under_a_scaled_rotated_parent() {
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity(); 2]);
        builder.add_joint(
            0,
            0,
            "shoulder".to_string(),
            None,
            Matrix4::from_angle_z(Deg(40.0)) * Matrix4::from_scale(2.0),
        );
        builder.add_joint(
            1,
            1,
            "wrist".to_string(),
            Some(0),
            Matrix4::from_translation(Vector3::unit_x())
                * Matrix4::from_nonuniform_scale(1.0, 0.5, 1.0),
        );
        let model = Model {
            meshes: vec![],
            skeleton: builder.build(),
            animations: vec![],
        };
        let q = wrist_target();
        let rest = skinning_transforms(&model, &AnimExpr::Rest, &mut no_warning);
        let after = skinning_transforms(
            &model,
            &orient("wrist", q, 1.0, AnimExpr::Rest),
            &mut no_warning,
        );
        assert!(angle_between(joint_rotation(&after, 1), q) < 1e-5);
        // Axis lengths: the parent's scale 2 times the wrist's own (1, 0.5, 1).
        let lengths = [after[1].x, after[1].y, after[1].z].map(|axis| axis.truncate().magnitude());
        for (length, expected) in lengths.into_iter().zip([2.0, 1.0, 2.0]) {
            assert!((length - expected).abs() < 1e-5, "axis lengths {lengths:?}");
        }
        assert!((joint_translation(&after, 1) - joint_translation(&rest, 1)).magnitude() < 1e-6);
    }

    #[test]
    fn orient_rolls_the_wrist_about_its_bone() {
        // The end bone points along +X; a 90° roll about X keeps +X and turns
        // the joint's +Y onto +Z.
        let model = reach_model();
        let expr = orient(
            "end",
            Quaternion::from_angle_x(Deg(90.0)),
            1.0,
            AnimExpr::Rest,
        );
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        assert!((transforms[2].x.truncate() - Vector3::unit_x()).magnitude() < 1e-6);
        assert!((transforms[2].y.truncate() - Vector3::unit_z()).magnitude() < 1e-6);
        assert_eq!(joint_translation(&transforms, 2), vec3(2.0, 0.0, 0.0));
    }

    #[test]
    fn orient_treats_q_and_negated_q_alike() {
        let model = reach_model();
        let base = rotate("lower", [0.0, 0.0, 2.5], AnimExpr::Rest);
        let q = wrist_target();
        for weight in [0.5, 1.0] {
            let positive = skinning_transforms(
                &model,
                &orient("end", q, weight, base.clone()),
                &mut no_warning,
            );
            let negative = skinning_transforms(
                &model,
                &orient("end", -q, weight, base.clone()),
                &mut no_warning,
            );
            assert_eq!(positive, negative, "weight {weight}");
        }
    }

    #[test]
    fn orient_weight_slerps_from_the_evaluated_rotation() {
        let model = reach_model();
        let q = Quaternion::from_angle_y(Deg(80.0));
        let at = |weight| {
            skinning_transforms(
                &model,
                &orient("end", q, weight, AnimExpr::Rest),
                &mut no_warning,
            )
        };
        let rest = skinning_transforms(&model, &AnimExpr::Rest, &mut no_warning);
        assert_eq!(at(0.0), rest);
        let half = joint_rotation(&at(0.5), 2);
        assert!(angle_between(half, Quaternion::from_angle_y(Deg(40.0))) < 1e-5);
        assert!(angle_between(joint_rotation(&at(1.0), 2), q) < 1e-6);
        assert_eq!(at(1.0), at(7.0), "weight clamps to 1");
    }

    #[test]
    fn orient_after_reach_keeps_the_reached_position() {
        let model = reach_model();
        let target = [0.0, 2.0_f32.sqrt(), 0.0];
        let q = wrist_target();
        let after = skinning_transforms(
            &model,
            &orient("end", q, 1.0, reach_to(target, AnimExpr::Rest)),
            &mut no_warning,
        );
        assert!((joint_translation(&after, 2) - Vector3::from(target)).magnitude() < 1e-5);
        assert!(angle_between(joint_rotation(&after, 2), q) < 1e-5);
        // The other order lets reach's chain rotation carry the wrist away.
        let before = skinning_transforms(
            &model,
            &reach_to(target, orient("end", q, 1.0, AnimExpr::Rest)),
            &mut no_warning,
        );
        assert!((joint_translation(&before, 2) - Vector3::from(target)).magnitude() < 1e-5);
        assert!(angle_between(joint_rotation(&before, 2), q) > 0.1);
    }

    #[test]
    fn orient_composes_with_mask_and_blend() {
        let model = branch_model();
        let q = wrist_target();
        // A mask beneath cannot drop the joint orient drives.
        let beneath = orient(
            "right",
            q,
            1.0,
            AnimExpr::Mask {
                joints: vec!["left".to_string()],
                expr: Box::new(AnimExpr::Rest),
            },
        );
        let transforms = skinning_transforms(&model, &beneath, &mut no_warning);
        assert!(angle_between(joint_rotation(&transforms, 2), q) < 1e-6);
        // An enclosing mask that excludes it returns it to bind.
        let enclosing = AnimExpr::Mask {
            joints: vec!["left".to_string()],
            expr: Box::new(orient("right", q, 1.0, AnimExpr::Rest)),
        };
        let transforms = skinning_transforms(&model, &enclosing, &mut no_warning);
        assert!(angle_between(joint_rotation(&transforms, 2), Quaternion::one()) < 1e-6);
        // An even blend with rest lands halfway.
        let blended = AnimExpr::Blend(vec![
            (orient("right", q, 1.0, AnimExpr::Rest), 1.0),
            (AnimExpr::Rest, 1.0),
        ]);
        let transforms = skinning_transforms(&model, &blended, &mut no_warning);
        let half = Quaternion::one().slerp(q, 0.5);
        assert!(angle_between(joint_rotation(&transforms, 2), half) < 1e-5);
    }

    #[test]
    fn orient_on_unknown_joint_warns_and_is_ignored() {
        let model = reach_model();
        let mut missing = Vec::new();
        let expr = orient("wrist", wrist_target(), 1.0, AnimExpr::Rest);
        let transforms = skinning_transforms(&model, &expr, &mut |warning| {
            if let AnimWarning::MissingJoint(name) = warning {
                missing.push(name.to_string());
            }
        });
        assert_eq!(missing, vec!["wrist"]);
        assert_eq!(
            transforms,
            skinning_transforms(&model, &AnimExpr::Rest, &mut no_warning)
        );
    }

    #[test]
    fn orient_normalizes_a_non_unit_quaternion() {
        let model = reach_model();
        let q = wrist_target();
        let unit = skinning_transforms(
            &model,
            &orient("end", q, 1.0, AnimExpr::Rest),
            &mut no_warning,
        );
        for scale in [3.0, 1e-3, 1e30] {
            let scaled = skinning_transforms(
                &model,
                &orient("end", q * scale, 1.0, AnimExpr::Rest),
                &mut no_warning,
            );
            assert!(
                (joint_rotation(&scaled, 2) - joint_rotation(&unit, 2)).magnitude() < 1e-6,
                "scale {scale}"
            );
        }
        // Zero encodes no rotation: ignored (the prelude rejects it).
        let zero = orient("end", Quaternion::zero(), 1.0, AnimExpr::Rest);
        assert_eq!(
            skinning_transforms(&model, &zero, &mut no_warning),
            skinning_transforms(&model, &AnimExpr::Rest, &mut no_warning)
        );
    }

    #[test]
    fn add_layers_a_delta_on_top_of_the_base() {
        // Base: slide at 1s → (1,0,0). Layer: lift at 1s → delta (0,2,0)
        // from bind. Half weight → (1,1,0).
        let model = test_model();
        let expr = AnimExpr::Add {
            base: Box::new(clip("slide", 1.0)),
            layer: Box::new(clip("lift", 1.0)),
            weight: 0.5,
        };
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_warning), 0);
        assert_eq!(t, vec3(1.0, 1.0, 0.0));
    }

    #[test]
    fn add_composes_rotations_as_deltas() {
        // A one-joint model with a rotation clip holding 90° about Y; adding
        // it at full weight onto itself yields 180°.
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity()]);
        builder.add_joint(0, 0, "root".to_string(), None, Matrix4::identity());
        let skeleton = builder.build();
        let model = Model {
            meshes: vec![],
            skeleton,
            animations: vec![Animation {
                name: "quarter".to_string(),
                duration: 1.0,
                channels: vec![AnimationChannel {
                    target_node_index: 0,
                    target_property: AnimationProperty::Rotation,
                    keyframes: vec![Keyframe {
                        time: 0.0,
                        value: AnimationValue::Rotation(Quaternion::from_angle_y(Deg(90.0))),
                    }],
                }],
            }],
        };
        let expr = AnimExpr::Add {
            base: Box::new(clip("quarter", 0.0)),
            layer: Box::new(clip("quarter", 0.0)),
            weight: 1.0,
        };
        let transforms = skinning_transforms(&model, &expr, &mut no_warning);
        // 90° + 90° about Y: the X axis lands on -X.
        let x_axis = transforms[0].x.truncate();
        assert!((x_axis.x + 1.0).abs() < 1e-4, "x axis: {x_axis:?}");
    }

    /// An 80-joint, Mixamo-sized rig: a depth-7 binary tree of skin joints
    /// `j0..j79` under a non-skinning `Armature` ancestor (cm→m scale), each
    /// bind a translate + rotate. Two 1 s, 30 fps clips (`walk`, `run`)
    /// rotate every joint and translate the root.
    fn rig_model() -> Model {
        use crate::model::{build_skeleton_from_skin, HierarchyNode};
        const JOINTS: usize = 80;
        let mut hierarchy = std::collections::HashMap::new();
        hierarchy.insert(
            0,
            HierarchyNode {
                parent: None,
                name: "Armature".to_string(),
                transform: Matrix4::from_scale(0.01) * Matrix4::from_angle_x(Deg(-90.0)),
            },
        );
        for i in 0..JOINTS {
            let parent = if i == 0 { 0 } else { (i - 1) / 2 + 1 };
            hierarchy.insert(
                i + 1,
                HierarchyNode {
                    parent: Some(parent),
                    name: format!("j{i}"),
                    transform: Matrix4::from_translation(vec3(0.0, 10.0, 1.0))
                        * Matrix4::from_angle_z(Deg(std::hint::black_box(5.0 + i as f32))),
                },
            );
        }
        let inverse_binds = (0..JOINTS)
            .map(|i| Matrix4::from_translation(vec3(0.0, -(i as f32), 0.0)))
            .collect();
        let nodes: Vec<usize> = (1..=JOINTS).collect();
        let skeleton = build_skeleton_from_skin(inverse_binds, &nodes, &hierarchy);
        let rig_clip = |name: &str, speed: f32| {
            let mut channels: Vec<AnimationChannel> = nodes
                .iter()
                .map(|&node| AnimationChannel {
                    target_node_index: node,
                    target_property: AnimationProperty::Rotation,
                    keyframes: (0..=30)
                        .map(|k| {
                            let phase = std::hint::black_box(k as f32 * speed + node as f32);
                            let axis = vec3(phase.sin(), phase.cos(), 0.5).normalize();
                            Keyframe {
                                time: k as f32 / 30.0,
                                value: AnimationValue::Rotation(Quaternion::from_axis_angle(
                                    axis,
                                    Rad(0.5 * (phase * 0.7).sin()),
                                )),
                            }
                        })
                        .collect(),
                })
                .collect();
            channels.push(AnimationChannel {
                target_node_index: 1,
                target_property: AnimationProperty::Translation,
                keyframes: (0..=30)
                    .map(|k| Keyframe {
                        time: k as f32 / 30.0,
                        value: AnimationValue::Translation(vec3(
                            0.0,
                            100.0 + std::hint::black_box(k as f32 * speed).sin() * 5.0,
                            k as f32 * speed,
                        )),
                    })
                    .collect(),
            });
            Animation {
                name: name.to_string(),
                duration: 1.0,
                channels,
            }
        };
        Model {
            meshes: vec![],
            skeleton,
            animations: vec![rig_clip("walk", 0.2), rig_clip("run", 0.35)],
        }
    }

    fn rig_blend(playhead: f32, run: f32) -> AnimExpr {
        AnimExpr::Blend(vec![
            (clip("walk", playhead), 1.0 - run),
            (clip("run", playhead * 1.3), run),
        ])
    }

    /// Every node kind over multi-joint chains on the 80-joint rig, pinned to
    /// recorded fingerprints so an evaluator change that drifts numerically
    /// fails.
    #[test]
    fn rig_poses_match_the_recorded_evaluator() {
        let model = rig_model();
        let boxed = Box::new;
        let names = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        let exprs = vec![
            AnimExpr::Rest,
            clip("walk", 0.37),
            clip("run", -2.71),
            clip("walk", 1.0 / 30.0),
            rig_blend(0.52, 0.3),
            AnimExpr::Add {
                base: boxed(clip("walk", 0.2)),
                layer: boxed(clip("run", 0.9)),
                weight: 0.4,
            },
            AnimExpr::Blend(vec![
                (clip("walk", 0.6), 0.5),
                (
                    AnimExpr::Mask {
                        joints: names(&["j3"]),
                        expr: boxed(clip("run", 0.1)),
                    },
                    0.5,
                ),
            ]),
            AnimExpr::Mask {
                joints: names(&["j1", "j6"]),
                expr: boxed(AnimExpr::Add {
                    base: boxed(rig_blend(0.15, 0.6)),
                    layer: boxed(AnimExpr::Mask {
                        joints: names(&["j4"]),
                        expr: boxed(clip("run", 0.45)),
                    }),
                    weight: 0.7,
                }),
            },
            AnimExpr::Blend(vec![
                (
                    AnimExpr::Mask {
                        joints: names(&["j2"]),
                        expr: boxed(AnimExpr::Mask {
                            joints: names(&["j5"]),
                            expr: boxed(clip("walk", 0.8)),
                        }),
                    },
                    0.5,
                ),
                (
                    AnimExpr::Mask {
                        joints: names(&["j5"]),
                        expr: boxed(clip("run", 0.3)),
                    },
                    0.25,
                ),
            ]),
            AnimExpr::Rotate {
                joint: "j10".to_string(),
                euler: [0.3, -0.2, 0.9],
                expr: boxed(AnimExpr::Mask {
                    joints: names(&["j1"]),
                    expr: boxed(clip("walk", 0.25)),
                }),
            },
            AnimExpr::Rotate {
                joint: "j9".to_string(),
                euler: [0.0, 0.0, 1.2],
                expr: boxed(AnimExpr::Blend(vec![(clip("walk", 0.5), 0.0)])),
            },
            AnimExpr::LookAt {
                joint: "j19".to_string(),
                target: [0.3, 0.5, 2.0],
                max_angle: 1.0,
                weight: 0.8,
                expr: boxed(rig_blend(0.33, 0.5)),
            },
            AnimExpr::Reach {
                root: "j1".to_string(),
                middle: "j3".to_string(),
                end: "j7".to_string(),
                target: [0.2, 0.1, 0.9],
                weight: 0.9,
                expr: boxed(rig_blend(0.71, 0.4)),
            },
            AnimExpr::LookAt {
                joint: "j8".to_string(),
                target: [-1.0, 0.2, 0.4],
                max_angle: 0.5,
                weight: 1.0,
                expr: boxed(AnimExpr::Reach {
                    root: "j0".to_string(),
                    middle: "j2".to_string(),
                    end: "j6".to_string(),
                    target: [0.0, 0.3, 0.3],
                    weight: 0.5,
                    expr: boxed(AnimExpr::Mask {
                        joints: names(&["j2"]),
                        expr: boxed(clip("run", 0.05)),
                    }),
                }),
            },
            AnimExpr::Orient {
                joint: "j7".to_string(),
                rotation: [0.3, -0.5, 0.2, 0.8],
                weight: 0.7,
                expr: boxed(AnimExpr::Reach {
                    root: "j1".to_string(),
                    middle: "j3".to_string(),
                    end: "j7".to_string(),
                    target: [0.2, 0.1, 0.9],
                    weight: 1.0,
                    expr: boxed(rig_blend(0.71, 0.4)),
                }),
            },
        ];
        let mut outputs: Vec<Vec<Matrix4<f32>>> = exprs
            .iter()
            .map(|expr| skinning_transforms(&model, expr, &mut no_warning))
            .collect();
        // The zero-config default path (no expression) and the load-time
        // absolutes go through the skeleton directly.
        let skeleton = &model.skeleton;
        let default_pose = skeleton.sample_pose(&model.animations[1], 0.61);
        outputs.push(skeleton.skinning_transforms_for(&default_pose));
        outputs.push(skeleton.get_skinning_transforms());
        // A position-weighted sum per output: any joint or element moving
        // shifts it, while libm ulp differences across platforms stay far
        // inside the tolerance.
        let fingerprints: Vec<f64> = outputs
            .iter()
            .map(|transforms| {
                assert_eq!(transforms.len(), 80);
                let mut sum = 0.0;
                for (j, m) in transforms.iter().enumerate() {
                    let floats: &[f32; 16] = m.as_ref();
                    for (k, f) in floats.iter().enumerate() {
                        sum += f64::from(*f) * (1 + (j * 16 + k) % 7) as f64;
                    }
                }
                sum
            })
            .collect();
        let expected: [f64; 17] = [
            138.222271,
            -23.687245,
            -3.746612,
            5.565270,
            0.013203,
            -242.378618,
            16.628422,
            235.866932,
            158.385884,
            239.478895,
            130.841586,
            8.291764,
            17.016888,
            384.925620,
            36.829165,
            -1.762920,
            138.222228,
        ];
        for (i, (got, want)) in fingerprints.iter().zip(expected).enumerate() {
            assert!((got - want).abs() < 1e-3, "output {i}: {got} vs {want}");
        }
    }

    /// The render path (`skinning_transforms_for`) and the skeleton-rebuild
    /// path (`apply_pose`, behind `Skeleton::animate` and `functor inspect`)
    /// agree exactly, including at exact key times and past the clip's end.
    #[test]
    fn skinning_paths_agree_bit_for_bit() {
        let model = rig_model();
        let skeleton = &model.skeleton;
        for animation in &model.animations {
            for time in [0.0, 1.0 / 30.0, 0.37, 0.5, 1.0, 1.5] {
                let pose = skeleton.sample_pose(animation, time);
                let fast = skeleton.skinning_transforms_for(&pose);
                let rebuilt = skeleton.apply_pose(&pose).get_skinning_transforms();
                assert_eq!(fast, rebuilt, "{} at {time}", animation.name);
            }
        }
    }

    /// `cargo test --release -p functor_runtime_common rig_blend_eval_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn rig_blend_eval_bench() {
        let model = rig_model();
        let iterations = 20_000;
        let start = std::time::Instant::now();
        let mut sink = 0.0;
        for i in 0..iterations {
            let t = i as f32 * 0.0137;
            let transforms = skinning_transforms(&model, &rig_blend(t, 0.4), &mut no_warning);
            sink += transforms[79].w.x;
        }
        let micros = start.elapsed().as_secs_f64() * 1e6 / iterations as f64;
        println!("80-joint 2-clip blend: {micros:.2} µs/eval (sink {sink})");
    }
}
