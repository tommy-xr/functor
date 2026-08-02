//! Poll the first connected standard-mapping gamepad into the shared
//! [`GamepadSnapshot`] domain.
//!
//! The pure conversion — Y negation, trigger normalization, the
//! unmapped-trigger rest rule — lives beside the domain as
//! [`GamepadSnapshot::from_glfw_mapping`] (unit-tested there, next to its web
//! twin); this module only gathers `glfwGetGamepadState`'s raw arrays.
//!
//! "Primary pad" is re-resolved every scan as the lowest-id mapped joystick,
//! so if two pads are connected and the first disconnects, the game continues
//! on the second with no discontinuity signal — a deliberate consequence of
//! the single-pad contract, not an accident.

use functor_runtime_common::GamepadSnapshot;

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
        Some(GamepadSnapshot::from_glfw_mapping(axes, buttons))
    })
}
