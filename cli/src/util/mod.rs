mod shell_command;
// The wasm dev server `include_bytes!`s the web bundle, so it (and its bundle
// dependency) only exist under the `web` feature.
#[cfg(feature = "web")]
mod wasm_dev_server;

pub use shell_command::*;
#[cfg(feature = "web")]
pub use wasm_dev_server::*;
