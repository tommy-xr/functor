//! Declarative animation expressions — the pose algebra a scene `Model` node
//! carries (`ModelDescription::animation`).
//!
//! An [`AnimExpr`] is plain serializable data: the engine never owns a
//! playhead or a blend weight. Game code derives them (typically from `tts`
//! and model state) and bakes them into the expression each frame; evaluation
//! here is a pure function of `(expression, loaded model)`, so a frame
//! re-rendered from time-travel history reproduces the exact pose.
//!
//! `Clip` samples a named glTF clip at a playhead in seconds (looping by the
//! clip's duration). `Blend` mixes sub-poses by normalized weights — the
//! Bevy-RFC-51-shaped primitive that locomotion blending, and later additive
//! layers and per-joint overrides, build on.

use cgmath::Matrix4;
use serde::{Deserialize, Serialize};

use crate::model::{Model, Pose};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnimExpr {
    /// Sample the clip named `name` at `playhead` seconds. The playhead wraps
    /// by the clip's duration (negative playheads wrap backwards from the
    /// end), so a monotone `tts` plays a loop.
    Clip { name: String, playhead: f32 },
    /// Weighted mix of sub-expressions. Non-positive weights are skipped;
    /// the rest are normalized. An empty (or all-skipped) blend is the bind
    /// pose.
    Blend(Vec<(AnimExpr, f32)>),
}

/// Evaluate an expression against a loaded model and return the skinning
/// matrices (`absolute * inverse_bind` per joint) the skinned materials
/// consume. `on_missing_clip` fires for every clip name the model does not
/// contain (each frame — the caller dedups, e.g. `SceneContext::warn_once`);
/// a missing clip contributes the bind pose.
pub fn skinning_transforms(
    model: &Model,
    expr: &AnimExpr,
    on_missing_clip: &mut dyn FnMut(&str),
) -> Vec<Matrix4<f32>> {
    let pose = eval_pose(model, expr, on_missing_clip);
    model.skeleton.apply_pose(&pose).get_skinning_transforms()
}

