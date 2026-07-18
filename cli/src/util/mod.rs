pub mod asset_verify;
mod shell_command;
// The wasm dev server `include_bytes!`s the web bundle, so it (and its bundle
// dependency) only exist under the `web` feature. The static `build wasm`
// exporter writes those same embedded files, so it's gated too.
#[cfg(feature = "web")]
mod wasm_dev_server;
#[cfg(feature = "web")]
mod wasm_export;

pub use shell_command::*;
#[cfg(feature = "web")]
pub use wasm_dev_server::*;
#[cfg(feature = "web")]
pub use wasm_export::*;
