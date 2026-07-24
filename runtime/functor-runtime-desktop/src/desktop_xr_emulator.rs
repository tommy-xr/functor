//! Opt-in mouse/keyboard adapter for exercising XR input on desktop.
//!
//! This is deliberately a shell adapter over the canonical
//! [`functor_runtime_common::InputSnapshot`] protocol. Games consume the same
//! `Input.snapshot` record as Quest; a future gamepad or mobile adapter can
//! synthesize that record without changing Functor Lang.

use std::collections::BTreeSet;

use functor_runtime_common::{Key, TrackingPose, XrControllerSnapshot, XrInputSnapshot};

const REFERENCE_SURFACE: (i32, i32) = (800, 600);

pub fn reference_surface() -> (i32, i32) {
    REFERENCE_SURFACE
}

pub fn centered_pointer(surface_size: (i32, i32)) -> (i32, i32) {
    (surface_size.0 / 2, surface_size.1 / 2)
}

pub fn update_right_controls(
    controller: &mut XrControllerSnapshot,
    held_keys: &BTreeSet<Key>,
    primary_down: bool,
) {
    let held = |key| held_keys.contains(&key);
    let axis = |positive, negative| match (held(positive), held(negative)) {
        (true, false) => 1.0,
        (false, true) => -1.0,
        _ => 0.0,
    };

    controller.trigger = if primary_down || held(Key::Space) {
        1.0
    } else {
        0.0
    };
    controller.squeeze = if held(Key::Enter) { 1.0 } else { 0.0 };
    controller.thumbstick = [axis(Key::Right, Key::Left), axis(Key::Up, Key::Down)];
    controller.primary_pressed = held(Key::Num1);
    controller.secondary_pressed = held(Key::Num2);
    controller.thumbstick_pressed = held(Key::Num3);
}

/// Synthesize one rig-local XR sample.
///
/// The mouse moves the right controller over a plane in front of the head.
/// Space/left-click = trigger, Enter = squeeze, arrows = thumbstick,
/// 1/2 = primary/secondary, and 3 = thumbstick click.
pub fn sample(
    held_keys: &BTreeSet<Key>,
    mouse_pos: (i32, i32),
    primary_down: bool,
    surface_size: (i32, i32),
) -> XrInputSnapshot {
    let width = surface_size.0.max(1) as f32;
    let height = surface_size.1.max(1) as f32;
    let normalized_x = ((mouse_pos.0 as f32 - width * 0.5) / (width * 0.5)).clamp(-1.0, 1.0);
    let normalized_y = ((height * 0.5 - mouse_pos.1 as f32) / (height * 0.5)).clamp(-1.0, 1.0);
    let pose = |position| Some(TrackingPose::new(position, [0.0, 0.0, 0.0, 1.0]));
    let left_pose = pose([-0.24, -0.12, -0.55]);
    let right_pose = pose([
        0.24 + normalized_x * 0.28,
        -0.12 + normalized_y * 0.22,
        -0.55,
    ]);
    let mut right = XrControllerSnapshot {
        active: true,
        grip: right_pose,
        aim: right_pose,
        ..XrControllerSnapshot::default()
    };
    update_right_controls(&mut right, held_keys, primary_down);

    XrInputSnapshot {
        head: Some(TrackingPose::IDENTITY),
        left: XrControllerSnapshot {
            active: true,
            grip: left_pose,
            aim: left_pose,
            ..XrControllerSnapshot::default()
        },
        right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_pointer_produces_a_stable_two_controller_rig() {
        let surface = reference_surface();
        let snapshot = sample(&BTreeSet::new(), centered_pointer(surface), false, surface);
        assert_eq!(snapshot.head, Some(TrackingPose::IDENTITY));
        assert!(snapshot.left.active);
        assert!(snapshot.right.active);
        assert_eq!(
            snapshot.left.grip.expect("left grip").position,
            [-0.24, -0.12, -0.55]
        );
        assert_eq!(
            snapshot.right.aim.expect("right aim").position,
            [0.24, -0.12, -0.55]
        );

        let resized = (1600, 900);
        assert_eq!(
            sample(&BTreeSet::new(), centered_pointer(resized), false, resized,)
                .right
                .aim
                .expect("resized right aim")
                .position,
            [0.24, -0.12, -0.55]
        );
    }

    #[test]
    fn keys_and_mouse_button_map_to_controller_levels() {
        let held_keys = BTreeSet::from([
            Key::Space,
            Key::Enter,
            Key::Right,
            Key::Up,
            Key::Num1,
            Key::Num2,
            Key::Num3,
        ]);
        let surface = reference_surface();
        let snapshot = sample(&held_keys, centered_pointer(surface), false, surface);
        assert_eq!(snapshot.right.trigger, 1.0);
        assert_eq!(snapshot.right.squeeze, 1.0);
        assert_eq!(snapshot.right.thumbstick, [1.0, 1.0]);
        assert!(snapshot.right.primary_pressed);
        assert!(snapshot.right.secondary_pressed);
        assert!(snapshot.right.thumbstick_pressed);

        let mouse_trigger = sample(&BTreeSet::new(), centered_pointer(surface), true, surface);
        assert_eq!(mouse_trigger.right.trigger, 1.0);

        let opposing = BTreeSet::from([Key::Left, Key::Right, Key::Up, Key::Down]);
        assert_eq!(
            sample(&opposing, centered_pointer(surface), false, surface)
                .right
                .thumbstick,
            [0.0, 0.0]
        );
    }

    #[test]
    fn pointer_motion_is_clamped_to_the_emulation_plane() {
        let snapshot = sample(
            &BTreeSet::new(),
            (i32::MAX, i32::MIN),
            false,
            reference_surface(),
        );
        let [x, y, z] = snapshot.right.grip.expect("right grip").position;
        assert!((x - 0.52).abs() < f32::EPSILON);
        assert!((y - 0.10).abs() < f32::EPSILON);
        assert!((z + 0.55).abs() < f32::EPSILON);
    }
}
