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
//!   the programmatic per-joint control (head aim, finger curl).
//!
//! Evaluation carries a per-joint influence weight so `Mask` composes
//! through `Blend`/`Add`: a joint no input drives falls back to the bind
//! pose, and partial influence blends toward it.

use std::collections::HashMap;

use cgmath::{Euler, InnerSpace, Matrix4, Quaternion, Rad, Rotation, Vector3, Zero};
use serde::{Deserialize, Serialize};

use crate::model::{JointPose, Model, Pose, Skeleton};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// A missing reference discovered during evaluation — the caller surfaces
/// these (deduped) to the developer; evaluation falls back to the bind pose
/// (missing clip) or ignores the node (missing joint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimWarning<'a> {
    MissingClip(&'a str),
    MissingJoint(&'a str),
}

/// A pose with a per-joint influence weight in `[0, 1]` — how `Mask`
/// composes through the tree. `finalize` folds it back onto the bind pose.
struct WeightedPose {
    joints: HashMap<i32, (JointPose, f32)>,
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
    let weighted = eval(model, expr, on_warning);
    let pose = finalize(&model.skeleton, weighted);
    model.skeleton.apply_pose(&pose).get_skinning_transforms()
}

/// Fold a weighted pose onto the bind pose: fully-driven joints take the
/// evaluated pose, undriven joints stay at bind, partial influence nlerps
/// between the two.
fn finalize(skeleton: &Skeleton, weighted: WeightedPose) -> Pose {
    let mut pose = skeleton.base_pose();
    for (joint_id, (jp, weight)) in weighted.joints {
        let Some(bind) = pose.joints.get_mut(&joint_id) else {
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

fn eval(model: &Model, expr: &AnimExpr, on_warning: &mut dyn FnMut(AnimWarning)) -> WeightedPose {
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
            blend(&inputs)
        }
        AnimExpr::Add {
            base,
            layer,
            weight,
        } => {
            let mut base = eval(model, base, on_warning);
            if *weight > 0.0 {
                let layer = eval(model, layer, on_warning);
                let bind = model.skeleton.base_pose();
                add_layer(&mut base, &layer, &bind, *weight);
            }
            base
        }
        AnimExpr::Mask { joints, expr } => {
            let mut inner = eval(model, expr, on_warning);
            let kept = model.skeleton.subtree_joint_ids(joints);
            for name in joints {
                if model.skeleton.joint_id_by_name(name).is_none() {
                    on_warning(AnimWarning::MissingJoint(name));
                }
            }
            for (joint_id, (_, weight)) in inner.joints.iter_mut() {
                if !kept.contains(joint_id) {
                    *weight = 0.0;
                }
            }
            inner
        }
        AnimExpr::Rotate { joint, euler, expr } => {
            let mut inner = eval(model, expr, on_warning);
            match model.skeleton.joint_id_by_name(joint) {
                Some(joint_id) => {
                    // The inner expression may not have produced this joint
                    // (an all-zero-weight blend evaluates empty): rotate over
                    // the bind pose so the joint never silently snaps.
                    if !inner.joints.contains_key(&joint_id) {
                        if let Some(jp) = model.skeleton.base_pose().joints.get(&joint_id) {
                            inner.joints.insert(joint_id, (*jp, 0.0));
                        }
                    }
                    if let Some((jp, weight)) = inner.joints.get_mut(&joint_id) {
                        let q = Quaternion::from(Euler::new(
                            Rad(euler[0]),
                            Rad(euler[1]),
                            Rad(euler[2]),
                        ));
                        jp.rotation = (jp.rotation * q).normalize();
                        // The joint is explicitly driven — the rotation
                        // survives a mask/blend BENEATH this node (an
                        // enclosing Mask that excludes the joint still
                        // zeroes it, deliberately).
                        *weight = 1.0;
                    }
                }
                None => on_warning(AnimWarning::MissingJoint(joint)),
            }
            inner
        }
    }
}

fn full_weight(pose: Pose) -> WeightedPose {
    WeightedPose {
        joints: pose
            .joints
            .into_iter()
            .map(|(joint_id, jp)| (joint_id, (jp, 1.0)))
            .collect(),
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
fn blend(inputs: &[(WeightedPose, f32)]) -> WeightedPose {
    let Some((first, _)) = inputs.first() else {
        return WeightedPose {
            joints: HashMap::new(),
        };
    };
    let mut joints = HashMap::new();
    for &joint_id in inputs
        .iter()
        .flat_map(|(input, _)| input.joints.keys())
        .collect::<std::collections::HashSet<_>>()
    {
        let reference = first
            .joints
            .get(&joint_id)
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
            let Some((jp, joint_weight)) = input.joints.get(&joint_id) else {
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
            joints.insert(joint_id, (blended, (total / covering).min(1.0)));
        } else if let Some(jp) = fallback {
            joints.insert(joint_id, (jp, 0.0));
        }
    }
    WeightedPose { joints }
}

/// Apply an additive layer: per joint, `base + weight * (layer - bind)`,
/// scaled additionally by the layer's own joint weight (so a masked layer
/// only adds where it covers). Rotations compose as
/// `base_r * slerp(identity, bind_r⁻¹ * layer_r, w)`.
fn add_layer(base: &mut WeightedPose, layer: &WeightedPose, bind: &Pose, weight: f32) {
    for (joint_id, (base_jp, _)) in base.joints.iter_mut() {
        let Some((layer_jp, layer_weight)) = layer.joints.get(joint_id) else {
            continue;
        };
        let Some(bind_jp) = bind.joints.get(joint_id) else {
            continue;
        };
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
}
