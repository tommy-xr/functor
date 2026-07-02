#![cfg_attr(feature = "strict", deny(warnings))]

use std::any::Any;

use fable_library_rust::String_::LrcStr;

#[cfg(target_arch = "wasm32")]
use serde::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// Log a line so it is visible on every target. On wasm, Rust's `println!`
// writes to stdout, which isn't connected to anything in the browser
// (wasm32-unknown-unknown has no WASI), so F# `printfn` output is silently
// dropped; route it to `console.log` instead. On native, `println!` is fine.
#[cfg(target_arch = "wasm32")]
pub fn log(message: LrcStr) {
    web_sys::console::log_1(&JsValue::from_str(&message));
}

#[cfg(not(target_arch = "wasm32"))]
pub fn log(message: LrcStr) {
    println!("{}", message);
}

#[cfg(target_arch = "wasm32")]
pub fn to_js_value<T>(value: &T) -> JsValue
where
    T: Serialize,
{
    serde_wasm_bindgen::to_value(value).unwrap()
}

#[cfg(target_arch = "wasm32")]
pub fn from_js_value<T>(value: JsValue) -> T
where
    T: for<'de> Deserialize<'de>,
{
    serde_wasm_bindgen::from_value(value).unwrap()
}

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

pub struct OpaqueState {
    any: Box<dyn Any>,
}

impl OpaqueState {
    pub fn new<T: 'static>(obj: T) -> OpaqueState {
        OpaqueState { any: Box::new(obj) }
    }

    pub fn coerce<T: 'static + Clone + Sized>(opaque_state: OpaqueState) -> T {
        unsafe {
            // Convert Box<dyn Any> to raw pointer
            let raw = Box::into_raw(opaque_state.any);
            // Convert the raw pointer to Box<T>
            let boxed_t: Box<T> = Box::from_raw(raw as *mut T);
            // Unbox to get the inner T
            *boxed_t
        }
    }
}

pub mod animation;
pub mod asset;
pub mod audio;
mod camera;
mod effect;
mod effect_queue;
mod frame;
mod frame_time;
pub mod geometry;
mod input;
pub mod inspect;
pub mod io;
mod light;
pub mod material;
pub mod math;
pub mod model;
pub mod net;
pub mod protocol;
pub mod render;
mod render_context;
mod renderer;
mod scene3d;
mod shader;
pub mod shadow;
mod shader_program;
pub mod texture;
pub mod ui;
mod viewport;

pub use camera::*;
pub use effect::*;
pub use effect_queue::*;
pub use frame::*;
pub use frame_time::*;
pub use input::*;
pub use light::*;
pub use render_context::*;
pub use renderer::*;
pub use scene3d::*;
pub use viewport::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
