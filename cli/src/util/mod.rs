// The staging rules shared by both bundle exporters, and the native
// exporter itself — ungated: `build native` works without the web bundle.
pub mod bundle;
mod native_export;
mod shell_command;
// The wasm dev server `include_bytes!`s the web bundle, so it (and its
// exporter, which writes those same embedded files) only exist under the
// `web` feature.
#[cfg(feature = "web")]
mod wasm_dev_server;
#[cfg(feature = "web")]
mod wasm_export;

pub use native_export::*;
pub use shell_command::*;
#[cfg(feature = "web")]
pub use wasm_dev_server::*;
#[cfg(feature = "web")]
pub use wasm_export::*;