fn eval_pose(model: &Model, expr: &AnimExpr, on_missing_clip: &mut dyn FnMut(&str)) -> Pose {
    match expr {
        AnimExpr::Clip { name, playhead } => {
            match model.animations.iter().find(|a| a.name == *name) {
                Some(animation) => {
                    let time = if animation.duration > 0.0 {
                        playhead.rem_euclid(animation.duration)
                    } else {
                        0.0
                    };
                    model.skeleton.sample_pose(animation, time)
                }
                None => {
                    on_missing_clip(name);
                    model.skeleton.base_pose()
                }
            }
        }
        AnimExpr::Blend(items) => {
            let inputs: Vec<(Pose, f32)> = items
                .iter()
                .filter(|(_, weight)| *weight > 0.0)
                .map(|(sub, weight)| (eval_pose(model, sub, on_missing_clip), *weight))
                .collect();
            match inputs.is_empty() {
                true => model.skeleton.base_pose(),
                false => Pose::blend(&inputs),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Animation, AnimationChannel, AnimationProperty, AnimationValue, Keyframe};
    use crate::model::SkeletonBuilder;
    use cgmath::{vec3, Deg, Quaternion, Rotation3, SquareMatrix, Vector3};

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

    fn joint_translation(transforms: &[Matrix4<f32>]) -> Vector3<f32> {
        transforms[0].w.truncate()
    }

    fn no_missing(name: &str) {
        panic!("unexpected missing clip: {name}");
    }

    #[test]
    fn clip_samples_at_playhead_and_loops() {
        let model = test_model();
        let clip = |playhead: f32| AnimExpr::Clip {
            name: "slide".to_string(),
            playhead,
        };
        let at = |playhead: f32| {
            joint_translation(&skinning_transforms(&model, &clip(playhead), &mut no_missing))
        };
        assert_eq!(at(0.0), vec3(0.0, 0.0, 0.0));
        assert_eq!(at(1.0), vec3(1.0, 0.0, 0.0));
        // Wraps: 3s into a 2s clip is 1s in; negative playheads wrap backwards.
        assert_eq!(at(3.0), vec3(1.0, 0.0, 0.0));
        assert_eq!(at(-0.5), vec3(1.5, 0.0, 0.0));
    }

    #[test]
    fn blend_mixes_by_normalized_weight() {
        let model = test_model();
        let expr = AnimExpr::Blend(vec![
            (
                AnimExpr::Clip {
                    name: "slide".to_string(),
                    playhead: 1.0,
                },
                1.0,
            ),
            (
                AnimExpr::Clip {
                    name: "lift".to_string(),
                    playhead: 1.0,
                },
                3.0,
            ),
        ]);
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_missing));
        // At 1s, slide is at (1,0,0) and lift at (0,2,0); weights 1:3
        // normalize to 0.25/0.75.
        assert_eq!(t, vec3(0.25, 1.5, 0.0));
    }

    #[test]
    fn blend_skips_non_positive_weights_and_empty_is_bind_pose() {
        let model = test_model();
        let slide = AnimExpr::Clip {
            name: "slide".to_string(),
            playhead: 1.0,
        };
        let expr = AnimExpr::Blend(vec![(slide.clone(), 1.0), (slide.clone(), 0.0)]);
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_missing));
        assert_eq!(t, vec3(1.0, 0.0, 0.0));

        let empty = AnimExpr::Blend(vec![(slide, 0.0)]);
        let t = joint_translation(&skinning_transforms(&model, &empty, &mut no_missing));
        assert_eq!(t, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn blend_with_overflowing_weights_stays_finite() {
        // Weights that overflow the f32 total must not become NaN skinning
        // matrices — the degenerate joint falls back to the first pose.
        let model = test_model();
        let expr = AnimExpr::Blend(vec![
            (
                AnimExpr::Clip {
                    name: "slide".to_string(),
                    playhead: 1.0,
                },
                f32::MAX,
            ),
            (
                AnimExpr::Clip {
                    name: "lift".to_string(),
                    playhead: 1.0,
                },
                f32::MAX,
            ),
        ]);
        let t = joint_translation(&skinning_transforms(&model, &expr, &mut no_missing));
        // Falls back to the first input ("slide" at 1s).
        assert_eq!(t, vec3(1.0, 0.0, 0.0));
    }

    #[test]
    fn missing_clip_reports_and_falls_back_to_bind_pose() {
        let model = test_model();
        let expr = AnimExpr::Clip {
            name: "nope".to_string(),
            playhead: 0.0,
        };
        let mut missing = Vec::new();
        let transforms = skinning_transforms(&model, &expr, &mut |name| {
            missing.push(name.to_string())
        });
        assert_eq!(missing, vec!["nope".to_string()]);
        assert_eq!(joint_translation(&transforms), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn blend_of_rotations_slerps_between_hemispheres() {
        // A rotation-channel model: two clips holding 0° and 90° about Y.
        let mut builder = SkeletonBuilder::create(vec![Matrix4::identity()]);
        builder.add_joint(0, 0, "root".to_string(), None, Matrix4::identity());
        let skeleton = builder.build();
        let rotation_clip = |name: &str, deg: f32| Animation {
            name: name.to_string(),
            duration: 1.0,
            channels: vec![AnimationChannel {
                target_node_index: 0,
                target_property: AnimationProperty::Rotation,
                keyframes: vec![Keyframe {
                    time: 0.0,
                    value: AnimationValue::Rotation(Quaternion::from_angle_y(Deg(deg))),
                }],
            }],
        };
        let model = Model {
            meshes: vec![],
            skeleton,
            animations: vec![rotation_clip("zero", 0.0), rotation_clip("quarter", 90.0)],
        };
        let expr = AnimExpr::Blend(vec![
            (
                AnimExpr::Clip {
                    name: "zero".to_string(),
                    playhead: 0.0,
                },
                1.0,
            ),
            (
                AnimExpr::Clip {
                    name: "quarter".to_string(),
                    playhead: 0.0,
                },
                1.0,
            ),
        ]);
        let transforms = skinning_transforms(&model, &expr, &mut no_missing);
        // An equal mix of 0° and 90° about Y is 45°: the rotated X axis lands
        // at (cos45, 0, -sin45).
        let x_axis = transforms[0].x.truncate();
        let expected = 45.0_f32.to_radians();
        assert!((x_axis.x - expected.cos()).abs() < 1e-4, "x: {x_axis:?}");
        assert!((x_axis.z + expected.sin()).abs() < 1e-4, "x: {x_axis:?}");
    }
}
