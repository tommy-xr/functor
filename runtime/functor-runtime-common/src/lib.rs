use serde::*;
use std::any::Any;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Scene3D {
    Cube,
    Sphere,
    Cylinder,
}

impl Scene3D {
    pub fn cube() -> Self {
        Self::Cube
    }

    pub fn sphere() -> Self {
        Self::Sphere
    }

    pub fn cylinder() -> Self {
        Self::Cylinder
    }
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

pub mod geometry;
pub mod material;
mod render_context;
mod shader;
mod shader_program;
pub mod texture;

pub use render_context::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
