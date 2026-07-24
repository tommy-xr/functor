//! Desktop policy wrapper for the shared native debug HTTP transport.
//!
//! The transport and wire contract live in `functor-runtime-common`; this
//! shell retains the desktop CLI's bind logging and fatal bind-error policy.

use std::sync::mpsc::Receiver;

pub use functor_runtime_common::debug_protocol::{
    CaptureError, DebugRequest, InputCommand, RuntimeState, RuntimeView, RuntimeViewport,
    TimeCommand,
};
/// Start the debug server and return the frame loop's request receiver.
///
/// `127.0.0.1` keeps it local; `0.0.0.0` exposes it for remote development.
/// There is no authentication, so wide binds are appropriate only on trusted
/// networks where arbitrary game-code pushes are acceptable.
pub fn spawn(bind: &str, port: u16) -> Receiver<DebugRequest> {
    let address = format!("{bind}:{port}");
    let receiver = match functor_runtime_common::debug_http::spawn((bind, port)) {
        Ok(receiver) => receiver,
        Err(error) => {
            eprintln!("[debug-server] failed to bind {address}: {error}");
            std::process::exit(1);
        }
    };

    // Stderr, not stdout: `--debug-port` is a common `--json` automation combo,
    // so this notice must not land in the CLI's ndjson stream.
    eprintln!("[debug-server] listening on http://{address}");
    receiver
}
