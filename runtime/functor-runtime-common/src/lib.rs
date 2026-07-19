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

pub mod anim;
pub mod animation;
pub mod asset;
pub mod audio;
mod camera;
pub mod composite;
pub mod events;
pub mod fog;
pub mod gpu_counters;
mod frame;
mod frame_time;
pub mod game_clock;
pub mod geometry;
mod input;
pub mod inspect;
pub mod inspector;
pub mod io;
mod light;
pub mod manifest;
pub mod material;
pub mod math;
pub mod functor_lang_game_embedded;
pub mod functor_lang_prelude;
pub mod host_registry;
pub mod functor_lang_producer;
pub mod model;
pub mod net;
pub mod physics;
pub mod protocol;
pub mod render;
mod render_context;
pub mod render_target;
mod renderer;
mod scene3d;
mod shader;
pub mod shadow;
pub mod skybox;
mod shader_program;
pub mod texture;
pub mod timetravel;
pub mod trajectory;
pub mod ui;
pub mod webview;
mod viewport;

pub use camera::*;
pub use fog::Fog;
pub use frame::*;
pub use frame_time::*;
pub use game_clock::GameClock;
pub use input::*;
pub use light::*;
pub use render_context::*;
pub use render_target::RenderTargetDescriptor;
pub use renderer::*;
pub use skybox::SkyboxDescription;
pub use scene3d::*;
pub use trajectory::{
    interactive_preview, overlay, scene_preview, trajectory_trail, InteractivePreview,
    PreviewMode, PreviewOptions, ScenePreview, StrobeOptions,
};
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
