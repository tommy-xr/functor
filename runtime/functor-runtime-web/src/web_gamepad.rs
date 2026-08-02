//! Poll the browser Gamepad API into the shared [`GamepadSnapshot`] domain —
//! the web twin of the desktop shell's `desktop_gamepad`.
//!
//! Only pads reporting the `"standard"` mapping are considered (anything else
//! has no reliable axis/button identity), and the first connected one wins —
//! the same lowest-id primary-pad contract as desktop, with the same caveat
//! that identity silently re-resolves if that pad disconnects. The pure
//! conversion ([`GamepadSnapshot::from_standard_mapping`]) lives in
//! `functor_runtime_common` so it unit-tests natively; this module only
//! gathers the raw arrays across the js boundary.
//!
//! Browsers expose nothing through `getGamepads()` until the user presses a
//! button on the pad (a fingerprinting defense) — until then this returns
//! `None`, which is exactly the domain's no-pad capability signal.

use functor_runtime_common::GamepadSnapshot;
use wasm_bindgen::JsCast;

/// Sample the first connected standard-mapping pad, or `None`.
pub fn sample() -> Option<GamepadSnapshot> {
    let navigator = web_sys::window()?.navigator();
    let pads = navigator.get_gamepads().ok()?;
    pads.iter()
        // Unchecked rather than `dyn_into`: web-sys accessors are structural
        // (plain property reads), and an `instanceof Gamepad` check would
        // reject the duck-typed pads test harnesses install over
        // `navigator.getGamepads`. Null slots (disconnected indices) are
        // filtered first.
        .filter(|entry| !entry.is_null() && !entry.is_undefined())
        .map(|entry| entry.unchecked_into::<web_sys::Gamepad>())
        .find(|pad| {
            pad.connected() && pad.mapping() == web_sys::GamepadMappingType::Standard
        })
        .map(|pad| {
            let axes = pad.axes();
            let axis = |i: u32| axes.get(i).as_f64().unwrap_or(0.0) as f32;
            let buttons = pad.buttons();
            let button = |i: u32| {
                let entry = buttons.get(i);
                (!entry.is_null() && !entry.is_undefined())
                    .then(|| entry.unchecked_into::<web_sys::GamepadButton>())
            };
            let pressed = |i: u32| button(i).is_some_and(|b| b.pressed());
            let value = |i: u32| button(i).map_or(0.0, |b| b.value() as f32);
            let mut held = [false; 16];
            for (i, slot) in held.iter_mut().enumerate() {
                *slot = pressed(i as u32);
            }
            GamepadSnapshot::from_standard_mapping(
                [axis(0), axis(1), axis(2), axis(3)],
                [value(6), value(7)],
                held,
            )
        })
}
