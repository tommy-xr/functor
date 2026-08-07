//! Poll the first connected standard-mapping gamepad into the shared
//! [`GamepadSnapshot`] domain.
//!
//! GLFW's `glfwGetGamepadState` is already the SDL-mapping-normalized view, so
//! the field set matches the snapshot one-to-one; the two conversions this
//! adapter owns are the ones the snapshot contract pins: stick Y is negated
//! (GLFW reports down-positive; the domain is up-positive, matching the XR
//! thumbstick), and triggers are normalized from GLFW's `-1..1` (rest = `-1`)
//! to `0..1`. Values stay raw otherwise — no deadzone shaping.
//!
//! "Primary pad" is re-resolved every scan as the lowest-id mapped joystick,
//! so if two pads are connected and the first disconnects, the game continues
//! on the second with no discontinuity signal — a deliberate consequence of
//! the single-pad contract, not an accident.

use functor_runtime_common::GamepadSnapshot;

/// Raw GLFW gamepad axes in `glfwGetGamepadState` order:
/// left X/Y, right X/Y, left/right trigger.
pub type RawAxes = [f32; 6];

/// Raw GLFW gamepad buttons in `glfwGetGamepadState` order: A, B, X, Y,
/// left/right bumper, back, start, guide, left/right thumb,
/// dpad up/right/down/left.
pub type RawButtons = [bool; 15];

/// Map one raw GLFW gamepad reading into the target-neutral snapshot.
///
/// Pure — the unit tests drive the axis conventions through this seam
/// directly. The guide button has no snapshot field and is dropped.
///
/// Trigger caveat: `glfwGetGamepadState` zero-initializes its axis array and
/// only writes axes the pad's SDL mapping actually binds, so a pad WITHOUT
/// analog triggers leaves them at `0.0` — which naive `-1..1 → 0..1`
/// normalization would report as permanently half-pulled. An exact `0.0` is
/// therefore treated as rest (a real mid-pull landing on exactly `0.0` is a
/// measure-zero coincidence costing at most one frame).
pub fn map_gamepad(axes: RawAxes, buttons: RawButtons) -> GamepadSnapshot {
    let stick = |x: f32, y: f32| [x.clamp(-1.0, 1.0), (-y).clamp(-1.0, 1.0)];
    let trigger = |v: f32| {
        if v == 0.0 {
            0.0
        } else {
            ((v + 1.0) / 2.0).clamp(0.0, 1.0)
        }
    };
    GamepadSnapshot {
        left_stick: stick(axes[0], axes[1]),
        right_stick: stick(axes[2], axes[3]),
        left_trigger: trigger(axes[4]),
        right_trigger: trigger(axes[5]),
        south: buttons[0],
        east: buttons[1],
        west: buttons[2],
        north: buttons[3],
        left_bumper: buttons[4],
        right_bumper: buttons[5],
        select: buttons[6],
        start: buttons[7],
        left_stick_pressed: buttons[9],
        right_stick_pressed: buttons[10],
        dpad_up: buttons[11],
        dpad_right: buttons[12],
        dpad_down: buttons[13],
        dpad_left: buttons[14],
    }
}

/// Poll the first present joystick that has a standard gamepad mapping.
///
/// `None` when no pad is connected (or none has a mapping) — the capability
/// signal the snapshot contract requires, never a zeroed record.
pub fn sample(glfw: &glfw::Glfw) -> Option<GamepadSnapshot> {
    use glfw::{Action, GamepadAxis, GamepadButton, JoystickId};
    const AXES: [GamepadAxis; 6] = [
        GamepadAxis::AxisLeftX,
        GamepadAxis::AxisLeftY,
        GamepadAxis::AxisRightX,
        GamepadAxis::AxisRightY,
        GamepadAxis::AxisLeftTrigger,
        GamepadAxis::AxisRightTrigger,
    ];
    const BUTTONS: [GamepadButton; 15] = [
        GamepadButton::ButtonA,
        GamepadButton::ButtonB,
        GamepadButton::ButtonX,
        GamepadButton::ButtonY,
        GamepadButton::ButtonLeftBumper,
        GamepadButton::ButtonRightBumper,
        GamepadButton::ButtonBack,
        GamepadButton::ButtonStart,
        GamepadButton::ButtonGuide,
        GamepadButton::ButtonLeftThumb,
        GamepadButton::ButtonRightThumb,
        GamepadButton::ButtonDpadUp,
        GamepadButton::ButtonDpadRight,
        GamepadButton::ButtonDpadDown,
        GamepadButton::ButtonDpadLeft,
    ];
    // `get_gamepad_state` itself answers `None` for an absent, mapless, or
    // mid-scan-disconnected joystick, so no presence pre-checks are needed —
    // one platform poll per candidate instead of three.
    (0..16).filter_map(JoystickId::from_i32).find_map(|id| {
        let state = glfw.get_joystick(id).get_gamepad_state()?;
        let axes = AXES.map(|axis| state.get_axis(axis));
        let buttons = BUTTONS.map(|button| state.get_button_state(button) == Action::Press);
        Some(map_gamepad(axes, buttons))
    })
}

#[cfg(test)]
mod tests {
    use super::map_gamepad;

    #[test]
    fn sticks_negate_y_and_triggers_normalize_to_unit() {
        // GLFW: stick pushed UP reads negative Y; the domain is up-positive.
        let pad = map_gamepad([0.25, -1.0, -0.5, 1.0, -1.0, 1.0], [false; 15]);
        assert_eq!(pad.left_stick, [0.25, 1.0]);
        assert_eq!(pad.right_stick, [-0.5, -1.0]);
        // GLFW triggers rest at -1.0 and saturate at 1.0.
        assert_eq!(pad.left_trigger, 0.0);
        assert_eq!(pad.right_trigger, 1.0);
        // An exact 0.0 is an UNMAPPED trigger axis (GLFW zero-initializes and
        // only writes mapped axes), so it must read as rest — naive midpoint
        // normalization would report a pad without analog triggers as
        // permanently half-pulled.
        assert_eq!(map_gamepad([0.0; 6], [false; 15]).left_trigger, 0.0);
        // A nearly-centered REAL trigger still normalizes continuously.
        let near = map_gamepad([0.0, 0.0, 0.0, 0.0, 0.001, -0.001], [false; 15]);
        assert!((near.left_trigger - 0.5005).abs() < 1e-4);
        assert!((near.right_trigger - 0.4995).abs() < 1e-4);
        // Out-of-range hardware values clamp instead of leaking.
        let wild = map_gamepad([2.0, 2.0, 0.0, 0.0, 3.0, -3.0], [false; 15]);
        assert_eq!(wild.left_stick, [1.0, -1.0]);
        assert_eq!(wild.left_trigger, 1.0);
        assert_eq!(wild.right_trigger, 0.0);
    }

    #[test]
    fn buttons_map_positionally_and_guide_is_dropped() {
        let mut buttons = [false; 15];
        buttons[0] = true; // A → south
        buttons[6] = true; // Back → select
        buttons[8] = true; // Guide → (dropped)
        buttons[14] = true; // DpadLeft
        let pad = map_gamepad([0.0; 6], buttons);
        assert!(pad.south && pad.select && pad.dpad_left);
        assert!(!pad.east && !pad.west && !pad.north);
        assert!(!pad.start && !pad.left_bumper && !pad.right_bumper);
        assert!(!pad.left_stick_pressed && !pad.right_stick_pressed);
        assert!(!pad.dpad_up && !pad.dpad_right && !pad.dpad_down);
    }
}
