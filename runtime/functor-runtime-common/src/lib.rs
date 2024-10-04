#![cfg_attr(feature = "strict", deny(warnings))]

use std::any::Any;

#[cfg(target_arch = "wasm32")]
use serde::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

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
mod effect;
mod effect_queue;
mod frame_time;
pub mod geometry;
pub mod io;
pub mod material;
pub mod math;
pub mod model;
pub mod render;
mod render_context;
mod scene3d;
mod shader;
mod shader_program;
pub mod texture;

pub use effect::*;
pub use effect_queue::*;
pub use frame_time::*;
pub use render_context::*;
pub use scene3d::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
